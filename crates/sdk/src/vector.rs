//! Quake's `vec3_t` — three single-precision floats, 12 bytes, 4-byte aligned.

use bytemuck::{Pod, Zeroable};

/// Three-component float vector, matching `vec3_t`.
///
/// Quake's world coordinate system:
/// - `x` points forward (north in most maps)
/// - `y` points left
/// - `z` points up
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vec3 {
    /// X component (forward).
    pub x: f32,
    /// Y component (left).
    pub y: f32,
    /// Z component (up).
    pub z: f32,
}

impl Vec3 {
    /// The zero vector.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0 };

    /// Build a vector from its components.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Component-wise subtraction. Convenient for building direction
    /// vectors (`enemy - me`).
    #[inline]
    pub fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }

    /// Squared Euclidean length. Prefer this over [`Self::length`]
    /// whenever you only need to compare distances.
    #[inline]
    pub fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// Euclidean length.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_sq().sqrt()
    }
}

const _: () = assert!(core::mem::size_of::<Vec3>() == 12);
const _: () = assert!(core::mem::align_of::<Vec3>() == 4);
