//! Campaign — `hugit campaign open/close/show` (WP-PC1).
//!
//! The campaign-lifecycle porcelain: a campaign is the top-altitude bundle that
//! binds a DAG of intents to an acceptance contract. `open` records
//! `campaign.opened`; `close` is the SEAL (final whole-bundle proof + Ledger
//! "provado" + cost rollup + envelope sealing), NOT a CI trigger; `show`
//! projects landed/in-flight/blocked progress.
//!
//! ## Output convention
//!
//! **Stable JSON on stdout always** — agents are the primary typists, so the
//! machine shape is the contract (a `--human` pretty mode lands later). Errors
//! are structured JSON carrying the suggested fix; commands are idempotent
//! (re-running `open` with the same key returns the existing record, exit 0).
//!
//! ## Status
//!
//! Scaffold only (WP-PC0): every subcommand emits a structured NOT-IMPLEMENTED
//! error on stdout and exits 2. The real projections land in WP-PC1.

use clap::Subcommand;

use crate::porcelain::not_implemented;

/// The work package that fills the campaign stubs (carried in the stub error).
const STUB_WP: &str = "PC1";

/// `hugit campaign <subcommand>` — the campaign lifecycle.
#[derive(clap::Args, Debug)]
pub struct CampaignArgs {
    #[command(subcommand)]
    pub command: CampaignCommand,
}

/// The campaign subcommand surface (body lands in WP-PC1).
#[derive(Subcommand, Debug)]
pub enum CampaignCommand {
    /// Open a campaign: charter + human owner (D14) → `campaign.opened` record.
    Open,
    /// Close (SEAL) a campaign: whole-bundle proof + Ledger "provado" + cost rollup.
    Close,
    /// Show campaign progress: landed / in-flight / blocked.
    Show,
}

/// Dispatch a `campaign` subcommand.
///
/// Scaffold (WP-PC0): every arm emits the structured NOT-IMPLEMENTED error and
/// returns its exit code. WP-PC1 replaces each arm with the real projection.
pub fn run(args: CampaignArgs) -> std::process::ExitCode {
    match args.command {
        CampaignCommand::Open | CampaignCommand::Close | CampaignCommand::Show => {
            not_implemented(STUB_WP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::porcelain::not_implemented_json;

    // Smoke (WP-PC0): every campaign subcommand routes to the honest stub and
    // emits the structured NOT-IMPLEMENTED error for PC1. `run` returns an
    // ExitCode (consumed by main.rs); the route+shape is asserted via the stub
    // WP token. PC1 REPLACES these expectations with the real projections.
    #[test]
    fn each_subcommand_routes_to_not_implemented() {
        for command in [
            CampaignCommand::Open,
            CampaignCommand::Close,
            CampaignCommand::Show,
        ] {
            // Routes without panicking (the dispatch wiring is live).
            let _ = run(CampaignArgs { command });
        }
        assert_eq!(STUB_WP, "PC1");
        assert_eq!(
            not_implemented_json(STUB_WP),
            r#"{"error":{"kind":"not_implemented","wp":"PC1"}}"#
        );
    }
}
