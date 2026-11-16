//! Plausibility filters for candidate structures found while scanning.
//!
//! These are pure functions on already-read structs: no I/O, no
//! Windows. They live in `sdk` (rather than in the CLI) so that the
//! exact rules used by every locator are written once and unit-tested
//! on any host.

use crate::constants::{MAX_CLIENTS, MAX_ENTITIES_IN_SNAPSHOT};
use crate::playerstate::STAT_HEALTH;
use crate::snapshot::SnapshotHeader;

/// How strict [`looks_like_snapshot`] is.
///
/// Both levels reproduce, verbatim, the rules that were previously
/// copy-pasted across the `qcheat` subcommands:
///
/// - [`Loose`](Self::Loose): the filter used by `aimbot`, `aim-mouse`,
///   `esp` and `find-viewangles`.
/// - [`Strict`](Self::Strict): the filter used by `snapshot`; it also
///   rejects stale / zero-filled blocks.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SnapshotFilter {
    /// Fast, permissive checks (ranges on a handful of fields).
    Loose,
    /// Loose checks plus rejection of dead, zeroed or never-ticked states.
    Strict,
}

/// Sanity-check a candidate [`SnapshotHeader`].
///
/// Returns `true` when every field is within the range a real in-game
/// frame can produce.
pub fn looks_like_snapshot(h: &SnapshotHeader, filter: SnapshotFilter) -> bool {
    let min_entities = match filter {
        SnapshotFilter::Loose => 1,
        SnapshotFilter::Strict => 0,
    };
    if !(min_entities..=MAX_ENTITIES_IN_SNAPSHOT as i32).contains(&h.num_entities) {
        return false;
    }
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

    if filter == SnapshotFilter::Strict {
        let origin_zero = o.x == 0.0 && o.y == 0.0 && o.z == 0.0;
        let velocity_zero =
            ps.velocity.x == 0.0 && ps.velocity.y == 0.0 && ps.velocity.z == 0.0;
        let angles_zero =
            ps.viewangles.x == 0.0 && ps.viewangles.y == 0.0 && ps.viewangles.z == 0.0;
        if origin_zero && velocity_zero && angles_zero && hp <= 0 {
            return false;
        }
        const PM_NORMAL: i32 = 0;
        if ps.pm_type == PM_NORMAL && (hp <= 0 || origin_zero) {
            return false;
        }
        if ps.command_time <= 0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn plausible() -> SnapshotHeader {
        let mut h = SnapshotHeader::zeroed();
        h.num_entities = 5;
        h.server_time = 123_456;
        h.ping = 20;
        h.ps.client_num = 0;
        h.ps.weapon = 2;
        h.ps.pm_type = 0;
        h.ps.origin.x = 100.0;
        h.ps.stats[STAT_HEALTH] = 100;
        h.ps.command_time = 99_999;
        h
    }

    #[test]
    fn zeroed_block_is_rejected_by_both_filters() {
        let h = SnapshotHeader::zeroed();
        assert!(!looks_like_snapshot(&h, SnapshotFilter::Loose));
        assert!(!looks_like_snapshot(&h, SnapshotFilter::Strict));
    }

    #[test]
    fn plausible_frame_passes_both() {
        let h = plausible();
        assert!(looks_like_snapshot(&h, SnapshotFilter::Loose));
        assert!(looks_like_snapshot(&h, SnapshotFilter::Strict));
    }

    #[test]
    fn strict_rejects_never_ticked_state_that_loose_accepts() {
        let mut h = plausible();
        h.ps.command_time = 0;
        assert!(looks_like_snapshot(&h, SnapshotFilter::Loose));
        assert!(!looks_like_snapshot(&h, SnapshotFilter::Strict));
    }

    #[test]
    fn empty_entity_list_only_passes_strict() {
        let mut h = plausible();
        h.num_entities = 0;
        assert!(!looks_like_snapshot(&h, SnapshotFilter::Loose));
        assert!(looks_like_snapshot(&h, SnapshotFilter::Strict));
    }

    #[test]
    fn non_finite_origin_is_rejected() {
        let mut h = plausible();
        h.ps.origin.y = f32::NAN;
        assert!(!looks_like_snapshot(&h, SnapshotFilter::Loose));
    }
}
