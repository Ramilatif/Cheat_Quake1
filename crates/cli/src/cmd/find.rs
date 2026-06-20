//! `qcheat find` — locate a process by name and report its metadata.

use anyhow::Result;
use clap::Args as ClapArgs;

use crate::util::DEFAULT_PROCESS;

/// Arguments accepted by `qcheat find`.
#[derive(ClapArgs)]
pub struct Args {
    /// Name of the target executable (case-insensitive).
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,
}

/// Locate the process and print PID + module base.
pub fn run(args: Args) -> Result<()> {
    let p = process::find_by_name(&args.process)?;
    println!("Found process:");
    println!("  name         : {}", p.name);
    println!("  pid          : {}", p.pid);
    println!("  base address : 0x{:016X}", p.base_address);
    println!(
        "  module size  : 0x{:X} ({} bytes)",
        p.module_size, p.module_size
    );
    Ok(())
}
