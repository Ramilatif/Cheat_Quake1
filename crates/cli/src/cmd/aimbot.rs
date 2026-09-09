//! `qcheat aimbot` — aim assistance targeting the closest visible player.

use anyhow::Result;
use clap::Args as ClapArgs;
use sdk::{EntityState, EntityType, Snapshot, MAX_ENTITIES_IN_SNAPSHOT};
use std::thread;
use std::time::Duration;

use crate::util::{parse_hex, DEFAULT_PROCESS};

const CHUNK: usize = 4096;
const HEADER_SIZE: usize = core::mem::size_of::<sdk::SnapshotHeader>();
const DEFAULT_CENTER: usize = 0x07000000;
const DEFAULT_RANGE: usize = 0x02000000;

// Offsets trouvés empiriquement (tableau de 32 snapshots)
const CL_VIEWANGLES_YAW_BASE: usize = 0x7B4098;
const CL_VIEWANGLES_PITCH_BASE: usize = 0x7B409C;
const SNAPSHOT_STRIDE: usize = 0x21C;  // 540 bytes = sizeof(clSnapshot_t)
const NUM_SNAPSHOTS: usize = 32;

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    #[arg(long, value_parser = parse_hex)]
    pub center: Option<usize>,

    #[arg(long, value_parser = parse_hex)]
    pub range: Option<usize>,

    #[arg(long, default_value = "100")]
    pub interval_ms: u64,
}

pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    let handle = process::ProcessHandle::open(proc.pid)?;
    let module_base = proc.base_address;

    let center = args.center.unwrap_or(DEFAULT_CENTER);
    let range = args.range.unwrap_or(DEFAULT_RANGE);
    let start = center.saturating_sub(range);
    let end = center.saturating_add(range);

    println!(
        "Aimbot active on {} (pid {}). Scanning for snapshot...\n",
        proc.name, proc.pid
    );

    // Build list of all 32 snapshot addresses
    let mut yaw_addrs = Vec::new();
    let mut pitch_addrs = Vec::new();

    for i in 0..NUM_SNAPSHOTS {
        yaw_addrs.push(module_base + CL_VIEWANGLES_YAW_BASE + (i * SNAPSHOT_STRIDE));
        pitch_addrs.push(module_base + CL_VIEWANGLES_PITCH_BASE + (i * SNAPSHOT_STRIDE));
    }

    println!("Target addresses ({} snapshots):", NUM_SNAPSHOTS);
    println!("  Yaw base:   0x{:016X}", yaw_addrs[0]);
    println!("  Pitch base: 0x{:016X}\n", pitch_addrs[0]);

    loop {
        // Find snapshot
        let hits = find_snapshot_candidates(&handle, start, end);
        if hits.is_empty() {
            println!(".");
            thread::sleep(Duration::from_millis(args.interval_ms));
            continue;
        }

        println!("\nFound {} snapshot candidate(s)", hits.len());

        const SS_SIZE: usize = core::mem::size_of::<Snapshot>();
        let pair = hits.windows(2).find(|w| w[1] - w[0] == SS_SIZE);

        let snap_addr = match pair {
            Some(w) => {
                println!("✓ Found snapshot pair!");
                let a = w[0];
                let b = w[1];
                let ta = handle
                    .read::<sdk::SnapshotHeader>(a)
                    .map(|h| h.server_time)
                    .unwrap_or(i32::MIN);
                let tb = handle
                    .read::<sdk::SnapshotHeader>(b)
                    .map(|h| h.server_time)
                    .unwrap_or(i32::MIN);
                if ta >= tb { a } else { b }
            }
            None => {
                println!("✗ No 53772-byte pair found, using first candidate");
                hits[0]
            }
        };

        // Read snapshot
        let snap = match handle.read::<Snapshot>(snap_addr) {
            Ok(s) => {
                println!("✓ Read snapshot at 0x{:016X}, {} entities", snap_addr, s.header.num_entities);
                Box::new(s)
            }
            Err(e) => {
                println!("✗ Failed to read snapshot: {}", e);
                thread::sleep(Duration::from_millis(args.interval_ms));
                continue;
            }
        };

        // Find closest player
        let local_pos = snap.header.ps.origin;
        let mut closest: Option<(&EntityState, f32)> = None;
        let mut entity_types = vec![];

        for es in &snap.entities[..snap.header.num_entities.min(MAX_ENTITIES_IN_SNAPSHOT as i32) as usize] {
            entity_types.push(es.e_type);
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
                Some((_, closest_dist)) if dist < *closest_dist => closest = Some((es, dist)),
                _ => {}
            }
        }

        // Calculate and write angles to ALL 32 snapshots
        if let Some((target, dist)) = closest {
            let target_pos = target.pos.tr_base;
            let angles = calculate_angles(local_pos, target_pos);

            // Write to all 32 snapshot copies
            let mut write_count = 0;
            for i in 0..NUM_SNAPSHOTS {
                let _ = handle.write_f32(yaw_addrs[i], angles.0);
                let _ = handle.write_f32(pitch_addrs[i], angles.1);
                write_count += 1;
            }

            println!(
                "Target: client {}, dist {:.1}m, angles ({:.1}°, {:.1}°) [wrote to {} snapshots]",
                target.client_num, dist, angles.1, angles.0, write_count
            );
        } else {
            let players = entity_types.iter().filter(|&&t| t == EntityType::PLAYER as i32).count();
            println!("No players found ({} entities): types={:?}", snap.header.num_entities, entity_types);
        }

        thread::sleep(Duration::from_millis(args.interval_ms));
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

fn calculate_angles(from: sdk::Vec3, to: sdk::Vec3) -> (f32, f32) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let dz = to.z - from.z;

    let yaw = dy.atan2(dx).to_degrees();
    let horiz_dist = (dx * dx + dy * dy).sqrt();
    let pitch = (-dz).atan2(horiz_dist).to_degrees();

    (yaw, pitch)
}

#[repr(C)]
#[derive(Copy, Clone)]
struct Chunk([u8; CHUNK + HEADER_SIZE]);

unsafe impl bytemuck::Zeroable for Chunk {}
unsafe impl bytemuck::Pod for Chunk {}
