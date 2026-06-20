//! `qcheat players` — heap-wide scan for live player entities.

use anyhow::Result;
use clap::Args as ClapArgs;
use sdk::{EntityState, EntityType, MAX_CLIENTS};

use crate::util::{parse_hex, DEFAULT_PROCESS};

/// Heap window in which Quake's QVM keeps its data in observed builds.
const SCAN_CENTER: usize = 0x06800000;
const SCAN_RANGE: usize = 0x01800000;

/// Arguments accepted by `qcheat players`.
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

/// Walk the heap window and print every entry that looks like a live
/// PLAYER entity.
pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;
    println!("Attached to {} (pid {})", proc.name, proc.pid);

    let center = args.center.unwrap_or(SCAN_CENTER);
    let range = args.range.unwrap_or(SCAN_RANGE);
    let start = center.saturating_sub(range);
    let end = center.saturating_add(range);
    println!(
        "Scanning heap 0x{:016X}..0x{:016X} ({} KiB) for live players...\n",
        start,
        end,
        (end - start) / 1024
    );

    let raw_hits = scanner::scan_aligned::<EntityState, _>(
        &handle,
        start,
        end,
        4,
        is_live_player,
    )?;

    // Dedupe overlaps the same way scan does.
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

    if deduped.is_empty() {
        println!("No live players found in the scan window.");
        println!("Make sure Quake is in-game and the snapshot has been received.");
        return Ok(());
    }

    println!("Found {} player entity copy/copies:\n", deduped.len());
    println!(
        "{:<18} {:<5} {:<7} {:<7} {}",
        "address", "num", "client", "weapon", "origin"
    );
    println!("{:-<18} {:-<5} {:-<7} {:-<7} {:-<36}", "", "", "", "", "");
    for h in &deduped {
        let es = &h.value;
        println!(
            "0x{:016X} {:<5} {:<7} {:<7} ({:>8.1}, {:>8.1}, {:>8.1})",
            h.address,
            es.number,
            es.client_num,
            es.weapon,
            es.origin.x,
            es.origin.y,
            es.origin.z
        );
    }
    println!("\nNote: the engine keeps several copies per player");
    println!(
        "      (currentState, nextState, snapshot buffers). \
         Multiple hits per client_num is normal."
    );
    Ok(())
}

/// Strict filter: every field must be consistent with a real Q3 player
/// entity, so random memory with `e_type == 1` is rejected.
fn is_live_player(es: &EntityState) -> bool {
    if es.e_type != EntityType::PLAYER {
        return false;
    }
    if !(0..MAX_CLIENTS as i32).contains(&es.number) {
        return false;
    }
    if !(0..MAX_CLIENTS as i32).contains(&es.client_num) {
        return false;
    }
    if !(0..=9).contains(&es.weapon) {
        return false;
    }
    let (ox, oy, oz) = (es.origin.x, es.origin.y, es.origin.z);
    if !ox.is_finite() || !oy.is_finite() || !oz.is_finite() {
        return false;
    }
    if ox.abs() < 1.0 || oy.abs() < 1.0 {
        return false;
    }
    if ox.abs() >= 32_768.0 || oy.abs() >= 32_768.0 || oz.abs() >= 32_768.0 {
        return false;
    }
    (es.pos.tr_base.x - ox).abs() < 256.0
        && (es.pos.tr_base.y - oy).abs() < 256.0
        && (es.pos.tr_base.z - oz).abs() < 256.0
}
