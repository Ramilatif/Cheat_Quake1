//! Server-side view of an entity: the head of the game module's
//! `gentity_t` (`entityState_t s` followed by `entityShared_t r`).
//!
//! The engine does not know the full `gentity_t` — only its first
//! fields, in this exact order, and its total size, which the game
//! module reports through `trap_LocateGameData`. That is why the
//! array `g_entities[MAX_GENTITIES]` is addressed here as
//! `base + index * stride` with a *runtime* stride: the tail of
//! `gentity_t` (classname, think callbacks, …) is private to the mod
//! and its size depends on the build.
//!
//! Layout notes (ioquake3 `g_public.h`):
//! - `entityShared_t` starts with an `entityState_t` that is kept
//!   only for QVM compatibility and is otherwise unused. It shifts
//!   every later field of `r` by 208 bytes.
//! - Everything up to and including `r.ownerNum` is 4-byte fields
//!   only, so the layout is the same for 32-bit QVM code and native
//!   64-bit modules. Pointers only start at offset 516 (`client`).

use crate::entitystate::{EntityState, EntityType};
use crate::vector::Vec3;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

/// `svFlags`: entity is never sent to clients.
pub const SVF_NOCLIENT: i32 = 0x0000_0001;
/// `svFlags`: send only to the clients in `singleClient` (bitmask).
pub const SVF_CLIENTMASK: i32 = 0x0000_0002;
/// `svFlags`: the entity is a bot (set by the server on connect).
pub const SVF_BOT: i32 = 0x0000_0008;
/// `svFlags`: always sent, regardless of the PVS.
pub const SVF_BROADCAST: i32 = 0x0000_0020;
/// `eFlags`: the player is dead.
pub const EF_DEAD: i32 = 0x0000_0001;

/// Smallest plausible `sizeof(gentity_t)`: the head defined here.
pub const GENTITY_MIN_STRIDE: usize = 516;
/// Upper bound used when inferring the stride from scan hits.
pub const GENTITY_MAX_STRIDE: usize = 4096;

/// `entityShared_t` (308 bytes), the part of a server entity shared
/// between the server system and the game module.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct EntityShared {
    /// Vestigial `entityState_t`; expected to stay all-zero.
    pub unused: EntityState,
    /// Non-zero while the entity is linked into the world.
    pub linked: i32,
    /// Incremented every time the entity is (re)linked.
    pub link_count: i32,
    /// `SVF_*` bitmask.
    pub sv_flags: i32,
    /// Client bitmask for `SVF_CLIENTMASK` / client index for `SVF_SINGLECLIENT`.
    pub single_client: i32,
    /// Non-zero for brush models.
    pub bmodel: i32,
    /// Local bounding-box minimum.
    pub mins: Vec3,
    /// Local bounding-box maximum.
    pub maxs: Vec3,
    /// `CONTENTS_*` bitmask.
    pub contents: i32,
    /// World-space bounding-box minimum.
    pub abs_min: Vec3,
    /// World-space bounding-box maximum.
    pub abs_max: Vec3,
    /// Position used for collision and world linking.
    pub current_origin: Vec3,
    /// Orientation used for collision.
    pub current_angles: Vec3,
    /// Owner entity (missiles, …).
    pub owner_num: i32,
}

/// First 516 bytes of a `gentity_t`: `s` + `r`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SharedEntity {
    /// State networked to clients.
    pub s: EntityState,
    /// Server/game shared state.
    pub r: EntityShared,
}

const _: () = assert!(core::mem::size_of::<EntityShared>() == 308);
const _: () = assert!(core::mem::size_of::<SharedEntity>() == GENTITY_MIN_STRIDE);
const _: () = assert!(core::mem::offset_of!(SharedEntity, r) == 208);
const _: () = assert!(core::mem::offset_of!(EntityShared, linked) == 208);
const _: () = assert!(core::mem::offset_of!(EntityShared, sv_flags) == 216);
const _: () = assert!(core::mem::offset_of!(EntityShared, mins) == 228);
const _: () = assert!(core::mem::offset_of!(EntityShared, current_origin) == 280);
const _: () = assert!(core::mem::offset_of!(EntityShared, owner_num) == 304);

impl SharedEntity {
    /// `true` for player entities (`eType == ET_PLAYER`).
    #[inline]
    pub fn is_player(&self) -> bool {
        self.s.e_type == EntityType::PLAYER
    }

    /// `true` when the server flagged this entity as a bot.
    #[inline]
    pub fn is_bot(&self) -> bool {
        self.r.sv_flags & SVF_BOT != 0
    }

    /// `true` when the dead flag is set in `eFlags`.
    #[inline]
    pub fn is_dead(&self) -> bool {
        self.s.e_flags & EF_DEAD != 0
    }
}

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// Does `candidate` look like the *server-side* twin of `reference`,
/// a player entity taken from the client's snapshot?
///
/// Identity (`number`, `eType`, `clientNum`) must match exactly. The
/// position may differ by up to `tolerance` units because the client
/// snapshot lags the server by a frame or two. The shared block must
/// also be self-consistent: linked, sane bounding box, and
/// `r.currentOrigin` within `origin_tolerance` of `s.pos.trBase`. That
/// last check is what rejects the client-side copies of the same
/// entity (`cg_entities[]`, snapshot arrays), whose bytes at that
/// offset are something else.
pub fn is_server_twin(
    candidate: &SharedEntity,
    reference: &EntityState,
    tolerance: f32,
    origin_tolerance: f32,
) -> bool {
    let s = &candidate.s;
    if s.number != reference.number
        || s.e_type != reference.e_type
        || s.client_num != reference.client_num
    {
        return false;
    }
    let base = s.pos.tr_base;
    if !finite(base) || !finite(candidate.r.current_origin) {
        return false;
    }
    if base.sub(reference.pos.tr_base).length() > tolerance {
        return false;
    }
    if candidate.r.linked != 1 {
        return false;
    }
    let (mins, maxs) = (candidate.r.mins, candidate.r.maxs);
    if !finite(mins) || !finite(maxs) || maxs.z <= mins.z || maxs.z.abs() > 4096.0 {
        return false;
    }
    candidate.r.current_origin.sub(base).length() <= origin_tolerance
}

/// Result of [`infer_gentity_stride`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StrideGuess {
    /// `sizeof(gentity_t)` in bytes.
    pub stride: usize,
    /// Number of anchor pairs that agree on this stride.
    pub votes: usize,
}

/// Infer `sizeof(gentity_t)` from entities found at known indices.
///
/// `anchors` are `(entity number, address)` pairs. In `g_entities[]`
/// the entity numbered `n` lives at `base + n * stride`, so for any
/// two anchors `stride = Δaddress / Δnumber`. Every pair votes; the
/// winner must be a plausible size (multiple of 4, within
/// [`GENTITY_MIN_STRIDE`]..=[`GENTITY_MAX_STRIDE`]) and strictly ahead
/// of the runner-up. Returns `None` if fewer than two anchors agree
/// or the vote is ambiguous.
pub fn infer_gentity_stride(anchors: &[(i32, usize)]) -> Option<StrideGuess> {
    let mut votes: HashMap<usize, usize> = HashMap::new();
    for (i, &(na, aa)) in anchors.iter().enumerate() {
        for &(nb, ab) in &anchors[i + 1..] {
            if na == nb {
                continue;
            }
            let dn = i64::from(nb) - i64::from(na);
            let da = ab as i64 - aa as i64;
            if da % dn != 0 {
                continue;
            }
            let s = da / dn;
            if s <= 0 {
                continue;
            }
            let s = s as usize;
            if s % 4 != 0 || !(GENTITY_MIN_STRIDE..=GENTITY_MAX_STRIDE).contains(&s) {
                continue;
            }
            *votes.entry(s).or_insert(0) += 1;
        }
    }
    let mut ranked: Vec<(usize, usize)> = votes.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    match ranked.as_slice() {
        [] => None,
        [(s, v)] => Some(StrideGuess { stride: *s, votes: *v }),
        [(s, v), (_, v2), ..] if v > v2 => Some(StrideGuess { stride: *s, votes: *v }),
        _ => None,
    }
}

/// Address of `g_entities[0]` given one entity found at `address`
/// with index `number` and a known `stride`.
pub fn gentities_base_checked(address: usize, number: i32, stride: usize) -> Option<usize> {
    let n = usize::try_from(number).ok()?;
    address.checked_sub(n.checked_mul(stride)?)
}

/// Outcome of [`bots_only_verdict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalOnlyVerdict {
    /// At most one non-bot player (the host): safe to proceed.
    Ok {
        /// Linked player entities found.
        players: usize,
        /// Of those, how many carry `SVF_BOT`.
        bots: usize,
    },
    /// More than one player is not flagged as a bot. Holds their slots.
    OtherHumans(Vec<usize>),
}

/// Decide whether a server entity array describes a *local game
/// against bots only*.
///
/// `entities[i]` must be `g_entities[i]`. Only linked player entities
/// in the client slots (`0..MAX_CLIENTS`) are considered. The game
/// passes if at most one of them is **not** flagged `SVF_BOT` (the
/// host). The rule fails closed: if the `svFlags` offset were wrong,
/// bots would not show the flag, several "humans" would appear, and
/// the verdict would be [`LocalOnlyVerdict::OtherHumans`].
///
/// Limitation: a connected human who is a spectator is unlinked and
/// therefore not counted.
pub fn bots_only_verdict(entities: &[SharedEntity]) -> LocalOnlyVerdict {
    let mut players = 0usize;
    let mut bots = 0usize;
    let mut humans = Vec::new();
    for (i, e) in entities.iter().enumerate().take(crate::MAX_CLIENTS) {
        if !e.is_player() || e.r.linked == 0 || e.s.number as usize != i {
            continue;
        }
        players += 1;
        if e.is_bot() {
            bots += 1;
        } else {
            humans.push(i);
        }
    }
    if humans.len() > 1 {
        LocalOnlyVerdict::OtherHumans(humans)
    } else {
        LocalOnlyVerdict::Ok { players, bots }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(number: i32, pos: Vec3) -> SharedEntity {
        let mut e = SharedEntity::zeroed();
        e.s.number = number;
        e.s.e_type = EntityType::PLAYER;
        e.s.client_num = number;
        e.s.pos.tr_base = pos;
        e.r.linked = 1;
        e.r.mins = Vec3::new(-15.0, -15.0, -24.0);
        e.r.maxs = Vec3::new(15.0, 15.0, 32.0);
        e.r.current_origin = pos;
        e
    }

    #[test]
    fn stride_inferred_from_two_players() {
        let base = 0x0900_0000usize;
        let stride = 0x35C; // 860, plausible gentity_t size
        let anchors = [(0, base), (3, base + 3 * stride)];
        let g = infer_gentity_stride(&anchors).unwrap();
        assert_eq!(g.stride, stride);
        assert_eq!(gentities_base_checked(base + 3 * stride, 3, stride), Some(base));
    }

    #[test]
    fn stride_vote_rejects_a_spurious_anchor() {
        let base = 0x0900_0000usize;
        let stride = 0x35C;
        let anchors = [
            (0, base),
            (1, base + stride),
            (2, base + 2 * stride),
            (1, 0x0A12_3458), // stale copy elsewhere in memory
        ];
        let g = infer_gentity_stride(&anchors).unwrap();
        assert_eq!(g.stride, stride);
        assert!(g.votes >= 3);
    }

    #[test]
    fn single_anchor_gives_no_stride() {
        assert_eq!(infer_gentity_stride(&[(2, 0x1000_0000)]), None);
    }

    #[test]
    fn implausible_stride_is_rejected() {
        // 208 bytes apart (like a snapshot array), below sizeof(head).
        assert_eq!(infer_gentity_stride(&[(0, 0x1000), (1, 0x1000 + 208)]), None);
    }

    #[test]
    fn twin_matches_and_rejects_far_or_mismatched() {
        let r = player(2, Vec3::new(100.0, 200.0, 50.0));
        let cand = player(2, Vec3::new(110.0, 205.0, 50.0));
        assert!(is_server_twin(&cand, &r.s, 150.0, 64.0));

        let far = player(2, Vec3::new(1000.0, 0.0, 0.0));
        assert!(!is_server_twin(&far, &r.s, 150.0, 64.0));

        let other = player(3, Vec3::new(100.0, 200.0, 50.0));
        assert!(!is_server_twin(&other, &r.s, 150.0, 64.0));
    }

    #[test]
    fn twin_rejects_client_copy_with_inconsistent_shared_block() {
        let r = player(1, Vec3::new(0.0, 0.0, 0.0));
        let mut cand = player(1, Vec3::new(0.0, 0.0, 0.0));
        cand.r.current_origin = Vec3::new(5_000.0, 0.0, 0.0);
        assert!(!is_server_twin(&cand, &r.s, 150.0, 64.0));
        let mut unlinked = player(1, Vec3::ZERO);
        unlinked.r.linked = 0;
        assert!(!is_server_twin(&unlinked, &r.s, 150.0, 64.0));
    }

    #[test]
    fn bot_flag_reads_from_sv_flags() {
        let mut e = player(1, Vec3::ZERO);
        assert!(!e.is_bot());
        e.r.sv_flags |= SVF_BOT;
        assert!(e.is_bot());
    }

    #[test]
    fn verdict_accepts_host_plus_bots() {
        let mut ents = vec![SharedEntity::zeroed(); 4];
        for i in 0..3 {
            ents[i] = player(i as i32, Vec3::ZERO);
        }
        ents[1].r.sv_flags |= SVF_BOT;
        ents[2].r.sv_flags |= SVF_BOT;
        assert_eq!(
            bots_only_verdict(&ents),
            LocalOnlyVerdict::Ok { players: 3, bots: 2 }
        );
    }

    #[test]
    fn verdict_refuses_second_human() {
        let mut ents = vec![SharedEntity::zeroed(); 4];
        for i in 0..3 {
            ents[i] = player(i as i32, Vec3::ZERO);
        }
        ents[1].r.sv_flags |= SVF_BOT; // slot 2 is a second human
        assert_eq!(bots_only_verdict(&ents), LocalOnlyVerdict::OtherHumans(vec![0, 2]));
    }

    #[test]
    fn verdict_fails_closed_when_flags_are_unreadable() {
        let mut ents = vec![SharedEntity::zeroed(); 4];
        for i in 0..3 {
            ents[i] = player(i as i32, Vec3::ZERO); // no bot flag anywhere
        }
        assert!(matches!(bots_only_verdict(&ents), LocalOnlyVerdict::OtherHumans(_)));
    }

    #[test]
    fn verdict_ignores_unlinked_and_misnumbered_slots() {
        let mut ents = vec![SharedEntity::zeroed(); 3];
        ents[0] = player(0, Vec3::ZERO);
        ents[1] = player(1, Vec3::ZERO);
        ents[1].r.linked = 0;
        ents[2] = player(7, Vec3::ZERO); // number != index: not a real slot
        assert_eq!(
            bots_only_verdict(&ents),
            LocalOnlyVerdict::Ok { players: 1, bots: 0 }
        );
    }
}
