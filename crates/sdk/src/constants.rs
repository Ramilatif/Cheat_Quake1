//! Quake III engine constants.
//!
//! Values taken from `code/qcommon/q_shared.h` in ioquake3. These drive the
//! fixed-size arrays inside [`crate::entitystate::EntityState`] and future
//! `playerState_t` / `centity_t` mirrors.

/// Maximum number of connected clients (player slots).
pub const MAX_CLIENTS: usize = 64;

/// Upper bound on networked game entities. Defined as `1 << 10` in the
/// engine — players + projectiles + items + world objects all share this
/// budget.
pub const MAX_GENTITIES: usize = 1024;

/// Per-player stat slots (health, armor, ammo indices, …).
pub const MAX_STATS: usize = 16;

/// Persistent values carried across respawns (score, kills, …).
pub const MAX_PERSISTANT: usize = 16;

/// Powerup expiry timestamps, indexed by powerup id.
pub const MAX_POWERUPS: usize = 16;

/// Ammo counts, indexed by weapon id.
pub const MAX_WEAPONS: usize = 16;

/// Events batched into a single `playerState_t` snapshot.
pub const MAX_PS_EVENTS: usize = 2;
