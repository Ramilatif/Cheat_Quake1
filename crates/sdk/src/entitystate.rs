//! Networked entity snapshot (`entityState_t`).
//!
//! One of these exists in every `centity_t` slot as its `currentState`
//! field (i.e. at offset 0). Reading just this header — without touching
//! the rest of `centity_t` — is enough to know what an entity is, where
//! it is, and which client it belongs to. That's the minimum needed for
//! an ESP / aimbot.

use crate::trajectory::Trajectory;
use crate::vector::Vec3;
use bytemuck::{Pod, Zeroable};

/// `eType` discriminants as defined in `q_shared.h`. Exposed as `i32`
/// associated constants so unknown values (e.g. from a mod that added
/// entity kinds) read cleanly into a raw integer.
pub struct EntityType;

impl EntityType {
    /// Unclassified / generic entity.
    pub const GENERAL: i32 = 0;
    /// A player. The one type we care about for ESP / aimbot targeting.
    pub const PLAYER: i32 = 1;
    /// Pickable item (weapon, health, armor…).
    pub const ITEM: i32 = 2;
    /// In-flight projectile.
    pub const MISSILE: i32 = 3;
    /// Moving brush (door, platform, …).
    pub const MOVER: i32 = 4;
    /// Laser / rail trail beam.
    pub const BEAM: i32 = 5;
    /// Portal camera.
    pub const PORTAL: i32 = 6;
    /// Ambient sound emitter.
    pub const SPEAKER: i32 = 7;
    /// Jumppad / push trigger.
    pub const PUSH_TRIGGER: i32 = 8;
    /// Teleport trigger.
    pub const TELEPORT_TRIGGER: i32 = 9;
    /// Invisible marker / scriptable.
    pub const INVISIBLE: i32 = 10;
    /// Grappling-hook anchor.
    pub const GRAPPLE: i32 = 11;
    /// Team marker (flag base, obj, …).
    pub const TEAM: i32 = 12;
}

/// Networked state of any world entity (`entityState_t`).
///
/// Layout (208 bytes): 3 ints + 2 trajectories + 2 ints + 4 vec3 + 17
/// ints. Field order is frozen to match `q_shared.h`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct EntityState {
    /// Entity slot (0..[`MAX_GENTITIES`](crate::MAX_GENTITIES)).
    pub number: i32,
    /// Kind of entity — compare against the [`EntityType`] constants.
    pub e_type: i32,
    /// Engine + gamecode flag bitmask.
    pub e_flags: i32,

    /// Linear motion of this entity's origin.
    pub pos: Trajectory,
    /// Angular motion of its orientation.
    pub apos: Trajectory,

    /// Generic per-entity timestamp (event-specific).
    pub time: i32,
    /// Second generic timestamp.
    pub time2: i32,

    /// World-space origin (interpolated for non-linear `pos` types).
    pub origin: Vec3,
    /// Second origin slot — used by beams, portals, movers' dest.
    pub origin2: Vec3,
    /// World-space Euler angles `[pitch, yaw, roll]`.
    pub angles: Vec3,
    /// Second angle slot.
    pub angles2: Vec3,

    /// Entity this one is linked to (owner of a missile, attached portal…).
    pub other_entity_num: i32,
    /// Secondary linked entity.
    pub other_entity_num2: i32,
    /// Entity slot we're standing on, or `ENTITYNUM_NONE`.
    pub ground_entity_num: i32,

    /// Packed constant-light color + radius.
    pub constant_light: i32,
    /// Looping sound index.
    pub loop_sound: i32,

    /// Primary model index (resolved through `CS_MODELS` configstrings).
    pub modelindex: i32,
    /// Secondary model index (held weapon, skull, …).
    pub modelindex2: i32,

    /// For [`EntityType::PLAYER`], the slot of the owning client
    /// (0..[`MAX_CLIENTS`](crate::MAX_CLIENTS)).
    pub client_num: i32,
    /// Animation frame index.
    pub frame: i32,
    /// Encoded bounding-box / clip type.
    pub solid: i32,
    /// Queued event id (gunshot, pickup, …).
    pub event: i32,
    /// Parameter accompanying [`Self::event`].
    pub event_parm: i32,

    /// Bitmask of currently held powerups.
    pub powerups: i32,
    /// Currently held weapon index.
    pub weapon: i32,
    /// Lower-body animation id.
    pub legs_anim: i32,
    /// Upper-body animation id.
    pub torso_anim: i32,

    /// Free slot for mod-specific data.
    pub generic1: i32,
}

const _: () = assert!(core::mem::size_of::<EntityState>() == 208);
const _: () = assert!(core::mem::offset_of!(EntityState, number) == 0);
const _: () = assert!(core::mem::offset_of!(EntityState, e_type) == 4);
const _: () = assert!(core::mem::offset_of!(EntityState, origin) == 92);
const _: () = assert!(core::mem::offset_of!(EntityState, client_num) == 168);
