//! `qcheat inspect` — interpret an arbitrary address as an entityState_t.

use anyhow::{Context, Result};
use clap::{Args as ClapArgs, ValueEnum};
use sdk::{EntityState, EntityType, Vec3, MAX_GENTITIES};

use crate::util::{parse_hex, DEFAULT_PROCESS};

/// How to interpret the address given on the command line.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Mode {
    /// Address is the `origin` field of an EntityState — read the struct at `addr - 92`.
    Origin,
    /// Address is the start of an EntityState — read 208 bytes from there.
    Raw,
    /// Address is a Vec3 — read just 12 bytes and print them.
    Vec3,
}

/// Arguments accepted by `qcheat inspect`.
#[derive(ClapArgs)]
pub struct Args {
    /// Address to dump (hex).
    #[arg(value_parser = parse_hex)]
    pub address: usize,

    /// Name of the target executable.
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    /// Interpretation mode.
    #[arg(long, value_enum, default_value_t = Mode::Origin)]
    pub mode: Mode,
}

/// Read at the given address and pretty-print it according to `mode`.
pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    println!("Attached to {} (pid {})", proc.name, proc.pid);
    let handle = process::ProcessHandle::open(proc.pid)?;

    match args.mode {
        Mode::Vec3 => dump_vec3(&handle, args.address)?,
        Mode::Raw => dump_entity(&handle, args.address)?,
        Mode::Origin => {
            let struct_base = args.address.wrapping_sub(92);
            println!(
                "Assuming 0x{:016X} is the `origin` field.\n\
                 Reading EntityState at 0x{struct_base:016X} (-92).\n",
                args.address
            );
            dump_entity(&handle, struct_base)?;
        }
    }
    Ok(())
}

/// Read 12 bytes at `addr` as a [`Vec3`] and print.
fn dump_vec3(handle: &process::ProcessHandle, addr: usize) -> Result<()> {
    let v: Vec3 = handle
        .read(addr)
        .with_context(|| format!("reading Vec3 at 0x{addr:016X}"))?;
    println!(
        "Vec3 @ 0x{addr:016X} = ({:.2}, {:.2}, {:.2})",
        v.x, v.y, v.z
    );
    Ok(())
}

/// Read 208 bytes at `addr` as an [`EntityState`] and pretty-print
/// every field, flagging values outside sane ranges.
fn dump_entity(handle: &process::ProcessHandle, addr: usize) -> Result<()> {
    let es: EntityState = handle
        .read(addr)
        .with_context(|| format!("reading EntityState at 0x{addr:016X}"))?;

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
        es.origin.x,
        es.origin.y,
        es.origin.z,
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
        println!("Looks like a valid EntityState.");
        if es.e_type == EntityType::PLAYER {
            println!("This is a PLAYER entity (slot {}).", es.client_num);
        }
    } else {
        println!("Fields look out of range — probably not an EntityState here.");
        println!("Try a different offset, e.g. --mode raw, or scan for another copy.");
    }
    Ok(())
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

/// Q3 maps are bounded; no coordinate legitimately exceeds ~±32k.
fn reasonable_coord(v: Vec3) -> bool {
    v.x.is_finite()
        && v.y.is_finite()
        && v.z.is_finite()
        && v.x.abs() < 32_768.0
        && v.y.abs() < 32_768.0
        && v.z.abs() < 32_768.0
}

fn mark(ok: bool) -> &'static str {
    if ok {
        ""
    } else {
        "<-- suspicious"
    }
}
