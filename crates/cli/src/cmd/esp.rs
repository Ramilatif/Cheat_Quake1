//! `qcheat esp` — wallhack via a transparent screen overlay.
//!
//! Reads every entity from the snapshot regardless of line of sight
//! (there's no occlusion check in the data itself — that's purely a
//! rendering-time decision the game makes), projects each living
//! enemy's world position into 2D screen space using the local
//! player's own view angles, and draws a box over them in a
//! click-through layered window sitting on top of the game. No code
//! injection, no engine patch — same external-read architecture as
//! the rest of this toolkit.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use sdk::{EntityType, Snapshot, Vec3, MAX_ENTITIES_IN_SNAPSHOT};
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreatePen, CreateSolidBrush, DeleteObject, GetDC, ReleaseDC, Rectangle,
    SelectObject, SetBkMode, SetTextColor, TextOutW, HGDIOBJ, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetWindowThreadProcessId,
    PeekMessageW, PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, LWA_COLORKEY, MSG,
    PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WM_DESTROY, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::util::{parse_hex, DEFAULT_PROCESS};

const CHUNK: usize = 4096;
const HEADER_SIZE: usize = core::mem::size_of::<sdk::SnapshotHeader>();
const DEFAULT_CENTER: usize = 0x07000000;
const DEFAULT_RANGE: usize = 0x02000000;
/// bg_public.h: entityState_t.eFlags bit set on dead players.
const EF_DEAD: i32 = 0x0000_0001;
/// Rough standing player height (bbox maxs.z), in world units, used
/// to turn a single origin point into a head-to-feet screen box.
const PLAYER_HEIGHT: f32 = 40.0;
/// Black is the overlay's transparent color key — never draw with it.
const COLOR_KEY: u32 = 0x00000000;
const BOX_COLOR: u32 = 0x0000FF; // BGR: red
const TEXT_COLOR: u32 = 0x00FFFF; // BGR: yellow

/// Last real snapshot position/velocity for one tracked enemy, used to
/// extrapolate its position between real snapshots (which only arrive
/// at sv_fps, ~20-40Hz) so the box moves smoothly at the overlay's own
/// refresh rate instead of stair-stepping.
struct Track {
    pos: Vec3,
    vel: Vec3,
    wall: Instant,
}

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    #[arg(long, value_parser = parse_hex)]
    pub center: Option<usize>,

    #[arg(long, value_parser = parse_hex)]
    pub range: Option<usize>,

    /// How often to re-read entities and redraw the overlay (ms).
    /// Lower = smoother but more CPU/GDI overhead. 16 = ~60Hz, 8 = ~120Hz.
    #[arg(long, default_value = "8")]
    pub interval_ms: u64,

    /// Horizontal field of view in degrees (in-game `cg_fov` cvar).
    #[arg(long, default_value = "90.0")]
    pub fov: f32,
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

    let mut client_rect = RECT::default();
    unsafe { GetClientRect(game_hwnd, &mut client_rect) }?;
    let mut top_left = windows::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { ClientToScreen(game_hwnd, &mut top_left) }.ok()?;
    let width = (client_rect.right - client_rect.left).max(1);
    let height = (client_rect.bottom - client_rect.top).max(1);

    println!(
        "ESP overlay on {} (pid {}). Game client area: {}x{} at ({}, {})",
        proc.name, proc.pid, width, height, top_left.x, top_left.y
    );

    let overlay = create_overlay_window(top_left.x, top_left.y, width, height)?;

    println!("Overlay up. Ctrl+C in this terminal to stop.\n");

    let mut cached_addrs: Vec<usize> = locate_snapshot_addrs(&handle, start, end);
    let mut last_server_time = i32::MIN;
    let mut tracks: HashMap<i32, Track> = HashMap::new();

    loop {
        pump_messages();

        // Windows periodically demotes a topmost layered window (game
        // regains focus, alt-tab, a notification pops up, ...) since
        // we only asked for HWND_TOPMOST once at creation. Re-assert
        // it every tick — cheap, and keeps the overlay from silently
        // sinking behind the game.
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

        let cache_ok = !cached_addrs.is_empty()
            && cached_addrs.iter().any(|&a| {
                handle
                    .read::<sdk::SnapshotHeader>(a)
                    .map(|h| looks_like_snapshot(&h))
                    .unwrap_or(false)
            });
        if !cache_ok {
            let fresh = locate_snapshot_addrs(&handle, start, end);
            if fresh.is_empty() {
                cached_addrs.clear();
                thread::sleep(Duration::from_millis(args.interval_ms));
                continue;
            }
            cached_addrs = fresh;
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

        // Our own position/angles come from cg.predictedPlayerState,
        // which the game updates every render frame via client-side
        // prediction (not just at network tick rate) — so it's safe
        // to read fresh every poll, no extrapolation needed here.
        let eye = snap.header.ps.origin;
        let (forward, right, up) = angle_vectors(
            snap.header.ps.viewangles.y, // yaw
            snap.header.ps.viewangles.x, // pitch
        );

        if snap.header.server_time != last_server_time {
            // A real snapshot arrived — refresh velocity for every
            // currently-visible living enemy, and drop tracks for
            // anyone no longer present (dead, disconnected, out of
            // range) so their box doesn't linger extrapolated forever.
            let dt_server = (snap.header.server_time - last_server_time) as f32 / 1000.0;
            let mut seen = std::collections::HashSet::new();

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

        let mut boxes: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
        for (&client_num, track) in &tracks {
            let elapsed = track.wall.elapsed().as_secs_f32();
            let predicted = Vec3::new(
                track.pos.x + track.vel.x * elapsed,
                track.pos.y + track.vel.y * elapsed,
                track.pos.z + track.vel.z * elapsed,
            );
            if let Some(b) = project_player_box(
                predicted,
                client_num,
                eye,
                forward,
                right,
                up,
                width as f32,
                height as f32,
                args.fov,
            ) {
                boxes.push(b);
            }
        }

        draw_overlay(overlay, width, height, &boxes);

        thread::sleep(Duration::from_millis(args.interval_ms));
    }
}

/// Turn one (possibly extrapolated) feet position into an on-screen
/// `(left, top, right, bottom, client_num)` box, or `None` if it's
/// fully behind the camera.
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

/// Quake's `AngleVectors` with roll assumed 0 — returns
/// (forward, right, up) in world space for the given yaw/pitch.
fn angle_vectors(yaw_deg: f32, pitch_deg: f32) -> (Vec3, Vec3, Vec3) {
    let (sy, cy) = yaw_deg.to_radians().sin_cos();
    let (sp, cp) = pitch_deg.to_radians().sin_cos();

    let forward = Vec3::new(cp * cy, cp * sy, -sp);
    let right = Vec3::new(sy, -cy, 0.0);
    let up = Vec3::new(sp * cy, sp * sy, cp);
    (forward, right, up)
}

/// Perspective-project a world point into screen pixel coordinates.
/// Returns `None` if the point is behind (or right on top of) the
/// camera.
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

/// Find a visible top-level window owned by `pid`.
fn find_window_for_pid(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::Win32::Foundation::BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut window_pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        if window_pid == search.pid
            && windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool()
            && windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(hwnd) > 0
        {
            search.found = Some(hwnd);
            return windows::Win32::Foundation::BOOL(0); // stop enumerating
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
        let class_name = w("QCheatEspOverlay");

        let hinstance = windows::Win32::Foundation::HINSTANCE::from(hinstance);

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        // Ignore "class already exists" if we're re-run in the same process.
        let _ = RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(w("qcheat ESP").as_ptr()),
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
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        Ok(hwnd)
    }
}

/// Non-blocking pump so the overlay window doesn't get flagged as
/// "Not Responding" by the OS.
fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Clear the overlay to the transparent color key, then draw one box
/// (plus a client-number label) per living enemy on screen.
fn draw_overlay(hwnd: HWND, width: i32, height: i32, boxes: &[(f32, f32, f32, f32, i32)]) {
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

        let box_pen = CreatePen(PS_SOLID, 2, COLORREF(BOX_COLOR));
        SelectObject(hdc, HGDIOBJ::from(box_pen));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));

        for &(left, top, right, bottom, client_num) in boxes {
            let _ = Rectangle(hdc, left as i32, top as i32, right as i32, bottom as i32);
            let label = w(&format!("#{client_num}"));
            let label_slice = &label[..label.len().saturating_sub(1)]; // drop NUL for TextOutW len
            let _ = TextOutW(hdc, left as i32, (top as i32) - 16, label_slice);
        }

        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ::from(bg_brush));
        let _ = DeleteObject(HGDIOBJ::from(bg_pen));
        let _ = DeleteObject(HGDIOBJ::from(box_pen));
        ReleaseDC(hwnd, hdc);
    }
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
