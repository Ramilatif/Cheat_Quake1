//! `qcheat scan` — brute-scan a window for entityState_t-shaped bytes
//! and report any repeating stride.

use anyhow::Result;
use clap::Args as ClapArgs;
use sdk::{EntityState, MAX_CLIENTS, MAX_GENTITIES};

use crate::util::{parse_hex, DEFAULT_PROCESS};

/// Arguments accepted by `qcheat scan`.
#[derive(ClapArgs)]
pub struct Args {
    /// Center address of the scan window (hex).
    #[arg(value_parser = parse_hex)]
    pub center: usize,

    /// Half-range of the window (hex). Default ±512 KiB.
    #[arg(value_parser = parse_hex, default_value = "0x80000")]
    pub range: usize,

    /// Name of the target executable.
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,
}

/// Walk the window, report hits, then run stride detection.
pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;

    let start = args.center.saturating_sub(args.range);
    let end = args.center.saturating_add(args.range);
    println!(
        "Attached to {} (pid {}). Scanning 0x{:016X}..0x{:016X} ({} KiB).\n",
        proc.name,
        proc.pid,
        start,
        end,
        (end - start) / 1024
    );

    let raw_hits = scanner::scan_aligned::<EntityState, _>(
        &handle,
        start,
        end,
        4,
        looks_like_entity,
    )?;

    if raw_hits.is_empty() {
        println!("No plausible EntityState found in the scanned window.");
        println!("Try widening the range (e.g. 0x200000) or a different center.");
        return Ok(());
    }

    // Dedupe overlaps: one real EntityState at X also matches shifted
    // reads up to X+204 because the leftover bytes are mostly zero.
    const ES_SIZE: usize = core::mem::size_of::<EntityState>();
    let mut hits = raw_hits;
    hits.sort_by_key(|h| h.address);
    let mut deduped = Vec::<scanner::scan::Hit<EntityState>>::with_capacity(hits.len());
    for h in hits {
        if let Some(last) = deduped.last() {
            if h.address < last.address + ES_SIZE {
                continue;
            }
        }
        deduped.push(h);
    }
    let hits = deduped;

    println!("Found {} candidate EntityState(s) after dedup:\n", hits.len());
    for h in &hits {
        println!(
            "  0x{:016X}  number={:>4}  e_type={:>2}  client_num={:>3}  \
             origin=({:>8.1},{:>8.1},{:>8.1})",
            h.address,
            h.value.number,
            h.value.e_type,
            h.value.client_num,
            h.value.origin.x,
            h.value.origin.y,
            h.value.origin.z
        );
    }

    println!();
    let addrs: Vec<usize> = hits.iter().map(|h| h.address).collect();
    match scanner::detect_repeating_stride(&addrs, 3, 8192) {
        Some(m) => {
            println!(
                "Detected repeating stride = {} bytes (0x{:X}) over {} consecutive hits.",
                m.stride, m.stride, m.run_length
            );
            println!(
                "  Run spans 0x{:016X} -> 0x{:016X}",
                m.first_address, m.last_address
            );
            println!("=> likely sizeof(centity_t). First hit is &cg_entities[slot].currentState.");
        }
        None => {
            println!("No clear stride — the hits are scattered (not an array).");
        }
    }
    Ok(())
}

/// Strict filter: accept any plausible entity. Cross-field coherence
/// keeps noise out on the few hundred KiB-wide windows we scan.
fn looks_like_entity(es: &EntityState) -> bool {
    if !(0..=12).contains(&es.e_type) {
        return false;
    }
    if !(0..MAX_GENTITIES as i32).contains(&es.number) {
        return false;
    }
    if !(-1..MAX_CLIENTS as i32).contains(&es.client_num) {
        return false;
    }
    if !(0..=5).contains(&es.pos.tr_type) {
        return false;
    }
    if !reasonable(es.origin.x) || !reasonable(es.origin.y) || !reasonable(es.origin.z) {
        return false;
    }
    let mag2 = es.origin.x * es.origin.x
        + es.origin.y * es.origin.y
        + es.origin.z * es.origin.z;
    if mag2 < 1.0 {
        return false;
    }
    if !close(es.pos.tr_base.x, es.origin.x, 4096.0)
        || !close(es.pos.tr_base.y, es.origin.y, 4096.0)
        || !close(es.pos.tr_base.z, es.origin.z, 4096.0)
    {
        return false;
    }
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
