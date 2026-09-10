//! `qcheat find-viewangles` — locate the live view-angle variable
//! actually used for rendering (cg.refdefViewAngles / cl.viewangles),
//! as opposed to the cl.snapshots[] history ring buffer.
//!
//! Strategy: read the known-correct YAW from the current snapshot
//! (ground truth), scan process memory for floats close to that
//! value (excluding the snapshot ring buffer itself), then ask the
//! player to turn their view and keep only the candidates whose
//! value tracked the change. A handful of manual Cheat Engine passes
//! kept converging on the snapshot history buffer or stack noise;
//! this automates the same "changed value" idea with a known-good
//! reference instead of guesswork.

use anyhow::{bail, Result};
use clap::Args as ClapArgs;
use scanner::scan_aligned;
use std::thread;
use std::time::Duration;

use crate::util::{parse_hex, DEFAULT_PROCESS};

const CHUNK: usize = 4096;
const HEADER_SIZE: usize = core::mem::size_of::<sdk::SnapshotHeader>();
const SNAP_DEFAULT_CENTER: usize = 0x07000000;
const SNAP_DEFAULT_RANGE: usize = 0x02000000;
const SNAPSHOT_STRIDE: usize = 0x21C;
const NUM_SNAPSHOTS: usize = 32;

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    /// Scan window centre for the *candidate* float search (hex).
    #[arg(long, value_parser = parse_hex)]
    pub center: Option<usize>,

    /// Scan window half-range for the candidate float search (hex).
    #[arg(long, value_parser = parse_hex)]
    pub range: Option<usize>,

    /// Degrees of tolerance when matching a candidate against the
    /// known-good viewangle.
    #[arg(long, default_value = "3.0")]
    pub tolerance: f32,

    /// Seconds to wait between each confirmation pass — move the mouse
    /// during this window.
    #[arg(long, default_value = "3")]
    pub wait_secs: u64,

    /// Number of tight-tolerance confirmation rounds to run after the
    /// initial broad scan. With several bots on the server, a single
    /// +/-3 degree pass keeps matching unrelated entities by
    /// coincidence — running several tight rounds and intersecting
    /// them crushes that noise out.
    #[arg(long, default_value = "4")]
    pub rounds: u32,
}

pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;

    let center = args.center.unwrap_or(0x05000000);
    let range = args.range.unwrap_or(0x05000000);
    let start = center.saturating_sub(range);
    let end = center.saturating_add(range);

    println!("Locating cg.activeSnapshots for a ground-truth viewangle...");
    let (snap_addr, yaw1, pitch1) = read_reference_angles(&handle)?;
    println!(
        "Reference angles: yaw={:.2} pitch={:.2}  (snapshot @ 0x{:016X})\n",
        yaw1, pitch1, snap_addr
    );

    // Exclude the known cl.snapshots[] ring buffer — every copy in
    // there will spuriously match since it's fed the same value.
    let excl_start = snap_addr.saturating_sub(0x1000);
    let excl_end = snap_addr + NUM_SNAPSHOTS * SNAPSHOT_STRIDE + 0x1000;
    println!(
        "Excluding known snapshot ring buffer: 0x{:016X}..0x{:016X}\n",
        excl_start, excl_end
    );

    println!(
        "Scanning 0x{:016X}..0x{:016X} ({} MiB) for yaw ~= {:.2} (+/- {:.1})...",
        start,
        end,
        (end - start) / (1024 * 1024),
        yaw1,
        args.tolerance
    );

    let tol = args.tolerance;
    let hits = scan_aligned::<f32, _>(&handle, start, end, 4, |v: &f32| {
        v.is_finite() && (*v - yaw1).abs() <= tol
    })?;

    let mut candidates: Vec<usize> = hits
        .into_iter()
        .map(|h| h.address)
        .filter(|&a| a < excl_start || a >= excl_end)
        .collect();

    println!("Found {} candidate(s) after exclusion.\n", candidates.len());
    if candidates.is_empty() {
        println!("No candidates — try widening --range or --tolerance.");
        return Ok(());
    }
    if candidates.len() > 500 {
        println!(
            "That's a lot ({}) — the confirmation pass below will narrow it down.",
            candidates.len()
        );
    }

    // Tight tolerance for the confirmation rounds: real copies of the
    // same viewangle should match almost exactly every round, while a
    // coincidentally-close bot/entity value won't survive several
    // independent tight passes in a row.
    const TIGHT_TOL: f32 = 0.05;
    let mut prev_yaw = yaw1;

    for round in 1..=args.rounds {
        if candidates.is_empty() {
            break;
        }
        println!(
            "\n>>> [Round {}/{}] Bouge la souris pour changer ton YAW ({} secondes)...",
            round, args.rounds, args.wait_secs
        );
        thread::sleep(Duration::from_secs(args.wait_secs));

        let (_, yaw, _pitch) = read_reference_angles(&handle)?;
        println!("    New yaw = {:.2}", yaw);

        if (yaw - prev_yaw).abs() < 0.5 {
            println!(
                "    ! Le YAW a a peine change ({:.2} -> {:.2}). Bouge plus la souris.",
                prev_yaw, yaw
            );
        }
        prev_yaw = yaw;

        candidates.retain(|&addr| match handle.read::<f32>(addr) {
            Ok(v) => (v - yaw).abs() <= TIGHT_TOL,
            Err(_) => false,
        });
        println!(
            "    Candidats survivants: {} (tolerance +/- {})",
            candidates.len(),
            TIGHT_TOL
        );
    }

    println!("\nCandidats finaux: {}\n", candidates.len());
    for addr in candidates.iter().take(50) {
        let v = handle.read::<f32>(*addr).unwrap_or(f32::NAN);
        // Report the module-relative offset too, since that's what
        // you'll want to hardcode into the aimbot afterwards.
        let module_off = addr.checked_sub(proc.base_address);
        match module_off {
            Some(off) => println!("  0x{addr:016X}  (module+0x{off:X})  = {v:.2}"),
            None => println!("  0x{addr:016X}  = {v:.2}"),
        }
    }
    if candidates.len() > 50 {
        println!("  ... and {} more", candidates.len() - 50);
    }

    if candidates.is_empty() {
        println!(
            "\nAucun survivant. Essaie d'augmenter --tolerance, ou --wait-secs, \
             et bouge la souris plus franchement pendant l'attente."
        );
    } else {
        println!(
            "\nTeste chacune de ces adresses a la main dans Cheat Engine: \
             ecris une valeur dedans et verifie si la camera bouge en jeu."
        );
    }

    Ok(())
}

/// Locate the live `cg.activeSnapshots` pair and return
/// `(chosen_snapshot_addr, yaw, pitch)` from its `ps.viewangles`.
fn read_reference_angles(handle: &process::ProcessHandle) -> Result<(usize, f32, f32)> {
    let start = SNAP_DEFAULT_CENTER.saturating_sub(SNAP_DEFAULT_RANGE);
    let end = SNAP_DEFAULT_CENTER.saturating_add(SNAP_DEFAULT_RANGE);

    let hits = find_snapshot_candidates(handle, start, end);
    if hits.is_empty() {
        bail!("No snapshot candidates found — is the game running and in a match?");
    }

    const SS_SIZE: usize = core::mem::size_of::<sdk::Snapshot>();
    let pair = hits.windows(2).find(|w| w[1] - w[0] == SS_SIZE);

    let addr = match pair {
        Some(w) => {
            let ta = handle
                .read::<sdk::SnapshotHeader>(w[0])
                .map(|h| h.server_time)
                .unwrap_or(i32::MIN);
            let tb = handle
                .read::<sdk::SnapshotHeader>(w[1])
                .map(|h| h.server_time)
                .unwrap_or(i32::MIN);
            if ta >= tb { w[0] } else { w[1] }
        }
        None => hits[0],
    };

    let snap = handle.read::<sdk::Snapshot>(addr)?;
    Ok((addr, snap.header.ps.viewangles.y, snap.header.ps.viewangles.x))
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
