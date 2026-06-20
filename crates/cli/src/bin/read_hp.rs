//! CLI utility: print the 32-bit integer at a given address in
//! `quake3e.x64.exe`, refreshed every 100 ms. Intended for reading HP
//! once you've located its address via Cheat Engine.
//!
//! Usage:
//! ```text
//! cargo run -p memory --bin read-hp -- 0x7FF6A1234ABC
//! cargo run -p memory --bin read-hp -- 0x7FF6A1234ABC quake3e.x64.exe
//! ```

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let addr_str = match args.next() {
        Some(s) => s,
        None => {
            eprintln!("usage: read-hp <address-hex> [process-name]");
            eprintln!("example: read-hp 0x7FF6A1234ABC");
            return ExitCode::FAILURE;
        }
    };
    let process_name = args
        .next()
        .unwrap_or_else(|| "quake3e.x64.exe".to_string());

    let address = match parse_hex(&addr_str) {
        Some(a) => a,
        None => {
            eprintln!("error: address must be hex, e.g. 0x7FF6A1234ABC");
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
    println!("Reading i32 at 0x{address:016X} every 100ms. Ctrl+C to stop.\n");

    let handle = match process::ProcessHandle::open(proc.pid) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    loop {
        match handle.read::<i32>(address) {
            Ok(v) => println!("HP: {v}"),
            Err(e) => {
                eprintln!("read failed: {e}");
                return ExitCode::FAILURE;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Parse a hex literal with optional `0x` / `0X` prefix.
fn parse_hex(s: &str) -> Option<usize> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    usize::from_str_radix(stripped, 16).ok()
}
