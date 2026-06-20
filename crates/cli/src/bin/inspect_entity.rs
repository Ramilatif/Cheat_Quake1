//! Reverse-engineering helper: given a candidate address (e.g. one that
//! Cheat Engine pointed at when we were scanning our own position),
//! try to interpret a Quake III `entityState_t` around it and print
//! every field with sanity indicators.
//!
//! The typical workflow is:
//! 1. In Cheat Engine, locate a copy of your XYZ position.
//! 2. Pass that address here — we assume it's the `origin` field of an
//!    [`EntityState`], so we read the struct starting at `addr - 92`.
//! 3. Inspect the output: if `number` is in `0..1024`, `e_type == 1`
//!    (ET_PLAYER), and `client_num` is small, you found
//!    `cg_entities[clientNum].currentState`.
//!
//! Usage:
//! ```text
//! cargo run -p memory --bin inspect-entity -- 0x0613F728
//! cargo run -p memory --bin inspect-entity -- 0x0613F728 ioquake3.x86_64.exe
//! cargo run -p memory --bin inspect-entity -- 0x0613F728 ioquake3.x86_64.exe raw
//! ```
//!
//! The optional third argument switches the interpretation:
//! - (absent) — assume the address is an `origin` and read struct at `addr-92`
//! - `raw`    — assume the address IS the start of the `entityState_t`
//! - `vec3`   — read just 12 bytes as a [`sdk::Vec3`] (for quick spot-checks)

use std::process::ExitCode;

use sdk::{EntityState, EntityType, Vec3, MAX_GENTITIES};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let addr_str = match args.next() {
        Some(s) => s,
        None => {
            eprintln!("usage: inspect-entity <address-hex> [process-name] [mode]");
            eprintln!("  mode = raw | vec3 | (default: origin-93)");
            return ExitCode::FAILURE;
        }
    };
    let process_name = args
        .next()
        .unwrap_or_else(|| "ioquake3.x86_64.exe".to_string());
    let mode = args.next().unwrap_or_else(|| "origin".to_string());

    let address = match parse_hex(&addr_str) {
        Some(a) => a,
        None => {
            eprintln!("error: address must be hex, e.g. 0x0613F728");
            return ExitCode::FAILURE;
        }
    };

    let proc = match process::find_by_name(&process_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("Attached to {} (pid {})", proc.name, proc.pid);

    let handle = match process::ProcessHandle::open(proc.pid) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    match mode.as_str() {
        "vec3" => dump_vec3(&handle, address),
        "raw" => dump_entity(&handle, address),
        _ => {
            // Default: caller pointed at an `origin` field — the struct
            // actually begins 92 bytes earlier.
            let struct_base = address.wrapping_sub(92);
            println!(
                "Assuming 0x{address:016X} is the `origin` field.\n\
                 Reading EntityState at 0x{struct_base:016X} (-92).\n"
            );
            dump_entity(&handle, struct_base);
        }
    }

    ExitCode::SUCCESS
}

/// Read 12 bytes at `addr` as a [`Vec3`] and print.
fn dump_vec3(handle: &process::ProcessHandle, addr: usize) {
    match handle.read::<Vec3>(addr) {
        Ok(v) => println!("Vec3 @ 0x{addr:016X} = ({:.2}, {:.2}, {:.2})", v.x, v.y, v.z),
        Err(e) => eprintln!("read failed: {e}"),
    }
}

/// Read 208 bytes at `addr` as an [`EntityState`], print every field,
/// and flag values that fall outside the sensible ranges so a bad guess
/// is obvious at a glance.
fn dump_entity(handle: &process::ProcessHandle, addr: usize) {
    let es: EntityState = match handle.read(addr) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("read failed: {e}");
            return;
        }
    };

    let number_ok = (0..MAX_GENTITIES as i32).contains(&es.number);
    let etype_ok = (0..=12).contains(&es.e_type);
    let client_ok = es.client_num >= -1 && es.client_num < 64;
    let origin_ok = reasonable_coord(es.origin);

    println!("EntityState @ 0x{addr:016X}");
    println!("  number          : {}  {}", es.number, mark(number_ok));
    println!(
        "  e_type          : {} ({})  {}",
        es.e_type,
        etype_name(es.e_type),
        mark(etype_ok)
    );
    println!("  e_flags         : 0x{:08X}", es.e_flags);
    println!(
        "  pos.tr_type     : {}  (base={:?})",
        es.pos.tr_type,
        (es.pos.tr_base.x, es.pos.tr_base.y, es.pos.tr_base.z)
    );
    println!(
        "  origin          : ({:.2}, {:.2}, {:.2})  {}",
        es.origin.x, es.origin.y, es.origin.z,
        mark(origin_ok)
    );
    println!(
        "  angles          : ({:.2}, {:.2}, {:.2})",
        es.angles.x, es.angles.y, es.angles.z
    );
    println!("  client_num      : {}  {}", es.client_num, mark(client_ok));
    println!("  weapon          : {}", es.weapon);
    println!("  modelindex      : {}", es.modelindex);
    println!("  event           : {}", es.event);
    println!();

    let looks_good = number_ok && etype_ok && client_ok && origin_ok;
    if looks_good {
        println!("✅ Looks like a valid EntityState.");
        if es.e_type == EntityType::PLAYER {
            println!("   This is a PLAYER entity (slot {}).", es.client_num);
        }
    } else {
        println!("❌ Fields look out of range — probably not an EntityState here.");
        println!("   Try a different offset, e.g. `raw` mode or scan for another copy.");
    }
}

/// Translate an `e_type` integer into a short human name.
fn etype_name(t: i32) -> &'static str {
    match t {
        0 => "GENERAL",
        1 => "PLAYER",
        2 => "ITEM",
        3 => "MISSILE",
        4 => "MOVER",
        5 => "BEAM",
        6 => "PORTAL",
        7 => "SPEAKER",
        8 => "PUSH_TRIGGER",
        9 => "TELEPORT_TRIGGER",
        10 => "INVISIBLE",
        11 => "GRAPPLE",
        12 => "TEAM",
        _ => "???",
    }
}

/// Q3 maps are bounded, no coordinate ever legitimately exceeds ~±32k.
fn reasonable_coord(v: Vec3) -> bool {
    v.x.is_finite()
        && v.y.is_finite()
        && v.z.is_finite()
        && v.x.abs() < 32_768.0
        && v.y.abs() < 32_768.0
        && v.z.abs() < 32_768.0
}

fn mark(ok: bool) -> &'static str {
    if ok { "" } else { "<-- suspicious" }
}

fn parse_hex(s: &str) -> Option<usize> {
    let stripped = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    usize::from_str_radix(stripped, 16).ok()
}
