//! `qcheat snapshot` — locate cg.activeSnapshots and dump the active
//! frame (local player + every visible entity).

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use sdk::{
    EntityState, EntityType, PlayerState, Snapshot, SnapshotHeader,
    MAX_ENTITIES_IN_SNAPSHOT, STAT_ARMOR, STAT_HEALTH,
};

use crate::locate::find_snapshot_candidates;
use sdk::SnapshotFilter;
use crate::util::{parse_hex, DEFAULT_PROCESS};


/// Default heap scan window. The 32 MiB band around `0x07000000`
/// covers the addresses where the cgame QVM allocates `cg` in our
/// observed builds. Override with `--center` / `--range` if needed.
const DEFAULT_CENTER: usize = 0x07000000;
const DEFAULT_RANGE: usize = 0x02000000;

/// Arguments accepted by `qcheat snapshot`.
#[derive(ClapArgs)]
pub struct Args {
    /// Name of the target executable.
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    /// Override the scan window centre (hex).
    #[arg(long, value_parser = parse_hex)]
    pub center: Option<usize>,

    /// Override the scan window half-range (hex).
    #[arg(long, value_parser = parse_hex)]
    pub range: Option<usize>,
}

/// Locate `cg.activeSnapshots`, pick the active one by `serverTime`,
/// then dump local player + visible entities.
pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;

    let center = args.center.unwrap_or(DEFAULT_CENTER);
    let range = args.range.unwrap_or(DEFAULT_RANGE);
    let start = center.saturating_sub(range);
    let end = center.saturating_add(range);
    println!(
        "Attached to {} (pid {}). Scanning 0x{:016X}..0x{:016X} ({} KiB) for cg.snap...\n",
        proc.name,
        proc.pid,
        start,
        end,
        (end - start) / 1024
    );

    let hits = find_snapshot_candidates(&handle, start, end, SnapshotFilter::Strict);
    if hits.is_empty() {
        println!("No snapshot_t candidates found.");
        println!("Tip: widen the range or shift the center via --center / --range.");
        return Ok(());
    }

    println!("Found {} snapshot candidate(s).", hits.len());

    // cg.activeSnapshots[2] sit exactly sizeof(snapshot_t) bytes apart.
    // That's our gold signal — if a pair matches that gap, we know we
    // found the real engine buffer.
    const SS_SIZE: usize = core::mem::size_of::<Snapshot>();
    let pair = hits
        .windows(2)
        .find(|w| w[1] - w[0] == SS_SIZE)
        .map(|w| (w[0], w[1]));

    let addr = match pair {
        Some((a, b)) => {
            println!(
                "\n=> Confirmed cg.activeSnapshots pair: 0x{a:016X} / 0x{b:016X} \
                 (Δ = {SS_SIZE} bytes)."
            );
            // Pick whichever has the larger serverTime — that's cg.snap
            // (latest received), the other is cg.nextSnap.
            let ta = handle
                .read::<SnapshotHeader>(a)
                .map(|h| h.server_time)
                .unwrap_or(i32::MIN);
            let tb = handle
                .read::<SnapshotHeader>(b)
                .map(|h| h.server_time)
                .unwrap_or(i32::MIN);
            let chosen = if ta >= tb { a } else { b };
            println!(
                "   serverTime: [{a:016X}]={ta}  [{b:016X}]={tb}  → reading the newer one"
            );
            chosen
        }
        None => {
            println!("\n=> No 53772-byte-spaced pair detected.");
            println!(
                "   Falling back to the first candidate; results may be stale or bogus."
            );
            for (i, h) in hits.iter().enumerate().take(20) {
                println!("   [{i}] 0x{h:016X}");
            }
            hits[0]
        }
    };

    println!("\nReading full snapshot at 0x{addr:016X}...\n");
    let snap = Box::new(
        handle
            .read::<Snapshot>(addr)
            .with_context(|| format!("reading Snapshot at 0x{addr:016X}"))?,
    );

    print_local_player(&snap.header.ps);
    print_entities(&snap.entities, snap.header.num_entities);
    Ok(())
}

/// Print the local player block.
fn print_local_player(ps: &PlayerState) {
    println!("LOCAL PLAYER (cg.snap.ps)");
    println!("  client_num  : {}", ps.client_num);
    println!("  pm_type     : {}", ps.pm_type);
    println!(
        "  origin      : ({:>8.1}, {:>8.1}, {:>8.1})",
        ps.origin.x, ps.origin.y, ps.origin.z
    );
    println!(
        "  velocity    : ({:>8.1}, {:>8.1}, {:>8.1})  (|v|={:.1})",
        ps.velocity.x,
        ps.velocity.y,
        ps.velocity.z,
        ps.velocity.length()
    );
    println!(
        "  viewangles  : ({:>8.2}, {:>8.2}, {:>8.2})",
        ps.viewangles.x, ps.viewangles.y, ps.viewangles.z
    );
    println!("  weapon      : {}", ps.weapon);
    println!("  HP          : {}", ps.stats[STAT_HEALTH]);
    println!("  Armor       : {}", ps.stats[STAT_ARMOR]);
    println!();
}

/// Iterate the entity array and print every PLAYER.
fn print_entities(entities: &[EntityState; MAX_ENTITIES_IN_SNAPSHOT], num: i32) {
    let n = num.clamp(0, MAX_ENTITIES_IN_SNAPSHOT as i32) as usize;
    println!("SNAPSHOT ENTITIES (num_entities = {n})");

    let mut by_type = [0usize; 13];
    for es in &entities[..n] {
        if (0..13).contains(&es.e_type) {
            by_type[es.e_type as usize] += 1;
        }
    }
    println!(
        "  breakdown: PLAYER={} ITEM={} MISSILE={} MOVER={} SPEAKER={} other={}",
        by_type[EntityType::PLAYER as usize],
        by_type[EntityType::ITEM as usize],
        by_type[EntityType::MISSILE as usize],
        by_type[EntityType::MOVER as usize],
        by_type[EntityType::SPEAKER as usize],
        n - by_type.iter().sum::<usize>(),
    );

    if by_type[EntityType::PLAYER as usize] == 0 {
        println!("  (no visible players this frame)");
        return;
    }

    println!();
    println!(
        "  {:<5} {:<7} {:<7} {:<7} {}",
        "slot", "client", "weapon", "anim", "position (tr_base)"
    );
    println!("  {:-<5} {:-<7} {:-<7} {:-<7} {:-<36}", "", "", "", "", "");
    for es in &entities[..n] {
        if es.e_type != EntityType::PLAYER {
            continue;
        }
        let p = es.pos.tr_base;
        println!(
            "  {:<5} {:<7} {:<7} {:<7} ({:>8.1}, {:>8.1}, {:>8.1})",
            es.number, es.client_num, es.weapon, es.legs_anim, p.x, p.y, p.z
        );
    }
}
