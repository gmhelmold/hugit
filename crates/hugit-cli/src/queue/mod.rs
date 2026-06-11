//! `hugit queue` — the union-batch landing-queue wedge made visible (WB2; stub).
//!
//! The audit's P1 finding: `pr land` returns a bare position with no ETA /
//! batch / blame, and the union-batch state is otherwise invisible. WB0 lands
//! the VERB into the registry (so the surface exists and the no-drift oracle
//! counts it) as a PC0-style honest stub; WB2 fills the projection over the
//! canonical `--log` seam.
//!
//! Subcommands (skeleton):
//!   - `show`  — the current landing-queue state: batch membership, position,
//!     ETA, and failure attribution.
//!
//! The arm currently returns the canonical NOT-IMPLEMENTED envelope
//! (`{"error":{"kind":"not_implemented","wp":"WB2", …}}`, exit `2`) — never a
//! fake success. The verb is LIVE the moment `main.rs` routes it.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

use crate::porcelain::not_implemented;

/// The work package that fills this stub with the real projection.
const OWNING_WP: &str = "WB2";

/// `hugit queue <subcommand>` — landing-queue visibility (WB2).
#[derive(clap::Args, Debug)]
pub struct QueueArgs {
    #[command(subcommand)]
    pub command: QueueCommand,
}

/// The queue subcommand surface — `show`.
#[derive(Subcommand, Debug)]
pub enum QueueCommand {
    /// Show the landing-queue state: batch, position, ETA, failure attribution.
    Show(ShowArgs),
}

/// `hugit queue show` flags (skeleton).
#[derive(clap::Args, Debug)]
pub struct ShowArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`) the queue
    /// projects from — the one `--log` seam every porcelain verb shares.
    #[arg(long)]
    pub log: PathBuf,
    /// Optional campaign key to scope the queue projection to.
    #[arg(long)]
    pub campaign: Option<String>,
}

/// Dispatch a `queue` subcommand. Returns the process exit code directly (the
/// porcelain verbs own their exit code; see `main.rs`).
pub fn run(args: QueueArgs) -> ExitCode {
    match args.command {
        QueueCommand::Show(_) => not_implemented(OWNING_WP),
    }
}
