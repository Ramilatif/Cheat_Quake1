//! External-process discovery and memory access on Windows.
//!
//! Two responsibilities, and only these two:
//!
//! - [`discovery`] — locate a running process by name and enumerate its
//!   loaded modules via the Toolhelp32 snapshot API.
//! - [`handle`] — open a process for reading and copy typed values out
//!   of its address space with [`ReadProcessMemory`][rpm].
//!
//! This crate knows nothing about Quake or any game in particular. It
//! is the foundation every higher-level crate (`scanner`, `engine`)
//! builds on, and stays deliberately small so it can be reviewed end
//! to end in one pass.
//!
//! [rpm]: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-readprocessmemory

#![warn(missing_docs)]
#![cfg(windows)]

pub mod discovery;
pub mod handle;

pub use discovery::{find_by_name, list_modules, Module, Process, ProcessError};
pub use handle::{ProcessHandle, ReadError};
