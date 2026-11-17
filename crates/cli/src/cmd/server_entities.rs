//! `qcheat server-entities` — read the *server's* entity array
//! (`g_entities[]`) from a listen server and compare it with what the
//! client actually received.
//!
//! This is the server-side counterpart of `snapshot`. The client only
//! gets the entities the server decided to send (PVS); the server
//! knows all of them. Because a listen server runs both halves in one
//! process, the authoritative array can be read with the same tools.
//!
//! How the array is found (no hard-coded address):
//! 1. Take the players from the client snapshot as *ground truth*.
//! 2. Scan memory for 516-byte blocks that look like the server twin
//!    of one of them (same `number`/`clientNum`, close position,
//!    self-consistent `r` block).
//! 3. Entity `n` lives at `base + n * sizeof(gentity_t)`, so two twins
//!    with different numbers reveal the stride; the base follows.
//! 4. Cross-check against the engine: `sv.gentities` is a pointer to
//!    that array stored in the exe's `.bss`; finding it yields a
//!    stable RVA usable with `--sv-rva` on later runs.
//!
//! Safety rule: the command refuses to display anything when more
//! than one non-bot player is present (see [`sdk::bots_only_verdict`]).
//! It is meant for a local game against bots, and enforces it.

use anyhow::{bail, Context, Result};
use clap::Args as ClapArgs;
use sdk::{
    bots_only_verdict, EntityState, EntityType, LocalOnlyVerdict, SharedEntity, Snapshot,
    SnapshotFilter, MAX_CLIENTS, MAX_ENTITIES_IN_SNAPSHOT, MAX_GENTITIES,
};
use std::collections::HashSet;

use crate::locate::{
    self, locate_snapshot_addrs, newest_snapshot_addr, DEFAULT_SNAPSHOT_CENTER,
    DEFAULT_SNAPSHOT_RANGE,
};
use crate::util::{parse_hex, DEFAULT_PROCESS};

/// Arguments accepted by `qcheat server-entities`.
#[derive(ClapArgs)]
pub struct Args {
    /// Name of the target executable.
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    /// Centre of the window scanned for the server entity array (hex).
    #[arg(long, value_parser = parse_hex)]
    pub center: Option<usize>,

    /// Half-range of that window (hex).
    #[arg(long, value_parser = parse_hex)]
    pub range: Option<usize>,

    /// Centre of the window scanned for the client snapshot (hex).
    #[arg(long, value_parser = parse_hex)]
    pub snap_center: Option<usize>,

    /// Half-range of the snapshot window (hex).
    #[arg(long, value_parser = parse_hex)]
    pub snap_range: Option<usize>,

    /// Max distance (world units) between the client's view of a
    /// player and its server twin. The client lags by a frame or two.
    #[arg(long, default_value = "150.0")]
    pub tolerance: f32,

    /// Force `sizeof(gentity_t)` in bytes (hex). Needed only when a
    /// single player is visible, so the stride cannot be inferred.
    #[arg(long, value_parser = parse_hex)]
    pub stride: Option<usize>,

    /// RVA (hex, relative to the exe base) of `sv.gentities`, as
    /// printed by a previous run. Skips the memory scan entirely.
    #[arg(long, value_parser = parse_hex)]
    pub sv_rva: Option<usize>,
}

/// Locate `g_entities[]`, validate it, and print the server's players
/// next to what the client received.
pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;
    println!("Attached to {} (pid {}).\n", proc.name, proc.pid);

    // --- Step 1: client snapshot = ground truth + PVS reference. ---
    let snap_start = args
        .snap_center
        .unwrap_or(DEFAULT_SNAPSHOT_CENTER)
        .saturating_sub(args.snap_range.unwrap_or(DEFAULT_SNAPSHOT_RANGE));
    let snap_end = args
        .snap_center
        .unwrap_or(DEFAULT_SNAPSHOT_CENTER)
        .saturating_add(args.snap_range.unwrap_or(DEFAULT_SNAPSHOT_RANGE));
    let snap_addrs = locate_snapshot_addrs(&handle, snap_start, snap_end, SnapshotFilter::Strict);
    let snapshot: Option<Box<Snapshot>> = newest_snapshot_addr(&handle, &snap_addrs)
        .and_then(|a| handle.read::<Snapshot>(a).ok())
        .map(Box::new);
    let client_players: Vec<EntityState> = snapshot
        .as_ref()
        .map(|s| {
            let n = s.header.num_entities.clamp(0, MAX_ENTITIES_IN_SNAPSHOT as i32) as usize;
            s.entities[..n]
                .iter()
                .filter(|e| e.e_type == EntityType::PLAYER)
                .copied()
                .collect()
        })
        .unwrap_or_default();
    match &snapshot {
        Some(s) => println!(
            "[1] Client snapshot: serverTime {}, {} entities, {} player(s) received.",
            s.header.server_time,
            s.header.num_entities,
            client_players.len()
        ),
        None => println!("[1] Client snapshot: not found (the PVS comparison will be skipped)."),
    }

    // --- Step 2: base + stride of g_entities[]. ---
    let (base, stride, via_engine) = match args.sv_rva {
        Some(rva) => {
            let at = proc.base_address + rva;
            let ptr = handle.read::<u64>(at).with_context(|| format!("reading sv.gentities at 0x{at:X}"))?;
            let size = handle.read::<i32>(at + 8).context("reading sv.gentitySize")?;
            if ptr == 0 || size < sdk::server::GENTITY_MIN_STRIDE as i32 {
                bail!("sv.gentities at 0x{at:X} looks empty (ptr=0x{ptr:X}, size={size}); is a game running?");
            }
            println!("[2] Engine globals at exe+0x{rva:X}: g_entities=0x{ptr:X}, sizeof(gentity_t)={size}.");
            (ptr as usize, size as usize, true)
        }
        None => {
            let opts = locate::GentitiesLookup {
                center: args.center.unwrap_or(locate::DEFAULT_SERVER_CENTER),
                range: args.range.unwrap_or(locate::DEFAULT_SERVER_RANGE),
                tolerance: args.tolerance,
                stride: args.stride,
            };
            println!("[2] Locating g_entities[]...");
            let (b, s) = locate::locate_gentities(&handle, &client_players, &opts)?;
            (b, s, false)
        }
    };

    // --- Step 3: read and validate the whole array. ---
    let mut entities = vec![bytemuck::Zeroable::zeroed(); MAX_GENTITIES];
    let mut readable = 0usize;
    for (i, slot) in entities.iter_mut().enumerate() {
        if let Ok(e) = handle.read::<SharedEntity>(base + i * stride) {
            *slot = e;
            readable += 1;
        }
    }
    let used: Vec<usize> = (0..MAX_GENTITIES)
        .filter(|&i| entities[i].s.e_type != 0 || entities[i].r.linked != 0)
        .collect();
    let consistent = used.iter().filter(|&&i| entities[i].s.number as usize == i).count();
    println!(
        "[3] g_entities @ 0x{base:016X}, stride {stride} (0x{stride:X}): {readable}/{MAX_GENTITIES} slots readable, \
         {consistent}/{} in-use slots have number == index.",
        used.len()
    );
    if used.is_empty() || consistent * 10 < used.len() * 9 {
        bail!("this does not look like g_entities[] (index/number mismatch). Try --stride or a different --center/--range.");
    }

    // --- Safety rule: local game vs bots only. ---
    match bots_only_verdict(&entities) {
        LocalOnlyVerdict::Ok { players, bots } => {
            println!("[4] Local-game check: {players} player(s), {bots} bot(s), at most one human. OK.\n");
        }
        LocalOnlyVerdict::OtherHumans(slots) => {
            bail!(
                "refusing to continue: slots {slots:?} are players not flagged as bots. \
                 This tool only runs on a local game against bots."
            );
        }
    }

    // --- Step 5: what the server knows vs what the client got. ---
    let visible: HashSet<i32> = snapshot
        .as_ref()
        .map(|s| {
            let n = s.header.num_entities.clamp(0, MAX_ENTITIES_IN_SNAPSHOT as i32) as usize;
            s.entities[..n].iter().map(|e| e.number).collect()
        })
        .unwrap_or_default();

    println!("slot  kind   state   origin (server)                  in client snapshot?");
    let mut server_players = 0usize;
    let mut hidden = 0usize;
    for (i, e) in entities.iter().enumerate().take(MAX_CLIENTS) {
        if !e.is_player() || e.r.linked == 0 || e.s.number as usize != i {
            continue;
        }
        server_players += 1;
        let seen = snapshot.is_none() || visible.contains(&e.s.number);
        if !seen {
            hidden += 1;
        }
        let o = e.r.current_origin;
        println!(
            "{i:>4}  {:<5}  {:<6}  ({:>8.1}, {:>8.1}, {:>8.1})  {}",
            if e.is_bot() { "bot" } else { "host" },
            if e.is_dead() { "dead" } else { "alive" },
            o.x,
            o.y,
            o.z,
            match (&snapshot, seen) {
                (None, _) => "n/a",
                (_, true) => "yes",
                (_, false) => "NO  <- filtered by the server (PVS)",
            }
        );
    }
    println!(
        "\n{server_players} player(s) on the server, {hidden} not sent to this client (PVS)."
    );
    let unused_zero = used
        .iter()
        .filter(|&&i| entities[i].r.unused.number == 0 && entities[i].r.unused.e_type == 0)
        .count();
    println!(
        "Study note: r.unused is zero in {unused_zero}/{} in-use slots (vestigial entityState_t).",
        used.len()
    );

    if !via_engine {
        println!("\nTip: next time, skip the scan with --sv-rva (printed in step 2 if found).");
    }
    Ok(())
}
