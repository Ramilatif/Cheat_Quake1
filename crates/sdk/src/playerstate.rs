//! Local player snapshot (`playerState_t`).
//!
//! Sits inside `snapshot_t` (cgame) / `clSnapshot_t` (engine) as the
//! per-frame view of the player owning the connection. It carries
//! everything an ESP / aimbot wants to know about *us*: position,
//! velocity, view angles, weapon, ammo, and the `stats[]` HUD values
//! (HP, armor…).
//!
//! Field order is frozen to match `code/qcommon/q_shared.h`. Total 468 B.

use crate::vector::Vec3;
use bytemuck::{Pod, Zeroable};

use crate::constants::{MAX_PERSISTANT, MAX_POWERUPS, MAX_PS_EVENTS, MAX_STATS, MAX_WEAPONS};

/// `stats[]` slot for current HP. Matches the HUD value in real time.
pub const STAT_HEALTH: usize = 0;
/// `stats[]` slot for armor amount.
pub const STAT_ARMOR: usize = 3;
/// `stats[]` slot for the soft HP cap (regen target, handicap…).
pub const STAT_MAX_HEALTH: usize = 6;

/// Networked per-frame state of the local player (`playerState_t`).
///
/// Read it from the current `snapshot_t.ps` (cgame) — every field below
/// is what the engine itself uses to render the HUD, run prediction,
/// and resolve commands. `stats[STAT_HEALTH]` is the live HP value.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PlayerState {
    /// `cmd->serverTime` of the last executed command.
    pub command_time: i32,
    /// Pmove kind (normal, dead, spectator, …).
    pub pm_type: i32,
    /// View bob counter, driven by movement.
    pub bob_cycle: i32,
    /// Pmove flag bitmask (ducked, jump-held, …).
    pub pm_flags: i32,
    /// Pmove timer for the current locked animation.
    pub pm_time: i32,

    /// World-space origin of the player.
    pub origin: Vec3,
    /// Linear velocity vector (units / second).
    pub velocity: Vec3,

    /// Cooldown ms until the current weapon can fire again.
    pub weapon_time: i32,
    /// Gravity strength applied this frame.
    pub gravity: i32,
    /// Max horizontal move speed in units / sec.
    pub speed: i32,
    /// Angle correction added to client cmd angles each frame.
    pub delta_angles: [i32; 3],

    /// Entity slot we're standing on, or `ENTITYNUM_NONE`.
    pub ground_entity_num: i32,

    /// Lock-out ms for the lower-body animation.
    pub legs_timer: i32,
    /// Current lower-body animation id (mask off `ANIM_TOGGLEBIT`).
    pub legs_anim: i32,
    /// Lock-out ms for the upper-body animation.
    pub torso_timer: i32,
    /// Current upper-body animation id.
    pub torso_anim: i32,

    /// 0..7 movement direction relative to view, for leg twist.
    pub movement_dir: i32,

    /// Grapple anchor point when `PMF_GRAPPLE_PULL` is set.
    pub grapple_point: Vec3,

    /// Engine + gamecode flag bitmask, mirrored into [`crate::EntityState`].
    pub e_flags: i32,

    /// Monotonic per-pmove event counter.
    pub event_sequence: i32,
    /// Event ids queued this frame.
    pub events: [i32; MAX_PS_EVENTS],
    /// Parameters paired with `events`.
    pub event_parms: [i32; MAX_PS_EVENTS],

    /// Server-injected one-shot event.
    pub external_event: i32,
    /// Parameter for `external_event`.
    pub external_event_parm: i32,
    /// Server time of `external_event`.
    pub external_event_time: i32,

    /// Owning client slot (0..[`MAX_CLIENTS`](crate::MAX_CLIENTS)).
    pub client_num: i32,
    /// Currently held weapon index, mirrored into entity state.
    pub weapon: i32,
    /// Internal weapon FSM state (raising, firing, …).
    pub weapon_state: i32,

    /// View angles `[pitch, yaw, roll]` — what the camera looks at.
    pub viewangles: Vec3,
    /// Eye height above [`Self::origin`].
    pub viewheight: i32,

    /// Increments when damage is received; used to latch the rest of
    /// the damage block.
    pub damage_event: i32,
    /// Yaw the damage came from.
    pub damage_yaw: i32,
    /// Pitch the damage came from.
    pub damage_pitch: i32,
    /// Magnitude of the damage.
    pub damage_count: i32,

    /// HUD stat values — health, armor, max-health, weapon-bitmask, …
    /// See the [`STAT_HEALTH`], [`STAT_ARMOR`], [`STAT_MAX_HEALTH`] indexes.
    pub stats: [i32; MAX_STATS],
    /// Score/kills/etc. that survive respawn.
    pub persistant: [i32; MAX_PERSISTANT],
    /// Per-powerup expiry timestamps (server time, ms).
    pub powerups: [i32; MAX_POWERUPS],
    /// Per-weapon ammo counts.
    pub ammo: [i32; MAX_WEAPONS],

    /// Mod-defined value, mirrored into [`crate::EntityState`].
    pub generic1: i32,
    /// Looping sound index.
    pub loop_sound: i32,
    /// Jumppad entity hit this frame.
    pub jumppad_ent: i32,

    /// Ping in ms (set by server, scoreboard only).
    pub ping: i32,
    /// Monotonic pmove frame counter.
    pub pmove_framecount: i32,
    /// `pmove_framecount` value of last jumppad hit.
    pub jumppad_frame: i32,
    /// Counter for `entityEvent` deltas.
    pub entity_event_sequence: i32,
}

const _: () = assert!(core::mem::size_of::<PlayerState>() == 468);
const _: () = assert!(core::mem::offset_of!(PlayerState, origin) == 20);
const _: () = assert!(core::mem::offset_of!(PlayerState, client_num) == 140);
const _: () = assert!(core::mem::offset_of!(PlayerState, viewangles) == 152);
const _: () = assert!(core::mem::offset_of!(PlayerState, stats) == 184);
