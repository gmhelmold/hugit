//! PR — `hugit pr open/land/show` (WP-PC3).
//!
//! The pull-request porcelain (git-proximate: `hugit pr open` ≈ `gh pr
//! create`). `open` bundles intents → PROPOSED (authz: author is an
//! orchestrator/human, never a subagent — D14 at the door); `land` enters the
//! landing queue (`LandableEntry`) and reports position; `show` projects the
//! PR record incl. the F3 `pr_record` rollup.
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
//! error on stdout and exits 2. The real projections land in WP-PC3.

use clap::Subcommand;

use crate::porcelain::not_implemented;

/// The work package that fills the pr stubs (carried in the stub error).
const STUB_WP: &str = "PC3";

/// `hugit pr <subcommand>` — the pull-request lifecycle.
#[derive(clap::Args, Debug)]
pub struct PrArgs {
    #[command(subcommand)]
    pub command: PrCommand,
}

/// The pr subcommand surface (body lands in WP-PC3).
#[derive(Subcommand, Debug)]
pub enum PrCommand {
    /// Open a PR: bundle intents → PROPOSED (D14 author authz at the door).
    Open,
    /// Land a PR: enter the landing queue (`LandableEntry`), report position.
    Land,
    /// Show a PR: the PR record incl. the F3 `pr_record` rollup.
    Show,
}

/// Dispatch a `pr` subcommand.
///
/// Scaffold (WP-PC0): every arm emits the structured NOT-IMPLEMENTED error and
/// returns its exit code. WP-PC3 replaces each arm with the real projection.
pub fn run(args: PrArgs) -> std::process::ExitCode {
    match args.command {
        PrCommand::Open | PrCommand::Land | PrCommand::Show => not_implemented(STUB_WP),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::porcelain::not_implemented_json;

    // Smoke (WP-PC0): every pr subcommand routes to the honest stub and emits
    // the structured NOT-IMPLEMENTED error for PC3. `run` returns an ExitCode
    // (consumed by main.rs); the route+shape is asserted via the stub WP token.
    // PC3 REPLACES these expectations with the real projections.
    #[test]
    fn each_subcommand_routes_to_not_implemented() {
        for command in [PrCommand::Open, PrCommand::Land, PrCommand::Show] {
            let _ = run(PrArgs { command });
        }
        assert_eq!(STUB_WP, "PC3");
        assert_eq!(
            not_implemented_json(STUB_WP),
            r#"{"error":{"kind":"not_implemented","wp":"PC3"}}"#
        );
    }
}
