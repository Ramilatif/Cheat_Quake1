//! External-process memory access primitives.
//!
//! For now this crate exposes only process discovery: given an executable
//! name such as `quake3e.x64.exe`, find the running PID and the base
//! address of its main module. Later iterations will add `ReadProcessMemory`
//! / `WriteProcessMemory` wrappers on top.

#![warn(missing_docs)]
#![cfg(windows)]

pub mod process;

pub use process::{find_by_name, Process, ProcessError};
