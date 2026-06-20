//! Chunked aligned scan over a process address space.
//!
//! Streams a window of the target's memory through a reusable buffer
//! and hands every `T`-sized aligned slice to a caller-supplied
//! predicate. Used by every locator in the codebase (entities,
//! snapshots, future server-side g_entities) to centralise the
//! "read 4 KiB, slide a window, skip unmapped pages" boilerplate.

use bytemuck::{from_bytes, Pod};
use process::{ProcessHandle, ReadError};

/// Size of each `ReadProcessMemory` round-trip during a scan.
///
/// 4 KiB matches the Windows page size: a single unmapped page kills
/// at most one chunk, and chunks fit cleanly in L1.
const CHUNK_SIZE: usize = 4096;

/// A successful match during a scan.
#[derive(Debug, Clone, Copy)]
pub struct Hit<T> {
    /// Virtual address in the target where the matched value starts.
    pub address: usize,
    /// Copy of the matched value.
    pub value: T,
}

/// Walk `[start, end)` in the target's address space, treat every
/// `alignment`-aligned offset as a candidate `T`, and collect every
/// candidate the predicate accepts.
///
/// Unmapped pages (read errors) skip forward one chunk and the scan
/// keeps going — partial coverage is fine, total failure isn't.
///
/// # Arguments
/// - `handle`: opened process to read from.
/// - `start`, `end`: half-open window in the target's address space.
/// - `alignment`: stride between candidate offsets within a chunk. Use
///   `4` for 4-byte-aligned C structs, `8` for pointer-sized scans.
/// - `predicate`: returns `true` for the candidates to keep.
///
/// The returned `Vec` is in increasing address order. Adjacent hits
/// produced by sliding window aliasing (a real match at X also passes
/// at X+4, X+8, …) are **not** deduplicated here — that's the caller's
/// call, because the correct dedup distance depends on `T`.
pub fn scan_aligned<T, F>(
    handle: &ProcessHandle,
    start: usize,
    end: usize,
    alignment: usize,
    mut predicate: F,
) -> Result<Vec<Hit<T>>, ReadError>
where
    T: Pod,
    F: FnMut(&T) -> bool,
{
    assert!(alignment > 0, "alignment must be non-zero");
    let t_size = core::mem::size_of::<T>();
    // The reusable buffer carries CHUNK_SIZE + t_size bytes so a `T`
    // straddling a chunk boundary is still recognised on the next pass.
    let mut buf = vec![0u8; CHUNK_SIZE + t_size];

    let mut hits = Vec::new();
    let mut cursor = start;
    while cursor < end {
        if handle.read_into(cursor, &mut buf).is_err() {
            // Unmapped page in this range — skip ahead and keep going.
            cursor = cursor.saturating_add(CHUNK_SIZE);
            continue;
        }

        let mut off = 0usize;
        while off + t_size <= buf.len() {
            let slice = &buf[off..off + t_size];
            let candidate: &T = from_bytes(slice);
            if predicate(candidate) {
                hits.push(Hit {
                    address: cursor + off,
                    value: *candidate,
                });
            }
            off += alignment;
        }
        cursor = cursor.saturating_add(CHUNK_SIZE);
    }
    Ok(hits)
}
