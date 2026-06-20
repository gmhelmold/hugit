//! `hugit policy` — declarative gate management.
//!
//! Currently ships `test` (`hugit policy test --context <path>`), which runs the
//! canonical house gate set ([`hugit_policy::Engine::house`]) against a supplied
//! [`EvalContext`] and reports each gate's outcome. This is the SAME evaluator
//! codepath the forge landing enforcement uses — `local ≡ forge` is a structural
//! property (WP-D6 ①), so `policy test` gives a byte-identical local preview of
//! what the forge gate will decide.
//!
//! `policy edit` (`hugit policy edit --gate <name> --enable|--disable`) is ALSO
//! REAL: it reconstructs the current gate set by folding prior `policy.change`
//! events over the [`hugit_policy::house_gates`] baseline (latest-wins), toggles
//! the named gate, and appends the new set as a Human-only `policy.change` record
//! (the [`hugit_policy::emit_policy_change`] wire shape) through the D14
//! Human-only guard. The log IS the gate-set persistence model — an append-only
//! accumulator, no second store. So `policy` ships BOTH `test` and `edit`; neither
//! is a stub.

use std::process::ExitCode;

use clap::Subcommand;

pub mod edit;
pub mod test;

pub use edit::EditArgs;
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
    /// Toggle a house gate's enabled state (records a Human-only policy.change).
    Edit(EditArgs),
}

/// Dispatch a `hugit policy` subcommand.
pub fn run(args: PolicyArgs) -> ExitCode {
    match args.command {
        PolicyCommand::Test(a) => test::run(a),
        PolicyCommand::Edit(a) => edit::run(a),
    }
}
