//! Subcommands wired into `qcheat`.
//!
//! Each submodule is one CLI verb. The contract is identical across
//! all of them:
//!
//! - a `pub struct Args` derived from [`clap::Args`] that declares its
//!   flags and positional arguments,
//! - a `pub fn run(args: Args) -> anyhow::Result<()>` entry point.
//!
//! Keeping the contract uniform makes the dispatch in `main.rs`
//! mechanical and lets contributors add a new verb by following the
//! shape of an existing one.

pub mod find;
pub mod hp;
pub mod inspect;
pub mod modules;
pub mod players;
pub mod scan;
pub mod snapshot;
