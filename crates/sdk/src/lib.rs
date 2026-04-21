//! Quake III Arena SDK.
//!
//! Rust mirrors of the engine's C structures, transposed verbatim from
//! ioquake3's headers (`code/qcommon/q_shared.h`). Every struct is
//! `#[repr(C)]` and carries compile-time size assertions so a drift from
//! the reference layout fails at build time, not silently at runtime.
//!
//! Scope for now is the minimum needed to enumerate entities:
//! - [`constants`] — MAX_* bounds
//! - [`vector`] — `vec3_t`
//! - [`trajectory`] — `trajectory_t` / `trType_t`
//! - [`entitystate`] — `entityState_t` (networked state of any world object)

#![warn(missing_docs)]

pub mod constants;
pub mod entitystate;
pub mod trajectory;
pub mod vector;

pub use constants::*;
pub use entitystate::{EntityState, EntityType};
pub use trajectory::{TrType, Trajectory};
pub use vector::Vec3;
