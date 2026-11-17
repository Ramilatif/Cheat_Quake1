//! Shared locators used by several subcommands.
//!
//! Until now every command that needed `cg.activeSnapshots` carried its
//! own copy of the scan loop and of the plausibility filter. Both now
//! live once: the filter in [`sdk::plausibility`] (pure, unit-tested)
//! and the scan loop here.

use process::ProcessHandle;
use sdk::{looks_like_snapshot, Snapshot, SnapshotFilter, SnapshotHeader};

/// Bytes advanced per read while scanning.
pub const CHUNK: usize = 4096;

/// Default centre of the cgame heap window observed on our builds.
pub const DEFAULT_SNAPSHOT_CENTER: usize = 0x07000000;
/// Default half-range of that window.
pub const DEFAULT_SNAPSHOT_RANGE: usize = 0x02000000;

const HEADER_SIZE: usize = core::mem::size_of::<SnapshotHeader>();
// The scan buffer is a `Vec<u32>` so candidate headers are 4-byte
// aligned; that only works if every step is a multiple of 4.
const _: () = assert!((CHUNK + HEADER_SIZE) % 4 == 0 && HEADER_SIZE % 4 == 0);

/// Walk `[start, end)` and return the addresses that pass
/// [`looks_like_snapshot`] with the given `filter`.
///
/// Behaviour matches the loops it replaces: 4-byte steps, unreadable
/// chunks are skipped, and after a hit the scan jumps past the whole
/// header so one real snapshot does not produce dozens of aliases.
pub fn find_snapshot_candidates(
    handle: &ProcessHandle,
    start: usize,
    end: usize,
    filter: SnapshotFilter,
) -> Vec<usize> {
    let mut words = vec![0u32; (CHUNK + HEADER_SIZE) / 4];
    let mut hits = Vec::new();
    let mut cursor = start;

    while cursor < end {
        let buf: &mut [u8] = bytemuck::cast_slice_mut(&mut words);
        if handle.read_into(cursor, buf).is_err() {
            cursor = cursor.saturating_add(CHUNK);
            continue;
        }

        let mut off = 0usize;
        while off + HEADER_SIZE <= buf.len() {
            let header: &SnapshotHeader = bytemuck::from_bytes(&buf[off..off + HEADER_SIZE]);
            if looks_like_snapshot(header, filter) {
                hits.push(cursor + off);
                off += HEADER_SIZE;
            } else {
                off += 4;
            }
        }
        cursor = cursor.saturating_add(CHUNK);
    }
    hits
}

/// Full scan for `cg.activeSnapshots`.
///
/// Returns the confirmed pair (two candidates exactly
/// `sizeof(snapshot_t)` apart) or, failing that, the first candidate
/// alone. Expensive: call once, then poll the returned addresses.
pub fn locate_snapshot_addrs(
    handle: &ProcessHandle,
    start: usize,
    end: usize,
    filter: SnapshotFilter,
) -> Vec<usize> {
    let hits = find_snapshot_candidates(handle, start, end, filter);
    if hits.is_empty() {
        return Vec::new();
    }
    const SS_SIZE: usize = core::mem::size_of::<Snapshot>();
    match hits.windows(2).find(|w| w[1] - w[0] == SS_SIZE) {
        Some(w) => vec![w[0], w[1]],
        None => vec![hits[0]],
    }
}

/// Of the given snapshot addresses, the one with the largest
/// `serverTime` (the active `cg.snap`), if any is readable.
pub fn newest_snapshot_addr(handle: &ProcessHandle, addrs: &[usize]) -> Option<usize> {
    addrs
        .iter()
        .filter_map(|&a| {
            handle
                .read::<SnapshotHeader>(a)
                .ok()
                .map(|h| (h.server_time, a))
        })
        .max_by_key(|&(t, _)| t)
        .map(|(_, a)| a)
}

// ---------------------------------------------------------------------
// g_entities[] (server-side array, listen server only)
// ---------------------------------------------------------------------

/// Default centre of the window scanned for the server entity array.
pub const DEFAULT_SERVER_CENTER: usize = 0x07000000;
/// Default half-range of that window.
pub const DEFAULT_SERVER_RANGE: usize = 0x04000000;
/// Max distance allowed between `r.currentOrigin` and `s.pos.trBase`
/// of one candidate entity (should be near-zero for a real entity).
const SERVER_ORIGIN_TOLERANCE: f32 = 64.0;

/// Parameters for [`locate_gentities`].
pub struct GentitiesLookup {
    /// Centre of the scan window.
    pub center: usize,
    /// Half-range of the scan window.
    pub range: usize,
    /// Max world-unit gap between a client player and its server twin.
    pub tolerance: f32,
    /// Force `sizeof(gentity_t)` instead of inferring it.
    pub stride: Option<usize>,
}

/// Find `g_entities[]` by matching known client-side players against
/// memory, inferring the array stride, then cross-checking against
/// the engine's own `sv.gentities` pointer. Returns `(base, stride)`.
///
/// Shared by `server-entities` and `esp --server`.
pub fn locate_gentities(
    handle: &ProcessHandle,
    client_players: &[sdk::EntityState],
    opts: &GentitiesLookup,
) -> anyhow::Result<(usize, usize)> {
    use anyhow::{bail, Context};
    use std::collections::HashMap;

    if client_players.is_empty() {
        bail!("no player in the client snapshot to use as ground truth. Load a map with bots, or pass --sv-rva.");
    }
    if client_players.len() < 2 && opts.stride.is_none() {
        bail!("only one player visible: cannot infer sizeof(gentity_t). Pass --stride, or face a bot and retry.");
    }

    let (start, end) = (
        opts.center.saturating_sub(opts.range),
        opts.center.saturating_add(opts.range),
    );
    println!(
        "Scanning 0x{start:X}..0x{end:X} ({} MiB) for the server twin of {} player(s)...",
        (end - start) / (1024 * 1024),
        client_players.len()
    );

    let tol = opts.tolerance;
    let mut hits = scanner::scan_aligned::<sdk::SharedEntity, _>(handle, start, end, 4, |e| {
        client_players
            .iter()
            .any(|r| sdk::server::is_server_twin(e, r, tol, SERVER_ORIGIN_TOLERANCE))
    })?;
    // A hit exactly on a chunk boundary is seen by two consecutive reads.
    hits.sort_by_key(|h| h.address);
    hits.dedup_by_key(|h| h.address);
    if hits.is_empty() {
        bail!("no server-side twin found. Is this a listen server (/devmap)? Widen --center/--range or raise --tolerance.");
    }
    for h in &hits {
        println!("  twin of slot {:>3} at 0x{:016X}", h.value.s.number, h.address);
    }

    let stride = match opts.stride {
        Some(s) => s,
        None => {
            let anchors: Vec<(i32, usize)> =
                hits.iter().map(|h| (h.value.s.number, h.address)).collect();
            let g = sdk::server::infer_gentity_stride(&anchors)
                .context("stride inference failed (hits disagree or only one slot matched); pass --stride")?;
            println!(
                "  inferred sizeof(gentity_t) = {} (0x{:X}), {} agreeing pair(s)",
                g.stride, g.stride, g.votes
            );
            g.stride
        }
    };

    // Several hits may belong to the same array: vote on the base.
    let mut bases: HashMap<usize, usize> = HashMap::new();
    for h in &hits {
        if let Some(b) = sdk::gentities_base_checked(h.address, h.value.s.number, stride) {
            *bases.entry(b).or_insert(0) += 1;
        }
    }
    let (&base, &votes) = bases
        .iter()
        .max_by_key(|&(b, v)| (*v, std::cmp::Reverse(*b)))
        .context("no consistent base address")?;
    println!("  base 0x{base:016X} ({votes} twin(s) agree)");

    engine_cross_check(handle, base, stride);
    Ok((base, stride))
}

/// Look for `sv.gentities` in the exe image: an 8-byte pointer equal
/// to `base`, followed by `gentitySize` (an `int` equal to `stride`).
fn engine_cross_check(handle: &ProcessHandle, base: usize, stride: usize) {
    let Ok(proc) = process::find_by_name(crate::util::DEFAULT_PROCESS) else {
        return;
    };
    let (start, end) = (proc.base_address, proc.base_address + proc.module_size);
    match scanner::scan_aligned::<u64, _>(handle, start, end, 8, |v| *v == base as u64) {
        Ok(mut hits) if !hits.is_empty() => {
            hits.sort_by_key(|h| h.address);
            hits.dedup_by_key(|h| h.address);
            for h in hits {
                let size = handle.read::<i32>(h.address + 8).unwrap_or(-1);
                let num = handle.read::<i32>(h.address + 12).unwrap_or(-1);
                let tag = if size as usize == stride { "MATCH sv.gentities" } else { "pointer only" };
                println!(
                    "  engine: exe+0x{:X} holds the base; +8 = {size}, +12 = {num}  [{tag}]",
                    h.address - proc.base_address
                );
            }
        }
        _ => println!("  engine: no pointer to the base found in the exe image."),
    }
}
