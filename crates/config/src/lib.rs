//! `qcheat.toml` loading and CLI-flag merging.
//!
//! `qcheat menu` grew enough tunables (hotkeys, sensitivity, aim
//! smoothing, FOV, target mode, ...) that re-typing them as flags on
//! every launch stopped being practical. [`MenuConfig`] mirrors those
//! fields with the same defaults the CLI already used, loaded from a
//! TOML file if one is found; CLI flags still take precedence so a
//! one-off override never has to touch the file.
//!
//! No Windows deps, no knowledge of Quake structures — this crate
//! only knows about text files and numbers.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default file name looked up in the current directory when no
/// explicit `--config` path is given.
pub const DEFAULT_FILE_NAME: &str = "qcheat.toml";

/// Everything `qcheat menu` can source from a config file.
///
/// Every field has a `#[serde(default = ..)]` matching the CLI's own
/// default, so a `qcheat.toml` that only sets the fields someone
/// actually wants to change is valid — missing keys silently fall
/// back to the built-in default rather than erroring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuConfig {
    /// Target executable name (`process::find_by_name`).
    pub process: String,
    /// Loop tick rate in milliseconds.
    pub interval_ms: u64,
    /// In-game `m_yaw` cvar — mouse-to-angle calibration constant.
    pub m_yaw: f32,
    /// In-game `m_pitch` cvar — mouse-to-angle calibration constant.
    pub m_pitch: f32,
    /// Virtual-key code (decimal, or `"0x.."` hex string) that
    /// opens/closes the in-game menu.
    pub menu_key: String,
    /// Virtual-key code that quick-toggles the aimbot on/off.
    pub aimbot_key: String,
    /// Whether the aimbot starts enabled.
    pub aimbot_enabled: bool,
    /// Initial aim sensitivity (matches the in-game `sensitivity` cvar).
    pub sensitivity: f32,
    /// Initial fraction of the remaining angle error corrected per tick.
    pub smooth: f32,
    /// Initial hard cap on raw mouse counts sent in a single tick.
    pub max_delta: f32,
    /// Initial target-selection mode: `"closest"` or `"smallest_angle"`.
    pub target_mode: String,
    /// Whether the wallhack overlay starts enabled.
    pub esp_enabled: bool,
    /// Initial horizontal field of view in degrees, for wallhack
    /// projection (matches the in-game `cg_fov` cvar).
    pub fov: f32,
}

impl Default for MenuConfig {
    fn default() -> Self {
        Self {
            process: "ioquake3.x86_64.exe".to_string(),
            interval_ms: 8,
            m_yaw: 0.022,
            m_pitch: 0.022,
            menu_key: "0x70".to_string(),   // F1
            aimbot_key: "0x2D".to_string(), // Insert
            aimbot_enabled: true,
            sensitivity: 5.0,
            smooth: 0.15,
            max_delta: 60.0,
            target_mode: "closest".to_string(),
            esp_enabled: true,
            fov: 90.0,
        }
    }
}

/// Errors raised while loading or parsing a config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Couldn't read the file at all (permissions, doesn't exist when
    /// explicitly requested via `--config`, ...).
    #[error("reading config file {path}: {source}")]
    Read {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file exists but isn't valid TOML, or a value doesn't match
    /// the expected type.
    #[error("parsing config file {path}: {source}")]
    Parse {
        /// Path that failed to parse.
        path: PathBuf,
        /// Underlying TOML error.
        #[source]
        source: toml::de::Error,
    },
}

/// Load a [`MenuConfig`] from `path`. Any missing key uses the
/// [`Default`] value for that field.
pub fn load(path: &Path) -> Result<MenuConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Resolve and load the config for `qcheat menu`.
///
/// - `explicit_path`: value of `--config`, if the user passed one.
///   Missing/unreadable/invalid is a hard error in this case — an
///   explicit path that doesn't work should not be silently ignored.
/// - Otherwise, look for [`DEFAULT_FILE_NAME`] in the current
///   directory; if it's not there, return the built-in default
///   silently (no config file is the normal, expected case).
pub fn load_or_default(explicit_path: Option<&Path>) -> Result<MenuConfig, ConfigError> {
    match explicit_path {
        Some(p) => load(p),
        None => {
            let default_path = PathBuf::from(DEFAULT_FILE_NAME);
            if default_path.is_file() {
                load(&default_path)
            } else {
                Ok(MenuConfig::default())
            }
        }
    }
}

/// Parse a virtual-key code field: `"0x70"`/`"0X70"` hex, or a plain
/// decimal string. Same accepted forms as the CLI's `--menu-key` /
/// `--aimbot-key` flags.
pub fn parse_vkey(s: &str) -> Result<usize, String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    let radix = if stripped.len() != s.len() { 16 } else { 10 };
    usize::from_str_radix(stripped, radix).map_err(|e| format!("invalid vkey `{s}`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips_through_toml() {
        let cfg = MenuConfig::default();
        let text = toml::to_string(&cfg).unwrap();
        let back: MenuConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let cfg: MenuConfig = toml::from_str("sensitivity = 12.5\n").unwrap();
        assert_eq!(cfg.sensitivity, 12.5);
        assert_eq!(cfg.fov, MenuConfig::default().fov);
    }

    #[test]
    fn parse_vkey_accepts_hex_and_decimal() {
        assert_eq!(parse_vkey("0x70").unwrap(), 0x70);
        assert_eq!(parse_vkey("112").unwrap(), 112);
        assert!(parse_vkey("nope").is_err());
    }

    /// Keeps `qcheat.example.toml` honest: if a field gets renamed or
    /// removed from `MenuConfig`, this fails instead of the example
    /// file silently rotting.
    #[test]
    fn example_toml_parses_and_matches_documented_defaults() {
        let text = include_str!("../../../qcheat.example.toml");
        let cfg: MenuConfig = toml::from_str(text).expect("example file should parse");
        assert_eq!(cfg, MenuConfig::default());
    }
}
