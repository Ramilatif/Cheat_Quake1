//! Generic memory-scan primitives shared by every signature-based
//! locator in the workspace.
//!
//! This crate is intentionally game-agnostic: it knows how to stream
//! chunked reads through a process, evaluate a candidate at every
//! aligned offset, and recognise repeating strides between hits. It
//! does **not** know what an `entityState_t` or a `snapshot_t` is —
//! the caller supplies the fingerprint predicate.
//!
//! The two main entry points are:
//! - [`scan_aligned`] — walk a window, hand every `T`-sized aligned
//!   slice to a predicate, return the addresses that pass.
//! - [`detect_repeating_stride`] — given sorted hit addresses, find
//!   the longest run separated by an identical gap (used to recognise
//!   array layouts in scattered hits).

#![warn(missing_docs)]
#![cfg(windows)]

pub mod scan;
pub mod stride;

pub use scan::scan_aligned;
pub use stride::{detect_repeating_stride, StrideMatch};
