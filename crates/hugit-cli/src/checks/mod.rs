//! `hugit checks` — the memoized-CI wedge made visible (WB2; honest stub here).
//!
//! The audit's P1 finding: the wedge — memoized CI hit-rate, memo keys, union-
//! batch state, failure attribution — is unreachable by the agent. WB0 lands the
//! VERB into the registry (so the surface exists and the no-drift oracle counts
//! it) as a PC0-style honest stub; WB2 fills the projection over the canonical
//! `--log` seam.
//!
//! Subcommands (skeleton):
//!   - `show`  — the memoized-CI hit-rate + union-batch state for a target.
//!   - `key`   — the content memo key a given check resolves to.
//!
//! Every arm currently returns the canonical NOT-IMPLEMENTED envelope
//! (`{"error":{"kind":"not_implemented","wp":"WB2", …}}`, exit `2`) — never a
//! fake success. The verb is LIVE the moment `main.rs` routes it.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

use crate::porcelain::not_implemented;

/// The work package that fills these stubs with the real projection.
const OWNING_WP: &str = "WB2";

/// `hugit checks <subcommand>` — memoized-CI visibility (WB2).
#[derive(clap::Args, Debug)]
pub struct ChecksArgs {
    #[command(subcommand)]
    pub command: ChecksCommand,
}

/// The checks subcommand surface — `show` / `key`.
#[derive(Subcommand, Debug)]
pub enum ChecksCommand {
    /// Show the memoized-CI hit-rate + union-batch state for a target.
    Show(ShowArgs),
    /// Show the content memo key a given check resolves to.
    Key(KeyArgs),
}

/// `hugit checks show` flags (skeleton).
#[derive(clap::Args, Debug)]
pub struct ShowArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`) the checks
    /// project from — the one `--log` seam every porcelain verb shares.
    #[arg(long)]
    pub log: PathBuf,
    /// The target (PR id / ref) whose checks to project.
    #[arg(long)]
    pub target: Option<String>,
}

/// `hugit checks key` flags (skeleton).
#[derive(clap::Args, Debug)]
pub struct KeyArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`).
    #[arg(long)]
    pub log: PathBuf,
    /// The check name to resolve a memo key for.
    #[arg(long)]
    pub check: Option<String>,
}

/// Dispatch a `checks` subcommand. Returns the process exit code directly (the
/// porcelain verbs own their exit code; see `main.rs`).
pub fn run(args: ChecksArgs) -> ExitCode {
    match args.command {
        ChecksCommand::Show(_) => not_implemented(OWNING_WP),
        ChecksCommand::Key(_) => not_implemented(OWNING_WP),
    }
}
