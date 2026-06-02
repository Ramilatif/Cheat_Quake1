//! Brute-force scanner for `entityState_t`-shaped bytes in a running
//! Quake III process.
//!
//! We know *one* address that holds a live player position
//! (`0x0613F728` in our session), but the surrounding bytes are all
//! zero — meaning that address lives in `cg_t` (the per-frame client
//! state) rather than inside the `cg_entities[MAX_GENTITIES]` array.
//! To drive an ESP / aimbot we need to find that array: 1024 adjacent
//! `centity_t` slots, each beginning with an [`EntityState`].
//!
//! Approach:
//! 1. Read a window of memory around a user-supplied center address
//!    (default ±512 KiB).
//! 2. At every 4-byte aligned offset, reinterpret the next 208 bytes as
//!    an [`EntityState`] and sanity-check every field.
//! 3. Print each hit; afterwards, look for runs of hits separated by a
//!    roughly constant stride — that stride is `sizeof(centity_t)` and
//!    the first hit is `&cg_entities[0].currentState`.
//!
//! Usage:
//! ```text
//! cargo run -p memory --bin scan-entities -- 0x0613F728
//! cargo run -p memory --bin scan-entities -- 0x0613F728 0x200000
//! cargo run -p memory --bin scan-entities -- 0x0613F728 0x80000 ioquake3.x86_64.exe
//! ```

use std::process::ExitCode;

use bytemuck::{from_bytes, Pod, Zeroable};
use sdk::{EntityState, MAX_CLIENTS, MAX_GENTITIES};

/// Size of each streaming read. Too big and we cross unmapped pages and
/// the whole read fails; too small and we hammer ReadProcessMemory.
const CHUNK: usize = 4096;

/// Size of the EntityState window we reinterpret at each offset.
const ES_SIZE: usize = core::mem::size_of::<EntityState>();

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let center_str = match args.next() {
        Some(s) => s,
        None => {
            eprintln!(
                "usage: scan-entities <center-hex> [range-hex=0x80000] [process-name]"
            );
            return ExitCode::FAILURE;
        }
    };
    let range = args
        .next()
        .as_deref()
        .map(parse_hex)
        .unwrap_or(Some(0x80000))
        .unwrap_or(0x80000);
    let process_name = args
        .next()
        .unwrap_or_else(|| "ioquake3.x86_64.exe".to_string());

    let center = match parse_hex(&center_str) {
        Some(a) => a,
        None => {
            eprintln!("error: center address must be hex, e.g. 0x0613F728");
            return ExitCode::FAILURE;
        }
    };

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
        "Attached to {} (pid {}). Scanning 0x{:016X}..0x{:016X} ({} KiB).\n",
        proc.name,
        proc.pid,
        start,
        end,
        (end - start) / 1024
    );

    let mut hits: Vec<Hit> = Vec::new();
    let mut cursor = start;
    while cursor < end {
        // Read CHUNK + overlap so an EntityState straddling a chunk
        // boundary is still recognised.
        let want = CHUNK + ES_SIZE;
        let buf = match handle.read::<Chunk>(cursor) {
            Ok(b) => b,
            Err(_) => {
                // Unmapped page — skip forward a full chunk and keep going.
                cursor = cursor.saturating_add(CHUNK);
                continue;
            }
        };

        // Walk every 4-byte-aligned offset that has ES_SIZE bytes behind it.
        let mut off = 0usize;
        while off + ES_SIZE <= want {
            let slice = &buf.0[off..off + ES_SIZE];
            let es: &EntityState = from_bytes(slice);
            if looks_like_entity(es) {
                hits.push(Hit {
                    addr: cursor + off,
                    number: es.number,
                    e_type: es.e_type,
                    client_num: es.client_num,
                    origin: (es.origin.x, es.origin.y, es.origin.z),
                });
            }
            off += 4;
        }

        cursor = cursor.saturating_add(CHUNK);
    }

    if hits.is_empty() {
        println!("No plausible EntityState found in the scanned window.");
        println!("Try widening the range (e.g. 0x200000) or a different center.");
        return ExitCode::SUCCESS;
    }

    // Dedupe overlaps: a real EntityState at X produces phantom matches
    // at X+4, X+8, ..., X+204 because the shifted windows are still all
    // zeros / near-zeros. Keep the first hit of each ES_SIZE cluster.
    hits.sort_by_key(|h| h.addr);
    let mut deduped: Vec<Hit> = Vec::new();
    for h in hits {
        if let Some(last) = deduped.last() {
            if h.addr < last.addr + ES_SIZE {
                continue;
            }
        }
        deduped.push(h);
    }
    let hits = deduped;

    println!("Found {} candidate EntityState(s) after dedup:\n", hits.len());
    for h in &hits {
        println!(
            "  0x{:016X}  number={:>4}  e_type={:>2}  client_num={:>3}  origin=({:>8.1},{:>8.1},{:>8.1})",
            h.addr, h.number, h.e_type, h.client_num,
            h.origin.0, h.origin.1, h.origin.2
        );
    }

    // Array detection: if a run of hits shows a constant stride, that
    // stride is sizeof(centity_t) and the first hit is cg_entities[0].
    println!();
    report_strides(&hits);

    ExitCode::SUCCESS
}

/// A successful match plus a few fields to help identify it.
struct Hit {
    addr: usize,
    number: i32,
    e_type: i32,
    client_num: i32,
    origin: (f32, f32, f32),
}

/// Fixed-size byte buffer we can hand to [`ProcessHandle::read`].
/// `Pod` + size known at compile time → one Win32 round-trip per chunk.
#[repr(C)]
#[derive(Copy, Clone)]
struct Chunk([u8; CHUNK + ES_SIZE]);

// SAFETY: a plain byte array is trivially Pod/Zeroable.
unsafe impl Zeroable for Chunk {}
unsafe impl Pod for Chunk {}

/// Strict filter — we accept any valid entity (player, item, missile,
/// mover…), relying on cross-field coherence to keep noise out. On an
/// active map this fills in almost every populated slot of
/// `cg_entities[]`, giving us enough hits to detect the array stride.
fn looks_like_entity(es: &EntityState) -> bool {
    if !(0..=12).contains(&es.e_type) {
        return false;
    }
    // number is the entity slot (0..MAX_GENTITIES).
    if !(0..MAX_GENTITIES as i32).contains(&es.number) {
        return false;
    }
    // client_num is -1 for non-player entities, 0..MAX_CLIENTS for players.
    if !(-1..MAX_CLIENTS as i32).contains(&es.client_num) {
        return false;
    }
    // Players use interpolated trajectories (INTERPOLATE=1) in cgame.
    if !(0..=5).contains(&es.pos.tr_type) {
        return false;
    }
    // Origin must be finite, in-map, and non-zero (spawn origin is never (0,0,0)
    // for an active player).
    if !reasonable(es.origin.x) || !reasonable(es.origin.y) || !reasonable(es.origin.z) {
        return false;
    }
    // Require a non-trivial magnitude: real player origins are hundreds
    // to thousands of units from map origin, never sub-unit.
    let mag2 = es.origin.x * es.origin.x
        + es.origin.y * es.origin.y
        + es.origin.z * es.origin.z;
    if mag2 < 1.0 {
        return false;
    }
    // Cross-field coherence: pos.tr_base is the trajectory's base point,
    // which for a live player sits within a few units of the current
    // origin. Random bytes almost never satisfy this.
    if !close(es.pos.tr_base.x, es.origin.x, 4096.0)
        || !close(es.pos.tr_base.y, es.origin.y, 4096.0)
        || !close(es.pos.tr_base.z, es.origin.z, 4096.0)
    {
        return false;
    }
    // Weapon index must be sane (0..=15 in vanilla Q3).
    if !(0..=15).contains(&es.weapon) {
        return false;
    }
    true
}

fn close(a: f32, b: f32, tol: f32) -> bool {
    a.is_finite() && b.is_finite() && (a - b).abs() <= tol
}

fn reasonable(v: f32) -> bool {
    v.is_finite() && v.abs() < 32_768.0
}

/// Look for runs of hits separated by an identical stride. If we see a
/// stride repeat 3+ times, that's almost certainly `sizeof(centity_t)`.
fn report_strides(hits: &[Hit]) {
    if hits.len() < 3 {
        println!("Not enough hits to infer an array stride.");
        return;
    }
    let mut best_start = 0usize;
    let mut best_stride = 0usize;
    let mut best_run = 0usize;
    for i in 0..hits.len() - 1 {
        let stride = hits[i + 1].addr - hits[i].addr;
        if stride == 0 || stride > 8192 {
            continue;
        }
        let mut run = 1usize;
        let mut j = i;
        while j + 1 < hits.len() && hits[j + 1].addr - hits[j].addr == stride {
            run += 1;
            j += 1;
        }
        if run > best_run {
            best_run = run;
            best_stride = stride;
            best_start = i;
        }
    }
    if best_run >= 3 {
        let first = &hits[best_start];
        let last = &hits[best_start + best_run - 1];
        println!(
            "Detected repeating stride = {} bytes (0x{:X}) over {} consecutive hits.",
            best_stride, best_stride, best_run
        );
        println!(
            "  Run spans 0x{:016X} -> 0x{:016X}",
            first.addr, last.addr
        );
        println!("=> likely sizeof(centity_t). First hit is &cg_entities[slot].currentState.");
    } else {
        println!("No clear stride — the hits are scattered (not an array).");
    }
}

fn parse_hex(s: &str) -> Option<usize> {
    let stripped = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    usize::from_str_radix(stripped, 16).ok()
}
