//! `qcheat aim-mouse` — aim assistance via synthetic mouse input.
//!
//! Every attempt at overwriting the render-side view-angle in memory
//! kept landing on downstream copies (snapshot history buffers, a
//! heap mirror, a transient memmove pointer) that don't feed the
//! renderer — see the session notes. Rather than keep hunting for the
//! one true address, this drives the camera the same way a human
//! does: it computes the angle delta to the closest player and
//! injects that as relative mouse movement via `SendInput`. The game
//! processes it through its own normal input path
//! (`CL_MouseEvent` -> `cl.viewangles` -> ... -> render), so it can't
//! miss the real state no matter where that lives in memory.

use anyhow::Result;
use clap::Args as ClapArgs;
use sdk::{EntityState, EntityType, Snapshot, MAX_ENTITIES_IN_SNAPSHOT};
use std::thread;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
};

use crate::util::{parse_hex, DEFAULT_PROCESS};

const CHUNK: usize = 4096;
const HEADER_SIZE: usize = core::mem::size_of::<sdk::SnapshotHeader>();
const DEFAULT_CENTER: usize = 0x07000000;
const DEFAULT_RANGE: usize = 0x02000000;

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    #[arg(long, value_parser = parse_hex)]
    pub center: Option<usize>,

    #[arg(long, value_parser = parse_hex)]
    pub range: Option<usize>,

    /// How often to recompute and send a mouse correction (ms).
    #[arg(long, default_value = "16")]
    pub interval_ms: u64,

    /// In-game `sensitivity` cvar.
    #[arg(long, default_value = "5.0")]
    pub sensitivity: f32,

    /// In-game `m_yaw` cvar (degrees per raw mouse count per
    /// sensitivity unit).
    #[arg(long, default_value = "0.022")]
    pub m_yaw: f32,

    /// In-game `m_pitch` cvar.
    #[arg(long, default_value = "0.022")]
    pub m_pitch: f32,

    /// Fraction of the remaining angle error corrected per tick.
    /// Lower = smoother/safer, higher = snappier but can overshoot
    /// with a miscalibrated sensitivity.
    #[arg(long, default_value = "0.15")]
    pub smooth: f32,

    /// Hard cap on raw mouse counts sent in a single tick, so a
    /// sudden big angle error (new target, teleport, ...) doesn't
    /// fling the view.
    #[arg(long, default_value = "60")]
    pub max_delta: i32,
}

pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;

    let center = args.center.unwrap_or(DEFAULT_CENTER);
    let range = args.range.unwrap_or(DEFAULT_RANGE);
    let start = center.saturating_sub(range);
    let end = center.saturating_add(range);

    println!(
        "Aim-mouse active on {} (pid {}). sensitivity={} m_yaw={} m_pitch={} smooth={}\n",
        proc.name, proc.pid, args.sensitivity, args.m_yaw, args.m_pitch, args.smooth
    );

    // degrees produced by one raw mouse count at this sensitivity
    let deg_per_count_yaw = args.sensitivity * args.m_yaw;
    let deg_per_count_pitch = args.sensitivity * args.m_pitch;

    // Locating cg.activeSnapshots means scanning tens of MiB of memory
    // — far too slow to redo every tick if we want to keep up with a
    // 60Hz+ control loop. Find it once, cache the (up to) two
    // candidate addresses, and on every tick just re-read those fixed
    // spots directly (two cheap typed reads) to pick whichever is
    // newer by serverTime. Only fall back to a full rescan if both
    // cached addresses stop reading as valid snapshots.
    let mut cached_addrs: Vec<usize> = locate_snapshot_addrs(&handle, start, end);
    if cached_addrs.is_empty() {
        println!("No snapshot found yet — will keep scanning until one appears.");
    } else {
        println!("Snapshot locked at {:X?} — polling directly from now on.", cached_addrs);
    }

    loop {
        if cached_addrs.is_empty()
            || !cached_addrs
                .iter()
                .any(|&a| handle.read::<sdk::SnapshotHeader>(a).is_ok())
        {
            cached_addrs = locate_snapshot_addrs(&handle, start, end);
            if cached_addrs.is_empty() {
                thread::sleep(Duration::from_millis(args.interval_ms));
                continue;
            }
            println!("Re-acquired snapshot at {:X?}.", cached_addrs);
        }

        let snap_addr = cached_addrs
            .iter()
            .copied()
            .max_by_key(|&a| {
                handle
                    .read::<sdk::SnapshotHeader>(a)
                    .map(|h| h.server_time)
                    .unwrap_or(i32::MIN)
            })
            .unwrap();

        let snap = match handle.read::<Snapshot>(snap_addr) {
            Ok(s) => Box::new(s),
            Err(_) => {
                cached_addrs.clear();
                thread::sleep(Duration::from_millis(args.interval_ms));
                continue;
            }
        };

        let local_pos = snap.header.ps.origin;
        let current_yaw = snap.header.ps.viewangles.y;
        let current_pitch = snap.header.ps.viewangles.x;

        let mut closest: Option<(&EntityState, f32)> = None;
        for es in &snap.entities[..snap.header.num_entities.min(MAX_ENTITIES_IN_SNAPSHOT as i32) as usize]
        {
            if es.e_type != EntityType::PLAYER {
                continue;
            }
            let target_pos = es.pos.tr_base;
            let dx = target_pos.x - local_pos.x;
            let dy = target_pos.y - local_pos.y;
            let dz = target_pos.z - local_pos.z;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            match &closest {
                None => closest = Some((es, dist)),
                Some((_, d)) if dist < *d => closest = Some((es, dist)),
                _ => {}
            }
        }

        if let Some((target, dist)) = closest {
            let (target_yaw, target_pitch) = calculate_angles(local_pos, target.pos.tr_base);

            let err_yaw = normalize_angle(target_yaw - current_yaw);
            let err_pitch = normalize_angle(target_pitch - current_pitch);

            let move_yaw = err_yaw * args.smooth;
            let move_pitch = err_pitch * args.smooth;

            // cl_input.c: `cl.viewangles[YAW] -= m_yaw * mx` — moving
            // the mouse right (positive mx) DECREASES yaw, so invert
            // here to turn toward positive err_yaw.
            let mut dx = -(move_yaw / deg_per_count_yaw).round() as i32;
            // cl_input.c: `cl.viewangles[PITCH] += m_pitch * my` —
            // moving the mouse down (positive my) increases pitch
            // (looks down), which matches our positive-pitch-is-down
            // convention directly, no inversion needed.
            let mut dy = (move_pitch / deg_per_count_pitch).round() as i32;

            dx = dx.clamp(-args.max_delta, args.max_delta);
            dy = dy.clamp(-args.max_delta, args.max_delta);

            if dx != 0 || dy != 0 {
                send_mouse_delta(dx, dy);
            }

            println!(
                "Target: client {}, dist {:.1}m, err (yaw {:.1} pitch {:.1}) -> mouse ({dx}, {dy})",
                target.client_num, dist, err_yaw, err_pitch
            );
        } else {
            println!("No players found.");
        }

        thread::sleep(Duration::from_millis(args.interval_ms));
    }
}

fn normalize_angle(mut diff: f32) -> f32 {
    while diff > 180.0 {
        diff -= 360.0;
    }
    while diff < -180.0 {
        diff += 360.0;
    }
    diff
}

fn send_mouse_delta(dx: i32, dy: i32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], core::mem::size_of::<INPUT>() as i32);
    }
}

fn calculate_angles(from: sdk::Vec3, to: sdk::Vec3) -> (f32, f32) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let dz = to.z - from.z;

    let yaw = dy.atan2(dx).to_degrees();
    let horiz_dist = (dx * dx + dy * dy).sqrt();
    let pitch = (-dz).atan2(horiz_dist).to_degrees();

    (yaw, pitch)
}

/// Full scan for `cg.activeSnapshots`, returning the confirmed pair
/// (or a single fallback candidate). Expensive — call once, then poll
/// the returned addresses directly instead of rescanning every tick.
fn locate_snapshot_addrs(
    handle: &process::ProcessHandle,
    start: usize,
    end: usize,
) -> Vec<usize> {
    let hits = find_snapshot_candidates(handle, start, end);
    if hits.is_empty() {
        return Vec::new();
    }
    const SS_SIZE: usize = core::mem::size_of::<Snapshot>();
    match hits.windows(2).find(|w| w[1] - w[0] == SS_SIZE) {
        Some(w) => vec![w[0], w[1]],
        None => vec![hits[0]],
    }
}

fn find_snapshot_candidates(
    handle: &process::ProcessHandle,
    start: usize,
    end: usize,
) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut cursor = start;

    while cursor < end {
        let buf = match handle.read::<Chunk>(cursor) {
            Ok(b) => b,
            Err(_) => {
                cursor = cursor.saturating_add(CHUNK);
                continue;
            }
        };

        let mut off = 0usize;
        while off + HEADER_SIZE <= buf.0.len() {
            let header: &sdk::SnapshotHeader =
                bytemuck::from_bytes(&buf.0[off..off + HEADER_SIZE]);
            if looks_like_snapshot(header) {
                hits.push(cursor + off);
                off += HEADER_SIZE;
            } else {
                off += 4;
            }
        }
        cursor = cursor.saturating_add(CHUNK);
    }
    hits
}

fn looks_like_snapshot(h: &sdk::SnapshotHeader) -> bool {
    if !(1..=256).contains(&h.num_entities) {
        return false;
    }
    if h.server_time < 1_000 {
        return false;
    }
    if !(0..=2_000).contains(&h.ping) {
        return false;
    }
    if !(0..64).contains(&h.ps.client_num) {
        return false;
    }
    if !(0..=15).contains(&h.ps.weapon) {
        return false;
    }
    if !(0..=8).contains(&h.ps.pm_type) {
        return false;
    }

    let o = h.ps.origin;
    if !o.x.is_finite() || !o.y.is_finite() || !o.z.is_finite() {
        return false;
    }
    if o.x.abs() >= 32_768.0 || o.y.abs() >= 32_768.0 || o.z.abs() >= 32_768.0 {
        return false;
    }

    let hp = h.ps.stats[0];
    if !(-50..=1_000).contains(&hp) {
        return false;
    }

    true
}

#[repr(C)]
#[derive(Copy, Clone)]
struct Chunk([u8; CHUNK + HEADER_SIZE]);

unsafe impl bytemuck::Zeroable for Chunk {}
unsafe impl bytemuck::Pod for Chunk {}
