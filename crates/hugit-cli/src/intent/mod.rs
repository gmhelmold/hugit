//! Intent — `hugit intent new/show` (WP-PC2).
//!
//! The intent porcelain: an intent is the unit of orchestrated work (charter +
//! acceptance, optionally bound to a campaign). `new` writes the sidecar and
//! claims it through the refstore seam, returning the intent id; `show`
//! projects the sidecar + envelope refs + verdicts.
//!
//! ## Output convention
//!
//! **Stable JSON on stdout always** — agents are the primary typists, so the
//! machine shape is the contract (a `--human` pretty mode lands later). Errors
//! are structured JSON carrying the suggested fix; commands are idempotent
//! (re-running `new` with the same key returns the existing record, exit 0).
//!
//! ## Status
//!
//! Scaffold only (WP-PC0): every subcommand emits a structured NOT-IMPLEMENTED
//! error on stdout and exits 2. The real projections land in WP-PC2.

use clap::Subcommand;

use crate::porcelain::not_implemented;

/// The work package that fills the intent stubs (carried in the stub error).
const STUB_WP: &str = "PC2";

/// `hugit intent <subcommand>` — the intent ceremony.
#[derive(clap::Args, Debug)]
pub struct IntentArgs {
    #[command(subcommand)]
    pub command: IntentCommand,
}

/// The intent subcommand surface (body lands in WP-PC2).
#[derive(Subcommand, Debug)]
pub enum IntentCommand {
    /// New intent: charter / acceptance / campaign → sidecar + refstore claim.
    New,
    /// Show an intent: sidecar + envelope refs + verdicts.
    Show,
}

/// Dispatch an `intent` subcommand.
///
/// Scaffold (WP-PC0): every arm emits the structured NOT-IMPLEMENTED error and
/// returns its exit code. WP-PC2 replaces each arm with the real projection.
pub fn run(args: IntentArgs) -> std::process::ExitCode {
    match args.command {
        IntentCommand::New | IntentCommand::Show => not_implemented(STUB_WP),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::porcelain::not_implemented_json;

    // Smoke (WP-PC0): every intent subcommand routes to the honest stub and
    // emits the structured NOT-IMPLEMENTED error for PC2. `run` returns an
    // ExitCode (consumed by main.rs); the route+shape is asserted via the stub
    // WP token. PC2 REPLACES these expectations with the real projections.
    #[test]
    fn each_subcommand_routes_to_not_implemented() {
        for command in [IntentCommand::New, IntentCommand::Show] {
            let _ = run(IntentArgs { command });
        }
        assert_eq!(STUB_WP, "PC2");
        assert_eq!(
            not_implemented_json(STUB_WP),
            r#"{"error":{"kind":"not_implemented","wp":"PC2"}}"#
        );
    }
}
