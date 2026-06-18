//! `hugit policy` — declarative gate management.
//!
//! Currently ships `test` (`hugit policy test --context <path>`), which runs the
//! canonical house gate set ([`hugit_policy::Engine::house`]) against a supplied
//! [`EvalContext`] and reports each gate's outcome. This is the SAME evaluator
//! codepath the forge landing enforcement uses — `local ≡ forge` is a structural
//! property (WP-D6 ①), so `policy test` gives a byte-identical local preview of
//! what the forge gate will decide.
//!
//! `policy edit` (mutating the gate set via a guarded `policy.change` event) is
//! deferred: the engine has no gate-set persistence/mutation model yet (no
//! current-gates-from-log accumulator, no apply-mutation), so wiring `edit` would
//! require a design decision rather than a faithful mirror. `policy` is therefore
//! graduated as `test`-only — an honest partial surface, not a stub.

use std::process::ExitCode;

use clap::Subcommand;

pub mod test;

pub use test::TestArgs;

/// `hugit policy <subcommand>`.
#[derive(clap::Args, Debug)]
pub struct PolicyArgs {
    #[command(subcommand)]
    pub command: PolicyCommand,
}

/// Policy subcommand surface.
#[derive(Subcommand, Debug)]
pub enum PolicyCommand {
    /// Run the house gate set against a context file and report each outcome.
    Test(TestArgs),
}

/// Dispatch a `hugit policy` subcommand.
pub fn run(args: PolicyArgs) -> ExitCode {
    match args.command {
        PolicyCommand::Test(a) => test::run(a),
    }
}
