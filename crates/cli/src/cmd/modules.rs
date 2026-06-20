//! `qcheat modules` — enumerate every module loaded in the target.

use anyhow::Result;
use clap::Args as ClapArgs;

use crate::util::DEFAULT_PROCESS;

/// Arguments accepted by `qcheat modules`.
#[derive(ClapArgs)]
pub struct Args {
    /// Name of the target executable.
    #[arg(long, default_value = DEFAULT_PROCESS)]
    pub process: String,
    /// Optional case-insensitive substring filter on the module name.
    #[arg(long)]
    pub filter: Option<String>,
}

/// Walk the module list and print each entry.
pub fn run(args: Args) -> Result<()> {
    let filter = args.filter.map(|s| s.to_ascii_lowercase());

    let proc = process::find_by_name(&args.process)?;
    println!("Attached to {} (pid {})\n", proc.name, proc.pid);

    let modules = process::list_modules(proc.pid)?;

    println!("{:<40}  {:<18}  {:>10}", "name", "base", "size (KiB)");
    println!("{:-<40}  {:-<18}  {:-<10}", "", "", "");

    let mut shown = 0usize;
    for m in &modules {
        if let Some(f) = &filter {
            if !m.name.to_ascii_lowercase().contains(f) {
                continue;
            }
        }
        shown += 1;
        println!(
            "{:<40}  0x{:016X}  {:>10}",
            m.name,
            m.base_address,
            m.size / 1024
        );
    }

    if shown == 0 {
        println!("(no module matched the filter)");
    } else {
        println!(
            "\n{} module(s) shown, {} total loaded.",
            shown,
            modules.len()
        );
    }
    Ok(())
}
