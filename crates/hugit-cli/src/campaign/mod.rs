//! Campaign — `hugit campaign open/close/show` (WP-PC1).
//!
//! The campaign-lifecycle porcelain: a campaign is the top-altitude bundle that
//! binds a DAG of intents (via their PRs) to an acceptance contract. `open`
//! records `campaign.opened`; `close` is the SEAL (final whole-bundle proof +
//! Ledger "provado" + cost rollup + envelope sealing), NOT a CI trigger; `show`
//! projects landed/in-flight/blocked progress.
//!
//! ## Hermetic file seam
//!
//! Like `why`/`impact`/`export`, every subcommand operates on **local state via
//! a `--log <path>` JSON file** — the CLI's hermetic, file-based seam over the
//! engine. Live DO/CAS binding is the P2 disclosed seam, not this WP's. The
//! input shape ([`world::WorldInput`]) carries the un-hashed events (the
//! orchestrator's local event log, hash-chained through the REAL
//! [`hugit_refstore::EventLog::append`] path) plus the captured envelopes and
//! queue-bundle truth needed to drive the F3 rollup. `open`/`close` append a
//! record and write the log back (`--log` doubles as the output path).
//!
//! ## Output convention
//!
//! **Stable JSON on stdout always** — agents are the primary typists, so the
//! machine shape is the contract. Errors are structured JSON carrying a
//! suggested fix (`{"error":{...}}`, exit nonzero); commands are idempotent
//! (re-running `open` with the same key returns the existing record, exit 0,
//! `"already_exists":true`).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

mod close;
mod open;
mod output;
mod show;
mod world;

pub use output::CampaignError;
pub use world::{Bundle, WorldInput};

/// `hugit campaign <subcommand>` — the campaign lifecycle.
#[derive(clap::Args, Debug)]
pub struct CampaignArgs {
    #[command(subcommand)]
    pub command: CampaignCommand,
}

/// The campaign subcommand surface (WP-PC1).
#[derive(Subcommand, Debug)]
pub enum CampaignCommand {
    /// Open a campaign: charter + human owner (D14) → `campaign.opened` record.
    Open(OpenArgs),
    /// Close (SEAL) a campaign: whole-bundle proof + Ledger "provado" + cost rollup.
    Close(CloseArgs),
    /// Show campaign progress: landed / in-flight / blocked.
    Show(ShowArgs),
}

/// `hugit campaign open` — record `campaign.opened` (idempotent on key).
#[derive(clap::Args, Debug)]
pub struct OpenArgs {
    /// Path to the JSON world file (the local event log + envelopes). Read,
    /// then rewritten with the appended `campaign.opened` record.
    #[arg(long)]
    pub log: PathBuf,
    /// The campaign key (stable identifier; the landing-queue bundle key).
    #[arg(long)]
    pub campaign: String,
    /// Human-readable charter — the campaign's "why".
    #[arg(long)]
    pub charter: String,
    /// The owning **human** principal (D14: a campaign is owned by a human,
    /// never a subagent).
    #[arg(long)]
    pub owner: String,
}

/// `hugit campaign close` — the SEAL (final proof + F3 rollup).
#[derive(clap::Args, Debug)]
pub struct CloseArgs {
    /// Path to the JSON world file. Read, then rewritten with the appended
    /// `campaign.closed` record on success.
    #[arg(long)]
    pub log: PathBuf,
    /// The campaign key to close.
    #[arg(long)]
    pub campaign: String,
}

/// `hugit campaign show` — progress projection (read-only).
#[derive(clap::Args, Debug)]
pub struct ShowArgs {
    /// Path to the JSON world file. Read-only — `show` never writes.
    #[arg(long)]
    pub log: PathBuf,
    /// The campaign key to project.
    #[arg(long)]
    pub campaign: String,
}

/// Dispatch a `campaign` subcommand.
///
/// Each arm runs the real projection over the hermetic file seam and emits a
/// single JSON object on stdout — success or a structured `{"error":{...}}`.
/// Returns the process exit code (0 on success, nonzero on a structured error).
pub fn run(args: CampaignArgs) -> ExitCode {
    let result = match args.command {
        CampaignCommand::Open(a) => open::run(a),
        CampaignCommand::Close(a) => close::run(a),
        CampaignCommand::Show(a) => show::run(a),
    };
    match result {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}
