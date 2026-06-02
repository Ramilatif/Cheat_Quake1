//! Motion curves used by networked entities (`trajectory_t`).

use crate::vector::Vec3;
use bytemuck::{Pod, Zeroable};

/// Kind of motion curve attached to a trajectory, namespace-only.
///
/// In C this is `trType_t`, an anonymous `enum` stored as a 4-byte `int`.
/// We expose the discriminants as `i32` associated constants rather than
/// a Rust `enum` so an unknown value read from memory stays representable
/// instead of being undefined behaviour.
pub struct TrType;

impl TrType {
    /// Fixed position: use `tr_base`.
    pub const STATIONARY: i32 = 0;
    /// Interpolated between snapshots (used for players).
    pub const INTERPOLATE: i32 = 1;
    /// Unbounded linear motion.
    pub const LINEAR: i32 = 2;
    /// Linear for `tr_duration` ms then frozen.
    pub const LINEAR_STOP: i32 = 3;
    /// Sine-wave oscillation.
    pub const SINE: i32 = 4;
    /// Parabolic fall under gravity.
    pub const GRAVITY: i32 = 5;
}

/// Motion parameters attached to an entity's position or angles.
///
/// Layout (36 bytes): `trType(4)` + `trTime(4)` + `trDuration(4)` +
/// `trBase(12)` + `trDelta(12)`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Trajectory {
    /// Motion kind — compare against the [`TrType`] constants.
    pub tr_type: i32,
    /// Server time (ms) the curve started at.
    pub tr_time: i32,
    /// Duration (ms), meaningful for [`TrType::LINEAR_STOP`].
    pub tr_duration: i32,
    /// Position at `tr_time`.
    pub tr_base: Vec3,
    /// Velocity or direction, depending on `tr_type`.
    pub tr_delta: Vec3,
}

const _: () = assert!(core::mem::size_of::<Trajectory>() == 36);
