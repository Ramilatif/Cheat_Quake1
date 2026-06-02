//! Locate the active cgame `snapshot_t` in the target's heap and dump
//! the local player plus every visible entity for the current frame.
//!
//! Why not just brute-scan for player-shaped bytes like `dump_players`?
//! Because the engine keeps several stale copies of every entity around
//! (ring buffers, baselines, double-buffered snapshots), and a brute
//! scan can't tell which copy is the live one. The snapshot is what the
//! engine itself renders from: read it, and you see exactly what the
//! HUD sees, this frame, no aliasing.
//!
//! Strategy:
//! 1. Walk the QVM heap window at 4-byte alignment.
//! 2. At each offset reinterpret 516 bytes as a [`SnapshotHeader`] and
//!    sanity-check: `ps.client_num` in `0..MAX_CLIENTS`, weapon sane,
//!    `num_entities` in `0..=MAX_ENTITIES_IN_SNAPSHOT`, origin finite.
//! 3. On a hit, read the next 53 KiB and iterate `entities[0..num_entities]`.
//!
//! We expect to find *two* snapshots (cg.activeSnapshots[2]) sitting
//! ~54 KiB apart — that's a strong signal we found the right structure.
//!
//! Usage:
//! ```text
//! cargo run -p memory --bin dump-snapshot
//! cargo run -p memory --bin dump-snapshot -- ioquake3.x86_64.exe
//! cargo run -p memory --bin dump-snapshot -- ioquake3.x86_64.exe 0x06000000 0x02000000
//! ```

use std::process::ExitCode;

use bytemuck::{from_bytes, Pod, Zeroable};
use sdk::{
    EntityState, EntityType, PlayerState, Snapshot, SnapshotHeader,
    MAX_CLIENTS, MAX_ENTITIES_IN_SNAPSHOT, STAT_ARMOR, STAT_HEALTH,
};

/// Bytes per chunked read while scanning.
const CHUNK: usize = 4096;
/// Size of the snapshot fingerprint we evaluate at each offset.
const HEADER_SIZE: usize = core::mem::size_of::<SnapshotHeader>();

/// Default heap scan window (centre, half-range). The 32 MiB band around
/// `0x07000000` covers the addresses where the cgame QVM allocates `cg`
/// in our observed builds. Override on the command line if needed.
const DEFAULT_CENTER: usize = 0x07000000;
const DEFAULT_RANGE: usize = 0x02000000;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let process_name = args
        .next()
        .unwrap_or_else(|| "ioquake3.x86_64.exe".to_string());
    let center = args
        .next()
        .as_deref()
        .map(parse_hex)
        .unwrap_or(Some(DEFAULT_CENTER))
        .unwrap_or(DEFAULT_CENTER);
    let range = args
        .next()
        .as_deref()
        .map(parse_hex)
        .unwrap_or(Some(DEFAULT_RANGE))
        .unwrap_or(DEFAULT_RANGE);

    let proc = match memory::find_by_name(&process_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let handle = match memory::ProcessHandle::open(proc.pid) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

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

    let hits = find_snapshot_candidates(&handle, start, end);
    if hits.is_empty() {
        println!("No snapshot_t candidates found.");
        println!("Tip: widen the range (3rd arg) or shift the center (2nd arg).");
        println!("     The buffer 0x05F88040 found by scan-entities is *inside* a snapshot.");
        return ExitCode::SUCCESS;
    }

    println!("Found {} snapshot candidate(s).", hits.len());

    // cg.activeSnapshots[2] sit exactly sizeof(snapshot_t) bytes apart.
    // That's our gold signal — if a pair matches that gap, we know we
    // found the real engine buffer and not some lookalike (parseEntities
    // entry, snapshots[PACKET_BACKUP] ring, …).
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
            println!("   Falling back to the first candidate; results may be a stale or bogus snapshot.");
            for (i, h) in hits.iter().enumerate().take(20) {
                println!("   [{i}] 0x{h:016X}");
            }
            hits[0]
        }
    };
    println!("\nReading full snapshot at 0x{addr:016X}...\n");
    let snap = match handle.read::<Snapshot>(addr) {
        Ok(s) => Box::new(s),
        Err(e) => {
            eprintln!("read failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    print_local_player(&snap.header.ps);
    print_entities(&snap.entities, snap.header.num_entities);

    ExitCode::SUCCESS
}

/// Walk the requested window and report addresses that look like a real
/// `snapshot_t`.
fn find_snapshot_candidates(
    handle: &memory::ProcessHandle,
    start: usize,
    end: usize,
) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let buf = match handle.read::<Chunk>(cursor) {
            Ok(b) => b,
            Err(_) => {
                cursor = cursor.saturating_add(CHUNK);
                continue;
            }
        };
        let mut off = 0usize;
        while off + HEADER_SIZE <= buf.0.len() {
            let header: &SnapshotHeader = from_bytes(&buf.0[off..off + HEADER_SIZE]);
            if looks_like_snapshot(header) {
                hits.push(cursor + off);
                // Skip past this snapshot — there can't be another one
                // overlapping it, and we don't want 50 dupes from shifted reads.
                off += HEADER_SIZE;
            } else {
                off += 4;
            }
        }
        cursor = cursor.saturating_add(CHUNK);
    }
    hits
}

/// Sanity-filter a candidate `SnapshotHeader`. Strict enough that an
/// all-zero block doesn't pass, loose enough that any real in-game
/// frame does. Empty/uninitialised memory full of zeros would otherwise
/// satisfy `num_entities == 0`, `client_num == 0`, etc. and produce a
/// useless "hit".
fn looks_like_snapshot(h: &SnapshotHeader) -> bool {
    if !(0..=MAX_ENTITIES_IN_SNAPSHOT as i32).contains(&h.num_entities) {
        return false;
    }
    // Real snapshots have a serverTime in the tens of thousands at minimum
    // (engine pumps ms steadily from connect). Tiny positive values are
    // almost always noise that happens to look int-ish.
    if h.server_time < 1_000 {
        return false;
    }
    if !(0..=2_000).contains(&h.ping) {
        return false;
    }
    let ps = &h.ps;
    if !(0..MAX_CLIENTS as i32).contains(&ps.client_num) {
        return false;
    }
    if !(0..=15).contains(&ps.weapon) {
        return false;
    }
    if !(0..=8).contains(&ps.pm_type) {
        return false;
    }
    let o = ps.origin;
    if !o.x.is_finite() || !o.y.is_finite() || !o.z.is_finite() {
        return false;
    }
    if o.x.abs() >= 32_768.0 || o.y.abs() >= 32_768.0 || o.z.abs() >= 32_768.0 {
        return false;
    }
    let hp = ps.stats[STAT_HEALTH];
    if !(-50..=1_000).contains(&hp) {
        return false;
    }

    // A real cg.snap is never fundamentally empty. At least one of
    // origin / velocity / viewangles must carry a non-zero float, or
    // HP must be positive. An all-zero playerState_t inside the engine
    // means the snapshot hasn't been populated yet and we shouldn't
    // claim it as "found".
    let origin_zero = o.x == 0.0 && o.y == 0.0 && o.z == 0.0;
    let velocity_zero =
        ps.velocity.x == 0.0 && ps.velocity.y == 0.0 && ps.velocity.z == 0.0;
    let angles_zero =
        ps.viewangles.x == 0.0 && ps.viewangles.y == 0.0 && ps.viewangles.z == 0.0;
    if origin_zero && velocity_zero && angles_zero && hp <= 0 {
        return false;
    }
    // An alive PM_NORMAL player must have HP > 0 and a real position.
    const PM_NORMAL: i32 = 0;
    if ps.pm_type == PM_NORMAL && (hp <= 0 || origin_zero) {
        return false;
    }
    // commandTime is the server tick that produced this snapshot.
    // Always positive once we've executed at least one command.
    if ps.command_time <= 0 {
        return false;
    }
    true
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

    let mut players = 0usize;
    let mut by_type = [0usize; 13];
    for es in &entities[..n] {
        if (0..13).contains(&es.e_type) {
            by_type[es.e_type as usize] += 1;
        }
        if es.e_type == EntityType::PLAYER {
            players += 1;
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

    if players == 0 {
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
        // PLAYER entities use TR_INTERPOLATE: the engine writes the
        // canonical position into pos.tr_base each snapshot and
        // interpolates between two consecutive snapshots client-side.
        // entityState_t.origin is left at zero for these.
        let p = es.pos.tr_base;
        println!(
            "  {:<5} {:<7} {:<7} {:<7} ({:>8.1}, {:>8.1}, {:>8.1})",
            es.number, es.client_num, es.weapon, es.legs_anim, p.x, p.y, p.z
        );
    }
}

/// Fixed-size byte buffer for chunked reads.
#[repr(C)]
#[derive(Copy, Clone)]
struct Chunk([u8; CHUNK + HEADER_SIZE]);

// SAFETY: a byte array is trivially Pod.
unsafe impl Zeroable for Chunk {}
unsafe impl Pod for Chunk {}

fn parse_hex(s: &str) -> Option<usize> {
    let stripped = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    usize::from_str_radix(stripped, 16).ok()
}
