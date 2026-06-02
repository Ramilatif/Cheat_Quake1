//! Quake III Arena SDK.
//!
//! Rust mirrors of the engine's C structures, transposed verbatim from
//! ioquake3's headers (`code/qcommon/q_shared.h`, `code/cgame/cg_public.h`).
//! Every struct is `#[repr(C)]` and carries compile-time size assertions
//! so a drift from the reference layout fails at build time, not silently
//! at runtime.
//!
//! Scope today is the cgame-side per-frame view used by an ESP:
//! - [`constants`] — MAX_* bounds
//! - [`vector`] — `vec3_t`
//! - [`trajectory`] — `trajectory_t` / `trType_t`
//! - [`entitystate`] — `entityState_t` (networked state of any world object)
//! - [`playerstate`] — `playerState_t` (local player view)
//! - [`snapshot`] — `snapshot_t` (one frame as cg sees it)

#![warn(missing_docs)]

pub mod constants;
pub mod entitystate;
pub mod playerstate;
pub mod snapshot;
pub mod trajectory;
pub mod vector;

pub use constants::*;
pub use entitystate::{EntityState, EntityType};
pub use playerstate::{PlayerState, STAT_ARMOR, STAT_HEALTH, STAT_MAX_HEALTH};
pub use snapshot::{Snapshot, SnapshotHeader};
pub use trajectory::{TrType, Trajectory};
pub use vector::Vec3;
