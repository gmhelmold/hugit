//! `hugit issue` — issue lifecycle verbs.
//!
//! Currently ships `transition` (`hugit issue transition --log <path> --n <n>
//! --to <state> [--priority <p>]`), which appends an `issue.transition` record
//! onto the canonical `--log` through the D14 Orchestrator/Land guard — CLI
//! parity with the serve verb `write_issue_transition` (no web-only verb).

use std::process::ExitCode;

use clap::Subcommand;

pub mod transition;

pub use transition::TransitionArgs;

/// `hugit issue <subcommand>`.
#[derive(clap::Args, Debug)]
pub struct IssueArgs {
    #[command(subcommand)]
    pub command: IssueCommand,
}

/// Issue subcommand surface.
#[derive(Subcommand, Debug)]
pub enum IssueCommand {
    /// Move an issue's state to backlog|open|closed|dispatch.
    Transition(TransitionArgs),
}

/// Dispatch a `hugit issue` subcommand.
pub fn run(args: IssueArgs) -> ExitCode {
    match args.command {
        IssueCommand::Transition(a) => transition::run(a),
    }
}
