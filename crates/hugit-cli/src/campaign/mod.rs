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
//! `--log` file is the **one canonical on-disk seam every porcelain verb
//! shares** (PC4): a JSON `[EventRecord, …]` array — the engine's
//! [`hugit_refstore::EventLog`] shape. The campaign projects everything it needs
//! (captured envelopes, PR bundles, the campaign envelope ref) **off the records
//! on that log**, so a PR opened by `hugit pr open` and an intent landed by
//! `hugit intent new --log` compose with these verbs on one shared file.
//! `open`/`close` append a record and write the log back (`--log` doubles as
//! the output path).
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

mod abandon;
mod close;
mod list;
mod open;
mod output;
pub(crate) mod seal_guard;
mod show;
pub(crate) mod world;

pub use output::CampaignError;

/// `hugit campaign <subcommand>` — the campaign lifecycle.
#[derive(clap::Args, Debug)]
pub struct CampaignArgs {
    #[command(subcommand)]
    pub command: CampaignCommand,
}

/// The campaign subcommand surface (WP-PC1 + WP-WB-CAMP).
#[derive(Subcommand, Debug)]
pub enum CampaignCommand {
    /// Open a campaign: charter + human owner (D14) → `campaign.opened` record.
    Open(OpenArgs),
    /// Close (SEAL) a campaign: whole-bundle proof + Ledger "provado" + cost rollup.
    Close(CloseArgs),
    /// Show campaign progress: landed / in-flight / blocked.
    Show(ShowArgs),
    /// List all campaigns on the log: key, charter, owner, state, progress.
    List(ListArgs),
    /// Abandon a campaign: appends `campaign.abandoned`; releases in-flight PR
    /// blocking from close semantics. Idempotent. Abandoning a closed campaign
    /// is a structured error.
    Abandon(AbandonArgs),
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
    /// Allow sealing a campaign that has rejected intents.  Without this flag
    /// `close` refuses with `campaign_has_rejected`/exit-2 when any intent
    /// carries a non-approve latest verdict — an operator must explicitly
    /// acknowledge that rejected work is being sealed over (WI-PROVEN2).
    #[arg(long, default_value_t = false)]
    pub allow_rejected: bool,
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

/// `hugit campaign list` — enumerate all campaigns on the log (read-only).
#[derive(clap::Args, Debug)]
pub struct ListArgs {
    /// Path to the JSON world file. Read-only — `list` never writes.
    #[arg(long)]
    pub log: PathBuf,
}

/// `hugit campaign abandon` — mark a campaign abandoned (idempotent).
#[derive(clap::Args, Debug)]
pub struct AbandonArgs {
    /// Path to the JSON world file. Read, then rewritten with the appended
    /// `campaign.abandoned` record on success.
    #[arg(long)]
    pub log: PathBuf,
    /// The campaign key to abandon.
    #[arg(long)]
    pub campaign: String,
    /// Human-readable reason for abandoning (required — honest attribution).
    #[arg(long)]
    pub reason: String,
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
        CampaignCommand::List(a) => list::run(a),
        CampaignCommand::Abandon(a) => abandon::run(a),
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
