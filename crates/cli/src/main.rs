//! `qcheat` — unified entry point for the Cheat_Quake1 toolkit.
//!
//! Every operation the project supports is reachable as a subcommand
//! of this single binary. Run `qcheat --help` to list them all.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod cmd;
mod util;

/// External-process reader for ioquake3 on Windows.
#[derive(Parser)]
#[command(name = "qcheat", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Aim assistance targeting the closest visible player.
    Aimbot(cmd::aimbot::Args),
    /// Aim assistance via synthetic mouse input (SendInput).
    AimMouse(cmd::aim_mouse::Args),
    /// Wallhack overlay — draws boxes over enemies through walls.
    Esp(cmd::esp::Args),
    /// Locate the target process and report its PID + module base.
    Find(cmd::find::Args),
    /// Scan memory for the live view-angle variable used by rendering.
    FindViewangles(cmd::find_viewangles::Args),
    /// List every DLL loaded in the target process.
    Modules(cmd::modules::Args),
    /// Poll a 32-bit value at a fixed address (typically HP).
    Hp(cmd::hp::Args),
    /// Pretty-print an arbitrary address as an entityState_t.
    Inspect(cmd::inspect::Args),
    /// Unified aimbot + wallhack with an in-game settings menu (F1).
    Menu(cmd::menu::Args),
    /// Scan a memory window for entityState_t-shaped bytes.
    Scan(cmd::scan::Args),
    /// Heap-wide scan for live player entities.
    Players(cmd::players::Args),
    /// Locate cg.activeSnapshots and dump the active frame.
    Snapshot(cmd::snapshot::Args),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Aimbot(a) => cmd::aimbot::run(a),
        Cmd::AimMouse(a) => cmd::aim_mouse::run(a),
        Cmd::Esp(a) => cmd::esp::run(a),
        Cmd::Find(a) => cmd::find::run(a),
        Cmd::FindViewangles(a) => cmd::find_viewangles::run(a),
        Cmd::Modules(a) => cmd::modules::run(a),
        Cmd::Hp(a) => cmd::hp::run(a),
        Cmd::Inspect(a) => cmd::inspect::run(a),
        Cmd::Menu(a) => cmd::menu::run(a),
        Cmd::Scan(a) => cmd::scan::run(a),
        Cmd::Players(a) => cmd::players::run(a),
        Cmd::Snapshot(a) => cmd::snapshot::run(a),
    }
}
