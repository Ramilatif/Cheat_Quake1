//! Opened process handle with typed memory reads.
//!
//! A [`ProcessHandle`] owns a Windows `HANDLE` obtained from
//! [`OpenProcess`] with `PROCESS_VM_READ | PROCESS_QUERY_INFORMATION`.
//! The handle is closed automatically on drop, and the type exposes a
//! generic [`read`](ProcessHandle::read) method that copies raw bytes
//! from the target's address space and casts them into any
//! [`bytemuck::Pod`] value — `u32`, `f32`, `[f32; 3]`, `#[repr(C)]`
//! structs, etc.
//!
//! [`OpenProcess`]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocess

use bytemuck::Pod;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
};

/// Owned handle to another process, usable for reading memory.
///
/// Drop closes the underlying Win32 handle, so leaks are impossible even
/// on error paths.
#[derive(Debug)]
pub struct ProcessHandle {
    handle: HANDLE,
    pid: u32,
}

/// Errors raised by a memory read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// [`OpenProcess`] was refused. The usual causes are: target PID no
    /// longer exists, target is a protected process (anti-cheat, service),
    /// or we're not running with sufficient privileges.
    #[error("OpenProcess({pid}) failed: {source}")]
    Open {
        /// PID we tried to open.
        pid: u32,
        /// Underlying Win32 error.
        #[source]
        source: windows::core::Error,
    },
    /// `ReadProcessMemory` failed — invalid address, unmapped page, or
    /// partial read shorter than the requested size.
    #[error("ReadProcessMemory at 0x{address:016X} ({size} bytes) failed: {source}")]
    Read {
        /// Address that was being read.
        address: usize,
        /// Number of bytes requested.
        size: usize,
        /// Underlying Win32 error.
        #[source]
        source: windows::core::Error,
    },
}

impl ProcessHandle {
    /// Open a process for reading. The handle stays valid until this
    /// value is dropped.
    pub fn open(pid: u32) -> Result<Self, ReadError> {
        // PROCESS_VM_READ: required for ReadProcessMemory.
        // PROCESS_QUERY_INFORMATION: lets us later call APIs like
        // GetModuleInformation or VirtualQueryEx without reopening.
        let handle = unsafe {
            OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid)
        }
        .map_err(|source| ReadError::Open { pid, source })?;

        Ok(Self { handle, pid })
    }

    /// PID this handle refers to.
    #[inline]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Read a value of type `T` from `address` in the target process.
    ///
    /// `T` must be [`Pod`] — a fixed-layout type with no padding invariants
    /// that would make arbitrary byte patterns invalid (so no `bool`, no
    /// references, no enums with niches). Typical fits: `i32`, `f32`,
    /// `[f32; 3]`, and your own `#[repr(C)]` mirror structs.
    pub fn read<T: Pod>(&self, address: usize) -> Result<T, ReadError> {
        // Zeroed stack slot to receive the bytes. `Pod` guarantees that
        // an all-zero bit pattern is a valid `T`.
        let mut out = T::zeroed();
        let size = core::mem::size_of::<T>();
        let mut bytes_read: usize = 0;

        // SAFETY: `out` is a valid writable buffer of exactly `size` bytes.
        // ReadProcessMemory copies into it; on success we know it was
        // fully initialised.
        unsafe {
            ReadProcessMemory(
                self.handle,
                address as *const _,
                (&mut out as *mut T).cast(),
                size,
                Some(&mut bytes_read),
            )
        }
        .map_err(|source| ReadError::Read {
            address,
            size,
            source,
        })?;

        // A "successful" partial read still means our data is incomplete
        // and we'd be returning garbage. Treat it as an error.
        if bytes_read != size {
            return Err(ReadError::Read {
                address,
                size,
                source: windows::core::Error::from_win32(),
            });
        }

        Ok(out)
    }

    /// Read `buf.len()` raw bytes from `address` into `buf`.
    ///
    /// Useful when the size isn't known at compile time — typically
    /// when the [`scanner`] crate streams large windows of the target's
    /// memory through a reusable buffer. Prefer the typed
    /// [`read`](Self::read) for fixed-layout structs.
    ///
    /// Returns [`ReadError::Read`] on any partial read.
    pub fn read_into(&self, address: usize, buf: &mut [u8]) -> Result<(), ReadError> {
        let size = buf.len();
        let mut bytes_read: usize = 0;

        // SAFETY: `buf` is a valid writable slice of exactly `size` bytes.
        unsafe {
            ReadProcessMemory(
                self.handle,
                address as *const _,
                buf.as_mut_ptr().cast(),
                size,
                Some(&mut bytes_read),
            )
        }
        .map_err(|source| ReadError::Read {
            address,
            size,
            source,
        })?;

        if bytes_read != size {
            return Err(ReadError::Read {
                address,
                size,
                source: windows::core::Error::from_win32(),
            });
        }

        Ok(())
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
