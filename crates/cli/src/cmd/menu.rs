//! `qcheat menu` — unified aimbot + wallhack with an in-game settings menu.
//!
//! Runs the aim-mouse and wallhack logic together off a single shared
//! snapshot read (instead of two separate processes each rescanning
//! memory), and adds a keyboard-driven overlay menu (F1 by default to
//! open/close) to flip settings live instead of restarting with new
//! CLI flags every time.
//!
//! Navigation is keyboard-only, not mouse: the game actively captures
//! and hides the system cursor for mouselook, so `GetCursorPos` isn't
//! a reliable "where is the user pointing" source while playing.
//! UP/DOWN selects a menu row, LEFT/RIGHT adjusts it.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use sdk::{EntityType, Snapshot, Vec3, MAX_ENTITIES_IN_SNAPSHOT};
use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreatePen, CreateSolidBrush, DeleteObject, GetDC, ReleaseDC, Rectangle,
    SelectObject, SetBkMode, SetTextColor, TextOutW, HGDIOBJ, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetWindowLongPtrW,
    GetWindowThreadProcessId, PeekMessageW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    CS_HREDRAW, CS_VREDRAW, GWL_EXSTYLE, HWND_TOPMOST, LWA_COLORKEY, MSG, PM_REMOVE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WM_DESTROY, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::util::{parse_hex, DEFAULT_PROCESS};

const CHUNK: usize = 4096;
const HEADER_SIZE: usize = core::mem::size_of::<sdk::SnapshotHeader>();
const DEFAULT_CENTER: usize = 0x07000000;
const DEFAULT_RANGE: usize = 0x02000000;
/// bg_public.h: entityState_t.eFlags bit set on dead players.
const EF_DEAD: i32 = 0x0000_0001;
const PLAYER_HEIGHT: f32 = 40.0;

const COLOR_KEY: u32 = 0x0000_0000; // transparent color key (never draw with it)
const BOX_COLOR: u32 = 0x0000_00FF; // COLORREF is 0x00BBGGRR -> red
const TEXT_COLOR: u32 = 0x0000_FFFF; // yellow
const MENU_BG: u32 = 0x0020_2020; // dark gray, never equals COLOR_KEY
const MENU_BORDER: u32 = 0x00FF_FFFF; // white
const MENU_NORMAL: u32 = 0x00FF_FFFF; // white
const MENU_SELECTED: u32 = 0x0000_FF00; // green

const VK_LEFT: i32 = 0x25;
const VK_UP: i32 = 0x26;
const VK_RIGHT: i32 = 0x27;
const VK_DOWN: i32 = 0x28;

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    #[arg(long, value_parser = parse_hex)]
    pub center: Option<usize>,

    #[arg(long, value_parser = parse_hex)]
    pub range: Option<usize>,

    /// Loop tick rate (ms). 8 = ~120Hz.
    #[arg(long, default_value = "8")]
    pub interval_ms: u64,

    /// In-game `m_yaw` cvar — calibration constant, not menu-editable.
    #[arg(long, default_value = "0.022")]
    pub m_yaw: f32,

    /// In-game `m_pitch` cvar — calibration constant, not menu-editable.
    #[arg(long, default_value = "0.022")]
    pub m_pitch: f32,

    /// Virtual-key code that opens/closes the menu (default: F1,
    /// 0x70). See Microsoft's Virtual-Key Codes docs for other
    /// values, e.g. 0x24 for Home, 0x2D for Insert.
    #[arg(long, value_parser = parse_hex, default_value = "0x70")]
    pub menu_key: usize,

    /// Virtual-key code that quick-toggles the aimbot on/off, even
    /// with the menu closed (default: Insert, 0x2D).
    #[arg(long, value_parser = parse_hex, default_value = "0x2D")]
    pub aimbot_key: usize,
}

/// Everything the menu can change live. Single-threaded — the menu,
/// the aimbot correction, and the wallhack draw all run in one loop,
/// so there's no locking to worry about.
#[derive(Clone, Copy)]
struct Settings {
    aimbot_enabled: bool,
    sensitivity: f32,
    smooth: f32,
    max_delta: f32,
    esp_enabled: bool,
    fov: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            aimbot_enabled: true,
            sensitivity: 5.0,
            smooth: 0.15,
            max_delta: 60.0,
            esp_enabled: true,
            fov: 90.0,
        }
    }
}

struct ToggleSpec {
    label: &'static str,
    get: fn(&Settings) -> bool,
    set: fn(&mut Settings, bool),
}

struct SliderSpec {
    label: &'static str,
    min: f32,
    max: f32,
    step: f32,
    get: fn(&Settings) -> f32,
    set: fn(&mut Settings, f32),
}

enum MenuItem {
    Toggle(ToggleSpec),
    Slider(SliderSpec),
}

fn menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::Toggle(ToggleSpec {
            label: "Aimbot",
            get: |s| s.aimbot_enabled,
            set: |s, v| s.aimbot_enabled = v,
        }),
        MenuItem::Slider(SliderSpec {
            label: "Sensitivity",
            min: 0.1,
            max: 20.0,
            step: 0.05,
            get: |s| s.sensitivity,
            set: |s, v| s.sensitivity = v,
        }),
        MenuItem::Slider(SliderSpec {
            label: "Smooth",
            min: 0.01,
            max: 1.0,
            step: 0.005,
            get: |s| s.smooth,
            set: |s, v| s.smooth = v,
        }),
        MenuItem::Slider(SliderSpec {
            label: "Max Delta",
            min: 5.0,
            max: 300.0,
            step: 1.0,
            get: |s| s.max_delta,
            set: |s, v| s.max_delta = v,
        }),
        MenuItem::Toggle(ToggleSpec {
            label: "Wallhack",
            get: |s| s.esp_enabled,
            set: |s, v| s.esp_enabled = v,
        }),
        MenuItem::Slider(SliderSpec {
            label: "FOV",
            min: 60.0,
            max: 140.0,
            step: 0.5,
            get: |s| s.fov,
            set: |s, v| s.fov = v,
        }),
    ]
}

/// Last real snapshot position/velocity for one tracked enemy, used to
/// extrapolate its position between real snapshots (sv_fps, ~20-40Hz)
/// for both smooth wallhack boxes and a live aim target.
struct Track {
    pos: Vec3,
    vel: Vec3,
    wall: Instant,
}

pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;

    let center = args.center.unwrap_or(DEFAULT_CENTER);
    let range = args.range.unwrap_or(DEFAULT_RANGE);
    let start = center.saturating_sub(range);
    let end = center.saturating_add(range);

    let game_hwnd = find_window_for_pid(proc.pid)
        .with_context(|| format!("no visible window found for pid {}", proc.pid))?;

    let (mut top_left, mut width, mut height) = game_window_rect(game_hwnd)?;

    let overlay = create_overlay_window(top_left.x, top_left.y, width, height)?;

    println!(
        "qcheat menu on {} (pid {}). vkey 0x{:02X}: open/close menu, vkey 0x{:02X}: quick aimbot toggle.",
        proc.name, proc.pid, args.menu_key, args.aimbot_key
    );
    println!("In the menu: UP/DOWN select, LEFT/RIGHT adjust. Ctrl+C here to quit.\n");

    let mut settings = Settings::default();
    let items = menu_items();
    let mut selected: usize = 0;
    let mut menu_open = false;

    let mut cached_addrs: Vec<usize> = locate_snapshot_addrs(&handle, start, end);
    let mut last_server_time = i32::MIN;
    let mut tracks: HashMap<i32, Track> = HashMap::new();
    let mut own_yaw: Option<f32> = None;
    let mut own_pitch: Option<f32> = None;
    let mut aim_local_pos = Vec3::ZERO;

    let mut home_was_down = false;
    let mut insert_was_down = false;
    let mut up_was_down = false;
    let mut down_was_down = false;
    let mut left_was_down = false;
    let mut right_was_down = false;

    let mut tick: u64 = 0;
    // Launching before actually being in a match can (a) capture the
    // wrong window size/position if the game hasn't reached its final
    // resolution yet, and (b) latch onto a memory block that merely
    // *looks* like a snapshot without being the real one, since our
    // validity check only asks "is this still structurally
    // plausible", not "has this actually changed". Periodically
    // re-fetching the window rect and forcing a fresh snapshot search
    // makes the tool self-heal once a real match actually starts,
    // regardless of when it was launched.
    const WINDOW_RESYNC_EVERY: u64 = 120; // ~1s at 8ms/tick
    const FORCE_RESCAN_EVERY: u64 = 600; // ~5s at 8ms/tick

    loop {
        tick += 1;
        pump_messages();

        // Windows periodically demotes a topmost layered window (game
        // regains focus, alt-tab, ...) — re-assert every tick.
        unsafe {
            let _ = SetWindowPos(
                overlay,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
        }

        if tick % WINDOW_RESYNC_EVERY == 0 {
            if let Ok((new_top_left, new_w, new_h)) = game_window_rect(game_hwnd) {
                if new_top_left.x != top_left.x
                    || new_top_left.y != top_left.y
                    || new_w != width
                    || new_h != height
                {
                    top_left = new_top_left;
                    width = new_w;
                    height = new_h;
                    unsafe {
                        let _ = SetWindowPos(
                            overlay,
                            HWND_TOPMOST,
                            top_left.x,
                            top_left.y,
                            width,
                            height,
                            SWP_NOACTIVATE,
                        );
                    }
                }
            }
        }

        // --- hotkeys ---
        let home_down = key_down(args.menu_key as i32);
        if home_down && !home_was_down {
            menu_open = !menu_open;
            // Block click-through while the menu is up so a stray
            // click doesn't also fire a weapon in-game; restore it on
            // close so gameplay is unaffected.
            set_click_through(overlay, !menu_open);
        }
        home_was_down = home_down;

        let insert_down = key_down(args.aimbot_key as i32);
        if insert_down && !insert_was_down {
            settings.aimbot_enabled = !settings.aimbot_enabled;
        }
        insert_was_down = insert_down;

        if menu_open {
            let up = key_down(VK_UP);
            if up && !up_was_down {
                selected = if selected == 0 { items.len() - 1 } else { selected - 1 };
            }
            up_was_down = up;

            let down = key_down(VK_DOWN);
            if down && !down_was_down {
                selected = (selected + 1) % items.len();
            }
            down_was_down = down;

            let left = key_down(VK_LEFT);
            let right = key_down(VK_RIGHT);
            match &items[selected] {
                MenuItem::Toggle(t) => {
                    if (left && !left_was_down) || (right && !right_was_down) {
                        let cur = (t.get)(&settings);
                        (t.set)(&mut settings, !cur);
                    }
                }
                MenuItem::Slider(s) => {
                    if left {
                        let v = ((s.get)(&settings) - s.step).max(s.min);
                        (s.set)(&mut settings, v);
                    }
                    if right {
                        let v = ((s.get)(&settings) + s.step).min(s.max);
                        (s.set)(&mut settings, v);
                    }
                }
            }
            left_was_down = left;
            right_was_down = right;
        }

        // --- snapshot cache (see aim_mouse.rs / esp.rs for why) ---
        let cache_ok = !cached_addrs.is_empty()
            && cached_addrs.iter().any(|&a| {
                handle
                    .read::<sdk::SnapshotHeader>(a)
                    .map(|h| looks_like_snapshot(&h))
                    .unwrap_or(false)
            });
        if !cache_ok || tick % FORCE_RESCAN_EVERY == 0 {
            let fresh = locate_snapshot_addrs(&handle, start, end);
            if fresh.is_empty() {
                if !cache_ok {
                    // Genuinely stale (or nothing found yet, e.g. not
                    // in a match) and the periodic sweep found nothing
                    // new either — try again next tick.
                    cached_addrs.clear();
                    thread::sleep(Duration::from_millis(args.interval_ms));
                    continue;
                }
                // Just the periodic sweep coming up empty (scan timing
                // glitch); the existing cache still looks valid.
            } else if fresh != cached_addrs {
                cached_addrs = fresh;
            }
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

        // Wallhack's own eye/angles: cg.predictedPlayerState updates every
        // render frame via client-side prediction, so it's safe (and
        // proven, in esp.rs) to read fresh every tick, no gating.
        let esp_eye = snap.header.ps.origin;
        let (esp_forward, esp_right, esp_up) =
            angle_vectors(snap.header.ps.viewangles.y, snap.header.ps.viewangles.x);

        if snap.header.server_time != last_server_time {
            let dt_server = (snap.header.server_time - last_server_time) as f32 / 1000.0;
            aim_local_pos = snap.header.ps.origin;
            own_yaw = Some(snap.header.ps.viewangles.y);
            own_pitch = Some(snap.header.ps.viewangles.x);

            let mut seen = HashSet::new();
            for es in &snap.entities
                [..snap.header.num_entities.min(MAX_ENTITIES_IN_SNAPSHOT as i32) as usize]
            {
                if es.e_type != EntityType::PLAYER {
                    continue;
                }
                if es.client_num == snap.header.ps.client_num {
                    continue;
                }
                if es.e_flags & EF_DEAD != 0 {
                    continue;
                }
                seen.insert(es.client_num);
                let new_pos = es.pos.tr_base;
                let vel = match tracks.get(&es.client_num) {
                    Some(prev) if last_server_time != i32::MIN && dt_server > 0.001 => Vec3::new(
                        (new_pos.x - prev.pos.x) / dt_server,
                        (new_pos.y - prev.pos.y) / dt_server,
                        (new_pos.z - prev.pos.z) / dt_server,
                    ),
                    _ => Vec3::ZERO,
                };
                tracks.insert(
                    es.client_num,
                    Track {
                        pos: new_pos,
                        vel,
                        wall: Instant::now(),
                    },
                );
            }
            tracks.retain(|client_num, _| seen.contains(client_num));
            last_server_time = snap.header.server_time;
        }

        let extrapolated: Vec<(i32, Vec3)> = tracks
            .iter()
            .map(|(&client_num, t)| {
                let elapsed = t.wall.elapsed().as_secs_f32();
                (
                    client_num,
                    Vec3::new(
                        t.pos.x + t.vel.x * elapsed,
                        t.pos.y + t.vel.y * elapsed,
                        t.pos.z + t.vel.z * elapsed,
                    ),
                )
            })
            .collect();

        // --- aimbot ---
        if settings.aimbot_enabled {
            if let (Some(oy), Some(op)) = (own_yaw, own_pitch) {
                let closest = extrapolated.iter().min_by(|a, b| {
                    dist_sq(aim_local_pos, a.1)
                        .partial_cmp(&dist_sq(aim_local_pos, b.1))
                        .unwrap()
                });

                if let Some(&(_, target_pos)) = closest {
                    let (target_yaw, target_pitch) = calculate_angles(aim_local_pos, target_pos);
                    let err_yaw = normalize_angle(target_yaw - oy);
                    let err_pitch = normalize_angle(target_pitch - op);

                    let move_yaw = err_yaw * settings.smooth;
                    let move_pitch = err_pitch * settings.smooth;

                    let deg_per_count_yaw = settings.sensitivity * args.m_yaw;
                    let deg_per_count_pitch = settings.sensitivity * args.m_pitch;

                    // cl_input.c: `cl.viewangles[YAW] -= m_yaw * mx`.
                    let mut dx = -(move_yaw / deg_per_count_yaw).round() as i32;
                    // cl_input.c: `cl.viewangles[PITCH] += m_pitch * my`.
                    let mut dy = (move_pitch / deg_per_count_pitch).round() as i32;

                    let max_delta = settings.max_delta as i32;
                    dx = dx.clamp(-max_delta, max_delta);
                    dy = dy.clamp(-max_delta, max_delta);

                    if dx != 0 || dy != 0 {
                        send_mouse_delta(dx, dy);
                    }

                    let applied_yaw = -deg_per_count_yaw * dx as f32;
                    let applied_pitch = deg_per_count_pitch * dy as f32;
                    own_yaw = Some(oy + applied_yaw);
                    own_pitch = Some((op + applied_pitch).clamp(-90.0, 90.0));
                }
            }
        }

        // --- wallhack ---
        let mut boxes: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
        if settings.esp_enabled {
            for &(client_num, pos) in &extrapolated {
                if let Some(b) = project_player_box(
                    pos,
                    client_num,
                    esp_eye,
                    esp_forward,
                    esp_right,
                    esp_up,
                    width as f32,
                    height as f32,
                    settings.fov,
                ) {
                    boxes.push(b);
                }
            }
        }

        draw_frame(
            overlay,
            width,
            height,
            &boxes,
            menu_open,
            &items,
            selected,
            &settings,
            args.menu_key,
        );

        thread::sleep(Duration::from_millis(args.interval_ms));
    }
}

fn dist_sq(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    dx * dx + dy * dy + dz * dz
}

fn key_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
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

fn calculate_angles(from: Vec3, to: Vec3) -> (f32, f32) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let dz = to.z - from.z;

    let yaw = dy.atan2(dx).to_degrees();
    let horiz_dist = (dx * dx + dy * dy).sqrt();
    let pitch = (-dz).atan2(horiz_dist).to_degrees();

    (yaw, pitch)
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

/// Quake's `AngleVectors` with roll assumed 0.
fn angle_vectors(yaw_deg: f32, pitch_deg: f32) -> (Vec3, Vec3, Vec3) {
    let (sy, cy) = yaw_deg.to_radians().sin_cos();
    let (sp, cp) = pitch_deg.to_radians().sin_cos();

    let forward = Vec3::new(cp * cy, cp * sy, -sp);
    let right = Vec3::new(sy, -cy, 0.0);
    let up = Vec3::new(sp * cy, sp * sy, cp);
    (forward, right, up)
}

fn world_to_screen(
    world: Vec3,
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    screen_w: f32,
    screen_h: f32,
    fov_deg: f32,
) -> Option<(f32, f32)> {
    let d = Vec3::new(world.x - eye.x, world.y - eye.y, world.z - eye.z);

    let cz = d.x * forward.x + d.y * forward.y + d.z * forward.z;
    if cz < 1.0 {
        return None;
    }
    let cx = d.x * right.x + d.y * right.y + d.z * right.z;
    let cy = d.x * up.x + d.y * up.y + d.z * up.z;

    let scale = (screen_w * 0.5) / (fov_deg.to_radians() * 0.5).tan();
    let sx = screen_w * 0.5 + cx * scale / cz;
    let sy = screen_h * 0.5 - cy * scale / cz;
    Some((sx, sy))
}

fn project_player_box(
    feet: Vec3,
    client_num: i32,
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    screen_w: f32,
    screen_h: f32,
    fov_deg: f32,
) -> Option<(f32, f32, f32, f32, i32)> {
    let head = Vec3::new(feet.x, feet.y, feet.z + PLAYER_HEIGHT);

    let (fx, fy) = world_to_screen(feet, eye, forward, right, up, screen_w, screen_h, fov_deg)?;
    let (hx, hy) = world_to_screen(head, eye, forward, right, up, screen_w, screen_h, fov_deg)?;

    let box_h = (fy - hy).abs().max(4.0);
    let box_w = box_h * 0.5;
    let cx = (fx + hx) * 0.5;
    let top = hy.min(fy);
    let bottom = hy.max(fy);

    Some((cx - box_w * 0.5, top, cx + box_w * 0.5, bottom, client_num))
}

fn locate_snapshot_addrs(handle: &process::ProcessHandle, start: usize, end: usize) -> Vec<usize> {
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

// ---------------------------------------------------------------------
// Overlay window plumbing
// ---------------------------------------------------------------------

/// Screen-space top-left + client size of `hwnd`. Re-queried
/// periodically (not just once at startup) because a game window
/// caught mid-loading-screen or before its final resolution is set
/// can resize/move once the player actually joins a match.
fn game_window_rect(
    hwnd: HWND,
) -> Result<(windows::Win32::Foundation::POINT, i32, i32)> {
    let mut client_rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client_rect) }?;
    let mut top_left = windows::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { ClientToScreen(hwnd, &mut top_left) }.ok()?;
    let width = (client_rect.right - client_rect.left).max(1);
    let height = (client_rect.bottom - client_rect.top).max(1);
    Ok((top_left, width, height))
}

fn find_window_for_pid(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn enum_proc(
        hwnd: HWND,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut window_pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        if window_pid == search.pid
            && windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool()
            && windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(hwnd) > 0
        {
            search.found = Some(hwnd);
            return windows::Win32::Foundation::BOOL(0);
        }
        windows::Win32::Foundation::BOOL(1)
    }

    let mut search = Search { pid, found: None };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::EnumWindows(
            Some(enum_proc),
            LPARAM(&mut search as *mut Search as isize),
        );
    }
    search.found
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn create_overlay_window(x: i32, y: i32, width: i32, height: i32) -> Result<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let hinstance = windows::Win32::Foundation::HINSTANCE::from(hinstance);
        let class_name = w("QCheatMenuOverlay");

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(w("qcheat menu").as_ptr()),
            WS_POPUP,
            x,
            y,
            width,
            height,
            None,
            None,
            hinstance,
            None,
        )?;

        SetLayeredWindowAttributes(hwnd, COLORREF(COLOR_KEY), 255, LWA_COLORKEY)?;
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        Ok(hwnd)
    }
}

fn set_click_through(hwnd: HWND, click_through: bool) {
    unsafe {
        let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if click_through {
            ex_style |= WS_EX_TRANSPARENT.0 as u32;
        } else {
            ex_style &= !(WS_EX_TRANSPARENT.0 as u32);
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
    }
}

fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn draw_frame(
    hwnd: HWND,
    width: i32,
    height: i32,
    boxes: &[(f32, f32, f32, f32, i32)],
    menu_open: bool,
    items: &[MenuItem],
    selected: usize,
    settings: &Settings,
    menu_key: usize,
) {
    unsafe {
        let hdc = GetDC(hwnd);
        if hdc.is_invalid() {
            return;
        }

        let bg_brush = CreateSolidBrush(COLORREF(COLOR_KEY));
        let bg_pen = CreatePen(PS_SOLID, 1, COLORREF(COLOR_KEY));
        let old_brush = SelectObject(hdc, HGDIOBJ::from(bg_brush));
        let old_pen = SelectObject(hdc, HGDIOBJ::from(bg_pen));
        let _ = Rectangle(hdc, 0, 0, width, height);

        SetBkMode(hdc, TRANSPARENT);

        if !boxes.is_empty() {
            let box_pen = CreatePen(PS_SOLID, 2, COLORREF(BOX_COLOR));
            SelectObject(hdc, HGDIOBJ::from(box_pen));
            SetTextColor(hdc, COLORREF(TEXT_COLOR));
            for &(l, t, r, b, client_num) in boxes {
                let _ = Rectangle(hdc, l as i32, t as i32, r as i32, b as i32);
                let label = w(&format!("#{client_num}"));
                let _ = TextOutW(hdc, l as i32, (t as i32) - 16, &label[..label.len() - 1]);
            }
            let _ = DeleteObject(HGDIOBJ::from(box_pen));
        }

        if menu_open {
            let panel_x = 20;
            let panel_y = 20;
            let line_h = 20;
            let panel_w = 300;
            let panel_h = (items.len() as i32) * line_h + 40;

            let panel_brush = CreateSolidBrush(COLORREF(MENU_BG));
            let panel_pen = CreatePen(PS_SOLID, 1, COLORREF(MENU_BORDER));
            SelectObject(hdc, HGDIOBJ::from(panel_brush));
            SelectObject(hdc, HGDIOBJ::from(panel_pen));
            let _ = Rectangle(hdc, panel_x, panel_y, panel_x + panel_w, panel_y + panel_h);

            SetTextColor(hdc, COLORREF(MENU_NORMAL));
            let title = w(&format!(
                "qcheat -- {} close, UP/DOWN, LEFT/RIGHT",
                vk_name(menu_key)
            ));
            let _ = TextOutW(hdc, panel_x + 8, panel_y + 6, &title[..title.len() - 1]);

            for (i, item) in items.iter().enumerate() {
                let y = panel_y + 26 + (i as i32) * line_h;
                let marker = if i == selected { ">" } else { " " };
                let text = match item {
                    MenuItem::Toggle(t) => {
                        format!("{marker} {}: {}", t.label, if (t.get)(settings) { "ON" } else { "OFF" })
                    }
                    MenuItem::Slider(s) => format!("{marker} {}: {:.2}", s.label, (s.get)(settings)),
                };
                SetTextColor(
                    hdc,
                    COLORREF(if i == selected { MENU_SELECTED } else { MENU_NORMAL }),
                );
                let wtext = w(&text);
                let _ = TextOutW(hdc, panel_x + 10, y, &wtext[..wtext.len() - 1]);
            }

            let _ = DeleteObject(HGDIOBJ::from(panel_brush));
            let _ = DeleteObject(HGDIOBJ::from(panel_pen));
        }

        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ::from(bg_brush));
        let _ = DeleteObject(HGDIOBJ::from(bg_pen));
        ReleaseDC(hwnd, hdc);
    }
}

/// Human-readable name for the handful of virtual-key codes anyone
/// would realistically bind the menu to; falls back to the raw hex
/// code for anything else.
fn vk_name(vk: usize) -> String {
    match vk {
        0x70 => "F1".to_string(),
        0x71 => "F2".to_string(),
        0x72 => "F3".to_string(),
        0x73 => "F4".to_string(),
        0x24 => "HOME".to_string(),
        0x23 => "END".to_string(),
        0x2D => "INSERT".to_string(),
        0x2E => "DELETE".to_string(),
        _ => format!("0x{vk:02X}"),
    }
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
