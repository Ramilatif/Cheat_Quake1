//! `qcheat hp` — poll a 32-bit value at a fixed address.

use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::Args as ClapArgs;

use crate::util::{parse_hex, DEFAULT_PROCESS};

/// Arguments accepted by `qcheat hp`.
#[derive(ClapArgs)]
pub struct Args {
    /// Address (hex) of the i32 to poll, typically the engine-side HP.
    #[arg(value_parser = parse_hex)]
    pub address: usize,

    /// Name of the target executable.
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,

    /// Polling interval in milliseconds.
    #[arg(long, default_value_t = 100)]
    pub interval_ms: u64,
}

/// Read the i32 at `args.address` forever, on the requested cadence.
pub fn run(args: Args) -> Result<()> {
    let proc = process::find_by_name(&args.process)?;
    println!("Attached to {} (pid {})", proc.name, proc.pid);
    println!(
        "Reading i32 at 0x{:016X} every {} ms. Ctrl+C to stop.\n",
        args.address, args.interval_ms
    );

    let handle = process::ProcessHandle::open(proc.pid)?;
    let dt = Duration::from_millis(args.interval_ms);
    loop {
        let v: i32 = handle.read(args.address)?;
        println!("HP: {v}");
        thread::sleep(dt);
    }
}
