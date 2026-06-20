//! Detect array layouts in scattered scan hits.
//!
//! Real arrays of `T` produce a chain of hits whose addresses are
//! separated by exactly `sizeof(T)` bytes — and any run of constant
//! gap is, statistically, almost never noise. This module finds the
//! longest such run.

/// A run of consecutive hits separated by an identical gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrideMatch {
    /// Constant gap (in bytes) between successive hits in the run.
    pub stride: usize,
    /// Number of hits that participate in the run.
    pub run_length: usize,
    /// Index in the input slice where the run starts.
    pub start_index: usize,
    /// Address of the first hit in the run.
    pub first_address: usize,
    /// Address of the last hit in the run.
    pub last_address: usize,
}

/// Find the longest run of consecutive addresses separated by an
/// identical gap, requiring at least `min_run` hits.
///
/// Returns `None` if no run reaches `min_run`. The addresses must be
/// sorted in increasing order; [`Vec::sort_by_key`] on a `Hit::address`
/// projection is the typical preprocessing step.
///
/// `max_stride` rejects gaps larger than the largest plausible struct
/// size for the target — keeps the search bounded against pathological
/// hit patterns.
pub fn detect_repeating_stride(
    addresses: &[usize],
    min_run: usize,
    max_stride: usize,
) -> Option<StrideMatch> {
    if addresses.len() < min_run.max(2) {
        return None;
    }

    let mut best: Option<StrideMatch> = None;
    let mut i = 0usize;
    while i + 1 < addresses.len() {
        let stride = addresses[i + 1].checked_sub(addresses[i])?;
        if stride == 0 || stride > max_stride {
            i += 1;
            continue;
        }
        let mut run = 1usize;
        let mut j = i;
        while j + 1 < addresses.len()
            && addresses[j + 1].checked_sub(addresses[j]) == Some(stride)
        {
            run += 1;
            j += 1;
        }
        if run >= min_run {
            let cand = StrideMatch {
                stride,
                run_length: run,
                start_index: i,
                first_address: addresses[i],
                last_address: addresses[j],
            };
            if best.map_or(true, |b| cand.run_length > b.run_length) {
                best = Some(cand);
            }
            // Skip past the run we already counted.
            i = j;
        } else {
            i += 1;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_constant_stride_run() {
        let addrs = [0x100, 0x200, 0x208, 0x210, 0x218, 0x220, 0x500];
        let got = detect_repeating_stride(&addrs, 3, 8192).unwrap();
        assert_eq!(got.stride, 8);
        assert_eq!(got.run_length, 5);
        assert_eq!(got.first_address, 0x200);
        assert_eq!(got.last_address, 0x220);
    }

    #[test]
    fn rejects_runs_below_threshold() {
        let addrs = [0x100, 0x108, 0x110, 0x500];
        assert!(detect_repeating_stride(&addrs, 5, 8192).is_none());
    }

    #[test]
    fn rejects_stride_above_max() {
        let addrs = [0x0, 0x10_000, 0x20_000, 0x30_000];
        assert!(detect_repeating_stride(&addrs, 3, 8192).is_none());
    }

    #[test]
    fn returns_none_on_empty() {
        assert!(detect_repeating_stride(&[], 2, 8192).is_none());
        assert!(detect_repeating_stride(&[0x100], 2, 8192).is_none());
    }
}
