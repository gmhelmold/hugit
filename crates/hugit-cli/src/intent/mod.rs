//! Intent — `hugit intent new/show` (WP-PC2).
//!
//! The intent porcelain: an intent is the unit of orchestrated work (charter +
//! acceptance, bound to a campaign). `new` builds the frozen `IntentSidecar` and
//! lands it onto the local event log through the REAL refstore path
//! ([`hugit_refstore::intent::import_sidecar`]) — the same seam the dogfood wave
//! drives — returning the intent id; `show` projects the native intent off the
//! log plus the sidecar corpus, the context-envelope ref, and any verdicts.
//!
//! ## Output convention
//!
//! **Stable JSON on stdout always** — agents are the primary typists, so the
//! machine shape is the contract (a `--human` pretty mode lands later). Errors
//! are a single structured JSON object carrying the suggested fix
//! ([`error::PorcelainError`]); commands are idempotent (re-running `new` with
//! the same id, explicit or content-derived, returns the existing record with
//! `already_exists:true`, exit 0).
//!
//! ## Module layout
//!
//! - [`store`] — the local hermetic event-log store (`--store` file seam).
//! - [`new`]   — `hugit intent new`: author through the real refstore path.
//! - [`show`]  — `hugit intent show`: project the intent record honestly.
//! - [`error`] — the structured, fix-carrying porcelain error.

pub mod error;
pub mod new;
pub mod show;
pub mod store;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

use crate::porcelain::PORCELAIN_ERROR_EXIT;

/// The default store path when `--store` is not given (local, hermetic).
const DEFAULT_STORE: &str = ".hugit/intents.json";

/// `hugit intent <subcommand>` — the intent ceremony.
#[derive(clap::Args, Debug)]
pub struct IntentArgs {
    #[command(subcommand)]
    pub command: IntentCommand,
}

/// The intent subcommand surface.
#[derive(Subcommand, Debug)]
pub enum IntentCommand {
    /// New intent: charter / acceptance / campaign → sidecar + refstore claim.
    New {
        /// Human-readable charter of what the intent intends to do.
        #[arg(long)]
        charter: String,
        /// The campaign key this intent is bound to.
        #[arg(long)]
        campaign: String,
        /// Acceptance criteria (repeatable).
        #[arg(long = "acceptance")]
        acceptance: Vec<String>,
        /// Explicit intent id (idempotency key); content-derived when omitted.
        #[arg(long)]
        id: Option<String>,
        /// The authoring agent type (defaults to the honest `"main"`).
        #[arg(long)]
        agent: Option<String>,
        /// Content-addressed ref to the full context blob (the envelope).
        #[arg(long = "context-ref")]
        context_ref: Option<String>,
        /// The local intent store file.
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
    },
    /// Show an intent: native projection + sidecar + envelope ref + verdicts.
    Show {
        /// The intent id to project.
        #[arg(long)]
        intent: String,
        /// The local intent store file.
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
    },
}

/// Dispatch an `intent` subcommand: run the real projection, print ONE stable
/// JSON object on stdout, and map the outcome to the process exit code (0 on
/// success, [`PORCELAIN_ERROR_EXIT`] on a structured error).
pub fn run(args: IntentArgs) -> ExitCode {
    match args.command {
        IntentCommand::New {
            charter,
            campaign,
            acceptance,
            id,
            agent,
            context_ref,
            store,
        } => {
            let input = new::NewIntent {
                charter,
                campaign,
                acceptance,
                id,
                agent,
                context_ref,
            };
            match new::run(input, &store) {
                Ok(result) => print_ok(&result),
                Err(e) => print_err(&e),
            }
        }
        IntentCommand::Show { intent, store } => {
            let input = show::ShowIntent { intent_id: intent };
            match show::run(input, &store) {
                Ok(value) => {
                    println!("{value}");
                    ExitCode::SUCCESS
                }
                Err(e) => print_err(&e),
            }
        }
    }
}

/// Print a successful result as one JSON object and return success.
fn print_ok<T: serde::Serialize>(result: &T) -> ExitCode {
    match serde_json::to_string(result) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        // A serialisation failure of our own type is an internal fault.
        Err(e) => print_err(&error::PorcelainError::new(
            "internal",
            format!("serialise result: {e}"),
            "this is an internal bug; report it",
        )),
    }
}

/// Print a structured error as one JSON object on stdout and return the
/// structured-error exit code.
fn print_err(e: &error::PorcelainError) -> ExitCode {
    println!("{}", e.to_json());
    ExitCode::from(PORCELAIN_ERROR_EXIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_error_is_the_stable_object_shape() {
        let e = error::PorcelainError::new("not_found", "no intent x", "create it first");
        assert_eq!(
            e.to_json(),
            r#"{"error":{"kind":"not_found","message":"no intent x","suggested_fix":"create it first"}}"#
        );
    }

    #[test]
    fn new_result_serialises_stably() {
        let r = new::NewResult {
            intent_id: "intent-abc".to_string(),
            already_exists: false,
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"intent_id":"intent-abc","already_exists":false}"#
        );
    }
}
