//! Shared helpers used by multiple subcommands.

/// Default ioquake3 process name used everywhere a `--process` flag is
/// optional. Centralised so renaming the target only needs one edit.
pub const DEFAULT_PROCESS: &str = "ioquake3.x86_64.exe";

/// Parse a hex literal with optional `0x` / `0X` prefix into a
/// `usize`. Suitable as a `clap` `value_parser` for address-shaped
/// arguments.
pub fn parse_hex(s: &str) -> Result<usize, String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    usize::from_str_radix(stripped, 16)
        .map_err(|e| format!("invalid hex `{s}`: {e}"))
}
