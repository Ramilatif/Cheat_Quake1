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
