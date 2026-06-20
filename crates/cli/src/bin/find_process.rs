//! CLI utility: find a Quake III process and print where it lives.
//!
//! Usage:
//! ```text
//! cargo run -p memory --bin find-process              # defaults to quake3e.x64.exe
//! cargo run -p memory --bin find-process quake3.exe
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let target = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "quake3e.x64.exe".to_string());

    match process::find_by_name(&target) {
        Ok(p) => {
            println!("Found process:");
            println!("  name         : {}", p.name);
            println!("  pid          : {}", p.pid);
            println!("  base address : 0x{:016X}", p.base_address);
            println!("  module size  : 0x{:X} ({} bytes)", p.module_size, p.module_size);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
