//! Find every live player by scanning the whole QVM heap.
//!
//! First "useful" feature of the cheat: a live list of every player
//! (including bots) in the game, with their position, weapon and
//! client slot. Foundation for ESP and aimbot later.
//!
//! How it works:
//! - Walk a fixed heap window (±24 MiB around `0x06800000`) at 4-byte
//!   alignment.
//! - At each offset, reinterpret 208 bytes as an [`EntityState`].
//! - Keep slots whose `e_type == PLAYER`, with a real 3D origin and a
//!   valid `client_num`.
//! - Dedupe overlaps — one real player produces matches at offsets
//!   X, X+4, …, X+204 because the shifted windows still look near-zero
//!   in most fields.
//!
//! We don't rely on finding a specific engine buffer (parseEntities,
//! cg_entities, …): those live at different addresses in different
//! builds and QVM vs native loads. Brute-forcing the heap for
//! PLAYER-shaped bytes works across all of them.
//!
//! Usage:
//! ```text
//! cargo run -p memory --bin dump-players
//! cargo run -p memory --bin dump-players -- ioquake3.x86_64.exe
//! ```

use std::process::ExitCode;

use bytemuck::{from_bytes, Pod, Zeroable};
use sdk::{EntityState, EntityType, MAX_CLIENTS};

/// Size of each streaming read.
const CHUNK: usize = 4096;
/// Size of the EntityState window re-read at each offset.
const ES_SIZE: usize = core::mem::size_of::<EntityState>();

/// Heap scan window. 48 MiB centred on `0x06800000` covers the region
/// where Quake's QVM heap lives in our observed builds.
const SCAN_CENTER: usize = 0x06800000;
const SCAN_RANGE: usize = 0x01800000;

fn main() -> ExitCode {
    let process_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ioquake3.x86_64.exe".to_string());

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
    println!("Attached to {} (pid {})", proc.name, proc.pid);

    let start = SCAN_CENTER.saturating_sub(SCAN_RANGE);
    let end = SCAN_CENTER.saturating_add(SCAN_RANGE);
    println!(
        "Scanning heap 0x{:016X}..0x{:016X} ({} KiB) for live players...\n",
        start,
        end,
        (end - start) / 1024
    );

    let mut players: Vec<(usize, EntityState)> = Vec::new();
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
        while off + ES_SIZE <= buf.0.len() {
            let es: &EntityState = from_bytes(&buf.0[off..off + ES_SIZE]);
            if is_live_player(es) {
                players.push((cursor + off, *es));
            }
            off += 4;
        }
        cursor = cursor.saturating_add(CHUNK);
    }

    // Dedupe: one real player produces ~50 overlapping hits at +4, +8, …
    players.sort_by_key(|(addr, _)| *addr);
    let mut deduped: Vec<(usize, EntityState)> = Vec::new();
    for (addr, es) in players {
        if let Some((last_addr, _)) = deduped.last() {
            if addr < last_addr + ES_SIZE {
                continue;
            }
        }
        deduped.push((addr, es));
    }

    if deduped.is_empty() {
        println!("No live players found in the scan window.");
        println!("Make sure Quake is in-game and the snapshot has been received.");
        return ExitCode::SUCCESS;
    }

    println!("Found {} player entity copy/copies:\n", deduped.len());
    println!(
        "{:<18} {:<5} {:<7} {:<7} {}",
        "address", "num", "client", "weapon", "origin"
    );
    println!("{:-<18} {:-<5} {:-<7} {:-<7} {:-<36}", "", "", "", "", "");
    for (addr, es) in &deduped {
        println!(
            "0x{:016X} {:<5} {:<7} {:<7} ({:>8.1}, {:>8.1}, {:>8.1})",
            addr,
            es.number,
            es.client_num,
            es.weapon,
            es.origin.x,
            es.origin.y,
            es.origin.z
        );
    }
    println!("\nNote: the engine keeps several copies per player");
    println!("      (currentState, nextState, snapshot buffers). Multiple hits per client_num is normal.");

    ExitCode::SUCCESS
}

/// Keep hits that look like a live player. Strict: every field must
/// be consistent with a real Q3 player entity, so random memory that
/// happens to have `e_type == 1` is rejected.
fn is_live_player(es: &EntityState) -> bool {
    if es.e_type != EntityType::PLAYER {
        return false;
    }
    // For a PLAYER entity, `number` is the client slot (0..MAX_CLIENTS).
    if !(0..MAX_CLIENTS as i32).contains(&es.number) {
        return false;
    }
    if !(0..MAX_CLIENTS as i32).contains(&es.client_num) {
        return false;
    }
    // Vanilla Q3 has 9 weapon slots (0 = none, 1..=8 = MG..BFG).
    if !(0..=9).contains(&es.weapon) {
        return false;
    }
    // All three origin components must be real, in-map and non-trivial.
    // Real players are never at x=0 or y=0 exactly — that's always noise.
    let ox = es.origin.x;
    let oy = es.origin.y;
    let oz = es.origin.z;
    if !ox.is_finite() || !oy.is_finite() || !oz.is_finite() {
        return false;
    }
    if ox.abs() < 1.0 || oy.abs() < 1.0 {
        return false;
    }
    if ox.abs() >= 32_768.0 || oy.abs() >= 32_768.0 || oz.abs() >= 32_768.0 {
        return false;
    }
    // Trajectory base must track the current origin tightly — for a
    // live interpolated player it's within a handful of units.
    (es.pos.tr_base.x - ox).abs() < 256.0
        && (es.pos.tr_base.y - oy).abs() < 256.0
        && (es.pos.tr_base.z - oz).abs() < 256.0
}

/// Fixed-size buffer passed to `ProcessHandle::read`, so the scan does
/// one Win32 round-trip per chunk.
#[repr(C)]
#[derive(Copy, Clone)]
struct Chunk([u8; CHUNK + ES_SIZE]);

unsafe impl Zeroable for Chunk {}
unsafe impl Pod for Chunk {}

