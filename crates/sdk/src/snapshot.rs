//! Cgame-side per-frame world view (`snapshot_t`).
//!
//! `cg.snap` points at one of `cg.activeSnapshots[2]`. Each snapshot
//! holds the local [`PlayerState`] plus a packed array of every visible
//! [`EntityState`] for that frame. Reading just this struct is enough
//! to drive an ESP or aimbot — no need to touch the engine's
//! `parseEntities[]` ring buffer.
//!
//! Field order matches `code/cgame/cg_public.h`. Total ~53.8 KiB.

use crate::constants::{MAX_ENTITIES_IN_SNAPSHOT, MAX_MAP_AREA_BYTES};
use crate::entitystate::EntityState;
use crate::playerstate::PlayerState;
use bytemuck::{Pod, Zeroable};

/// Snapshot header that precedes the entities array. Reading just this
/// (516 bytes) is enough to validate that a candidate address is a real
/// snapshot, without paying for the full 53 KiB read on every probe.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SnapshotHeader {
    /// Snapshot flags (`SNAPFLAG_RATE_DELAYED`, …).
    pub snap_flags: i32,
    /// Round-trip ping in ms at the time of the snapshot.
    pub ping: i32,
    /// Server time the snapshot is valid for (ms).
    pub server_time: i32,
    /// PVS area-visibility bit vector.
    pub areamask: [u8; MAX_MAP_AREA_BYTES],
    /// Local player state for this frame.
    pub ps: PlayerState,
    /// Count of valid entries in the following `entities[]` array
    /// (0..=[`MAX_ENTITIES_IN_SNAPSHOT`]).
    pub num_entities: i32,
}

const _: () = assert!(core::mem::size_of::<SnapshotHeader>() == 516);
const _: () = assert!(core::mem::offset_of!(SnapshotHeader, ps) == 44);
const _: () = assert!(core::mem::offset_of!(SnapshotHeader, num_entities) == 512);

/// Full `snapshot_t` mirror. ~53.8 KiB — prefer reading
/// [`SnapshotHeader`] first to find candidates, then this only on hits.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Snapshot {
    /// Header (snap_flags, ping, serverTime, areamask, ps, num_entities).
    pub header: SnapshotHeader,
    /// Visible entities for this frame. Only the first `header.num_entities`
    /// slots are populated — the rest is leftover data from prior frames.
    pub entities: [EntityState; MAX_ENTITIES_IN_SNAPSHOT],
    /// Number of text-based server commands queued for execution.
    pub num_server_commands: i32,
    /// Sequence id of the first such command.
    pub server_command_sequence: i32,
}

// SAFETY: only Pod fields, repr(C), no padding. Manual impls because
// `[EntityState; 256]` isn't covered by the derive's default bounds.
unsafe impl Zeroable for Snapshot {}
unsafe impl Pod for Snapshot {}

const _: () = assert!(core::mem::size_of::<Snapshot>() == 53_772);
const _: () = assert!(core::mem::offset_of!(Snapshot, entities) == 516);
