//! List every module (`.exe` + `.dll`s) loaded by a running process.
//!
//! Useful when we need to aim a memory scan at a specific DLL rather than
//! the heap. In Quake III terms: `cg_entities[]` lives in the `.bss` of
//! `cgamex86_64.dll` (the cgame VM loaded dynamically), so once we know
//! that DLL's base + size we can scan precisely that range.
//!
//! Usage:
//! ```text
//! cargo run -p memory --bin list-modules
//! cargo run -p memory --bin list-modules -- ioquake3.x86_64.exe
//! cargo run -p memory --bin list-modules -- ioquake3.x86_64.exe cgame
//! ```
//!
//! The optional second argument is a case-insensitive substring filter on
//! the module name (e.g. `cgame`, `.dll`, `ioquake3`).

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let process_name = args
        .next()
        .unwrap_or_else(|| "ioquake3.x86_64.exe".to_string());
    let filter = args.next().map(|s| s.to_ascii_lowercase());

    let proc = match memory::find_by_name(&process_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("Attached to {} (pid {})\n", proc.name, proc.pid);

    let modules = match memory::list_modules(proc.pid) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "{:<40}  {:<18}  {:>10}",
        "name", "base", "size (KiB)"
    );
    println!("{:-<40}  {:-<18}  {:-<10}", "", "", "");

    let mut shown = 0;
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
        println!("\n{} module(s) shown, {} total loaded.", shown, modules.len());
    }

    ExitCode::SUCCESS
}
