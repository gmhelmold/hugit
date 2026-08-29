//! Intent — `hugit intent new/show/list` (WP-PC2).
//!
//! The intent porcelain: an intent is the unit of orchestrated work (charter +
//! acceptance, bound to a campaign). `new` builds the frozen `IntentSidecar` and
//! lands it onto the local event log through the REAL refstore path
//! ([`hugit_refstore::intent::import_sidecar`]) — the same seam the dogfood wave
//! drives — returning the intent id; `show` projects the native intent off the
//! log plus the sidecar corpus, the context-envelope ref, and any verdicts;
//! `list` enumerates every intent in the store (the discovery verb agents need
//! to recover a lost id).
//!
//! ## Output convention
//!
//! **Stable JSON on stdout always** — agents are the primary typists, so the
//! machine shape is the contract (a `--human` pretty mode lands later). Errors
//! are a single structured JSON object carrying the fix hint
//! ([`error::PorcelainError`]); commands are idempotent (re-running `new` with
//! the same id, explicit or content-derived, returns the existing record with
//! `already_exists:true`, exit 0).
//!
//! ## Module layout
//!
//! - [`store`] — the local hermetic event-log store (`--store` file seam).
//! - [`new`]   — `hugit intent new`: author through the real refstore path.
//! - [`show`]  — `hugit intent show`: project the intent record honestly.
//! - [`list`]  — `hugit intent list`: enumerate all intents (the discovery verb).
//! - [`error`] — the structured, fix-carrying porcelain error (WB0 canonical shape).

pub mod canonical_log;
pub mod error;
pub mod list;
pub mod new;
pub mod show;
pub mod store;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

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
    /// List all intents in the store (the agent discovery verb — recovers a lost id).
    ///
    /// Prints a stable JSON object `{"intents":[…]}` where every item carries
    /// `id`, `charter` (first-80-char excerpt), `campaign`, `agent`, `landed`,
    /// and `log`. `landed` is resolved per-intent against its OWN owning log
    /// (recorded at `intent new --log`), so the default (no `--log`) global call
    /// is a truthful one-call fleet view — not `landed:null` for every intent.
    List {
        /// The local intent store file.
        #[arg(long, default_value = DEFAULT_STORE)]
        store: PathBuf,
        /// Optional SCOPE FILTER: restrict the listing to intents authored
        /// against this exact log (canonical-path match). Omit for the global,
        /// all-logs view. (`landed` is resolved per-intent regardless of this.)
        #[arg(long)]
        log: Option<PathBuf>,
        /// Filter to one campaign key (omit for all campaigns).
        #[arg(long)]
        campaign: Option<String>,
    },
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
        /// Optional **shared canonical event log** (the one `[EventRecord, …]`
        /// seam `hugit pr`/`hugit campaign` read). When given, `intent.landed`
        /// is ALSO appended here so a later `pr open --intent <id>` can validate
        /// the intent exists. Omit it to keep the `--store`-only behavior.
        #[arg(long)]
        log: Option<PathBuf>,
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
        IntentCommand::List {
            store,
            log,
            campaign,
        } => {
            let input = list::ListIntents { log, campaign };
            match list::run(input, &store) {
                Ok(result) => print_ok(&result),
                Err(e) => print_err(&e),
            }
        }
        IntentCommand::New {
            charter,
            campaign,
            acceptance,
            id,
            agent,
            context_ref,
            store,
            log,
        } => {
            let input = new::NewIntent {
                charter,
                campaign,
                acceptance,
                id,
                agent,
                context_ref,
                log,
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
        // A serialisation failure of our own type is an internal fault (exit 1).
        Err(e) => print_err(&error::PorcelainError::internal(format!(
            "serialise result: {e}"
        ))),
    }
}

/// Print a structured error as one JSON object on stdout and return the
/// structured-error exit code.
fn print_err(e: &error::PorcelainError) -> ExitCode {
    println!("{}", e.to_json());
    // PR-5: respect the internal flag — bug-class faults exit 1, domain 2.
    e.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_error_is_the_stable_object_shape() {
        let e = error::PorcelainError::new("not_found", "no intent x", "create it first");
        let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        // The WB0 canonical envelope: {"error":{"kind":…,"message":…,"fix":…}}.
        assert_eq!(v["error"]["kind"], "not_found");
        assert_eq!(v["error"]["message"], "no intent x");
        assert_eq!(v["error"]["fix"], "create it first");
        // "fix" is the key, not "suggested_fix".
        assert!(v["error"].get("suggested_fix").is_none());
    }

    #[test]
    fn new_result_serialises_stably() {
        // First-run shape (stable key-set invariant — all fields present).
        let r = new::NewResult {
            intent_id: "intent-abc".to_string(),
            already_exists: false,
            campaign: "cli-porcelain".to_string(),
            agent: "main".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"intent_id":"intent-abc","already_exists":false,"campaign":"cli-porcelain","agent":"main"}"#
        );
    }

    #[test]
    fn new_result_rerun_has_same_key_set() {
        // Re-run shape: same fields, already_exists:true (WB0 stable key-set).
        let r = new::NewResult {
            intent_id: "intent-abc".to_string(),
            already_exists: true,
            campaign: "cli-porcelain".to_string(),
            agent: "main".to_string(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        // Both runs share the exact same key set.
        assert!(v.get("intent_id").is_some());
        assert!(v.get("already_exists").is_some());
        assert!(v.get("campaign").is_some());
        assert!(v.get("agent").is_some());
    }
}
