//! External-process memory access primitives.
//!
//! Two responsibilities:
//!
//! - [`process`] — locate a running process by name and resolve the base
//!   address of its main module.
//! - [`handle`] — open a process for reading and cast raw bytes into
//!   typed values via [`ReadProcessMemory`][rpm].
//!
//! These form the foundation every later feature (HP reader, ESP, aimbot)
//! builds on.
//!
//! [rpm]: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-readprocessmemory

#![warn(missing_docs)]
#![cfg(windows)]

pub mod handle;
pub mod process;

pub use handle::{ProcessHandle, ReadError};
pub use process::{find_by_name, list_modules, Module, Process, ProcessError};
