//! Process discovery via the Windows Toolhelp32 snapshot API.
//!
//! We use a snapshot rather than `EnumProcesses` + `OpenProcess` + module
//! enumeration because Toolhelp32 returns process names and module base
//! addresses in two tight loops without needing `PROCESS_QUERY_INFORMATION`
//! rights on every candidate — handy on systems where some processes are
//! protected and would fail the open.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Process32FirstW, Process32NextW, MODULEENTRY32W,
    PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};

/// Information about a running process matched by name.
#[derive(Debug, Clone)]
pub struct Process {
    /// Windows process id.
    pub pid: u32,
    /// Executable file name (e.g. `quake3e.x64.exe`).
    pub name: String,
    /// Base virtual address of the main module in the target process's
    /// address space. All RVA-based offsets (pattern scans, hardcoded
    /// constants) are relative to this.
    pub base_address: usize,
    /// Size in bytes of the main module's image.
    pub module_size: usize,
}

/// Errors that can surface while looking a process up.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// The process snapshot could not be created (usually access denied
    /// or out-of-resources from the kernel).
    #[error("CreateToolhelp32Snapshot failed: {0}")]
    SnapshotFailed(#[from] windows::core::Error),
    /// The snapshot succeeded but no process matched the requested name.
    #[error("no process named `{0}` is currently running")]
    NotFound(String),
    /// The process was found but its module list could not be walked — for
    /// 64-bit targets this usually means we're running as a 32-bit binary
    /// and WoW64 redirection is hiding the module.
    #[error("process `{0}` found but its main module could not be read")]
    NoMainModule(String),
}

/// Look up the first running process whose executable name matches
/// `target_name` (case-insensitive, e.g. `"quake3e.x64.exe"`).
///
/// Returns the full [`Process`] descriptor, including the base address of
/// the main module — everything a memory reader needs to resolve hardcoded
/// offsets.
pub fn find_by_name(target_name: &str) -> Result<Process, ProcessError> {
    let snapshot = Snapshot::new(TH32CS_SNAPPROCESS, 0)?;

    let mut entry = PROCESSENTRY32W {
        dwSize: core::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    unsafe {
        if Process32FirstW(snapshot.handle, &mut entry).is_err() {
            return Err(ProcessError::NotFound(target_name.to_string()));
        }

        loop {
            let name = wide_to_string(&entry.szExeFile);
            if name.eq_ignore_ascii_case(target_name) {
                let (base, size) = module_info(entry.th32ProcessID)
                    .ok_or_else(|| ProcessError::NoMainModule(name.clone()))?;
                return Ok(Process {
                    pid: entry.th32ProcessID,
                    name,
                    base_address: base,
                    module_size: size,
                });
            }
            if Process32NextW(snapshot.handle, &mut entry).is_err() {
                return Err(ProcessError::NotFound(target_name.to_string()));
            }
        }
    }
}

/// Read the first module (== main executable) of `pid` and return its base
/// address and image size. `None` if the module snapshot fails — most often
/// because of a 32-bit-vs-64-bit mismatch with our own process.
fn module_info(pid: u32) -> Option<(usize, usize)> {
    // SNAPMODULE | SNAPMODULE32 so a 32-bit host can still enumerate a
    // 64-bit target (and vice versa). The first module returned by
    // Module32FirstW is always the main .exe.
    let snapshot = Snapshot::new(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid).ok()?;

    let mut module = MODULEENTRY32W {
        dwSize: core::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };

    unsafe {
        Module32FirstW(snapshot.handle, &mut module).ok()?;
    }

    Some((module.modBaseAddr as usize, module.modBaseSize as usize))
}

/// Convert a NUL-terminated UTF-16 slice (as returned by the Win32 APIs)
/// into an owned `String`, stopping at the first zero.
fn wide_to_string(wide: &[u16]) -> String {
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    OsString::from_wide(&wide[..len])
        .to_string_lossy()
        .into_owned()
}

/// RAII wrapper so the snapshot handle is always closed, even on early
/// return from the iteration loop.
struct Snapshot {
    handle: HANDLE,
}

impl Snapshot {
    fn new(flags: windows::Win32::System::Diagnostics::ToolHelp::CREATE_TOOLHELP_SNAPSHOT_FLAGS, pid: u32) -> windows::core::Result<Self> {
        let handle = unsafe { CreateToolhelp32Snapshot(flags, pid)? };
        if handle == INVALID_HANDLE_VALUE {
            return Err(windows::core::Error::from_win32());
        }
        Ok(Self { handle })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
