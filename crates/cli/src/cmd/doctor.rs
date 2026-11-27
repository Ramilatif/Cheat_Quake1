//! `qcheat doctor` — environment diagnostic.
//!
//! Runs every precondition the other subcommands silently assume
//! (process found, memory readable/writable, window locatable, a
//! real snapshot present, synthetic input accepted by the OS) and
//! reports pass/fail for each instead of letting `qcheat menu` fail
//! opaquely partway through. Meant to be the first thing to run after
//! an ioquake3 update, or when something that used to work stops
//! working, per the cahier des charges' "diagnostic integre" goal.

use anyhow::Result;
use clap::Args as ClapArgs;
use sdk::Snapshot;
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

    /// Explicit path to a `qcheat.toml` to validate, same as `qcheat menu --config`.
    #[arg(long)]
    pub config: Option<std::path::PathBuf>,
}

enum Status {
    Ok(String),
    Warn(String),
    Fail(String),
}

pub fn run(args: Args) -> Result<()> {
    let mut checks: Vec<(&str, Status)> = Vec::new();

    // 1. Process discovery.
    let proc = match process::find_by_name(&args.process) {
        Ok(p) => {
            checks.push((
                "Process found",
                Status::Ok(format!(
                    "{} — pid {}, base 0x{:016X}",
                    p.name, p.pid, p.base_address
                )),
            ));
            Some(p)
        }
        Err(e) => {
            checks.push((
                "Process found",
                Status::Fail(format!("{e} — is {} running?", args.process)),
            ));
            None
        }
    };

    // 2. Handle open with read+write access.
    let handle = proc.as_ref().and_then(|p| match process::ProcessHandle::open(p.pid) {
        Ok(h) => {
            checks.push((
                "Memory access",
                Status::Ok("OpenProcess granted PROCESS_VM_READ | PROCESS_VM_WRITE".to_string()),
            ));
            Some(h)
        }
        Err(e) => {
            checks.push((
                "Memory access",
                Status::Fail(format!("{e} — try running qcheat as Administrator")),
            ));
            None
        }
    });

    // 3. Game window locatable (needed by `esp`/`menu` for the overlay).
    if let Some(p) = &proc {
        match find_window_for_pid(p.pid) {
            Some(_) => checks.push((
                "Game window",
                Status::Ok("visible top-level window found".to_string()),
            )),
            None => checks.push((
                "Game window",
                Status::Warn(
                    "no visible window yet — fine if ioquake3 is still starting; \
                     required for `qcheat esp`/`qcheat menu`"
                        .to_string(),
                ),
            )),
        }
    }

    // 4. A real, currently-active snapshot.
    if let Some(h) = &handle {
        let center = args.center.unwrap_or(DEFAULT_CENTER);
        let range = args.range.unwrap_or(DEFAULT_RANGE);
        let start = center.saturating_sub(range);
        let end = center.saturating_add(range);

        let hits = find_snapshot_candidates(h, start, end);
        if hits.is_empty() {
            checks.push((
                "Snapshot located",
                Status::Warn(
                    "no candidate found — normal if not currently in a match; \
                     join one (or /addbot) and re-run"
                        .to_string(),
                ),
            ));
        } else {
            const SS_SIZE: usize = core::mem::size_of::<Snapshot>();
            let pair = hits.windows(2).find(|w| w[1] - w[0] == SS_SIZE);
            match pair {
                Some(w) => {
                    let snap_addr = w[0];
                    match h.read::<Snapshot>(snap_addr) {
                        Ok(s) => checks.push((
                            "Snapshot located",
                            Status::Ok(format!(
                                "cg.activeSnapshots pair confirmed, {} entities, HP {}",
                                s.header.num_entities,
                                s.header.ps.stats[sdk::STAT_HEALTH]
                            )),
                        )),
                        Err(e) => checks.push((
                            "Snapshot located",
                            Status::Fail(format!("pair found but read failed: {e}")),
                        )),
                    }
                }
                None => checks.push((
                    "Snapshot located",
                    Status::Warn(format!(
                        "{} candidate(s) but no confirmed 53772-byte pair — \
                         widen --range or try again",
                        hits.len()
                    )),
                )),
            }
        }
    }

    // 5. Synthetic mouse input accepted by the OS (the aimbot's whole
    // mechanism). A zero-motion event is non-disruptive but still
    // exercises the exact same SendInput path as a real correction.
    {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(&[input], core::mem::size_of::<INPUT>() as i32) };
        if sent == 1 {
            checks.push((
                "Mouse injection",
                Status::Ok("SendInput accepted the event".to_string()),
            ));
        } else {
            checks.push((
                "Mouse injection",
                Status::Fail(
                    "SendInput reported 0 events accepted — another process (a game \
                     running elevated, an anti-cheat, UIPI) may be blocking synthetic input"
                        .to_string(),
                ),
            ));
        }
    }

    // 6. qcheat.toml, if any.
    match config::load_or_default(args.config.as_deref()) {
        Ok(cfg) => {
            let source = args
                .config
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| {
                    if std::path::Path::new(config::DEFAULT_FILE_NAME).is_file() {
                        config::DEFAULT_FILE_NAME.to_string()
                    } else {
                        "(none — using built-in defaults)".to_string()
                    }
                });
            checks.push((
                "Config file",
                Status::Ok(format!("{source} — process=\"{}\"", cfg.process)),
            ));
        }
        Err(e) => checks.push(("Config file", Status::Fail(e.to_string()))),
    }

    // --- report ---
    println!("qcheat doctor\n");
    let mut fails = 0;
    let mut warns = 0;
    for (name, status) in &checks {
        let (icon, detail) = match status {
            Status::Ok(d) => ("[OK]  ", d.as_str()),
            Status::Warn(d) => {
                warns += 1;
                ("[WARN]", d.as_str())
            }
            Status::Fail(d) => {
                fails += 1;
                ("[FAIL]", d.as_str())
            }
        };
        println!("{icon} {name:<16} {detail}");
    }

    println!();
    if fails > 0 {
        println!("{fails} check(s) failed, {warns} warning(s) — see above.");
    } else if warns > 0 {
        println!("All critical checks passed, {warns} warning(s) — see above.");
    } else {
        println!("All checks passed.");
    }

    Ok(())
}

fn find_window_for_pid(pid: u32) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    };

    struct Search {
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut window_pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        if window_pid == search.pid
            && IsWindowVisible(hwnd).as_bool()
            && GetWindowTextLengthW(hwnd) > 0
        {
            search.found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut search = Search { pid, found: None };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
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
