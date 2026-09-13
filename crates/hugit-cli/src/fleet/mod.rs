//! `hugit fleet` — machine-readable fleet state (Phase D, graduated from
//! RESERVED with REAL wiring — not a stub).
//!
//! Projects the canonical event log into the versioned [`FleetState`] schema —
//! workspaces + agents at their current state, derived purely from the
//! `ws.state.*` / `agent.assigned|completed|failed` events (one projection, no
//! second store). REUSES the engine's own [`hugit_ledger::FleetState`] fold, so
//! the fleet view AGREES with every other surface by construction.
//!
//! `hugit fleet --log <path>` emits the schema-valid `FleetState` JSON
//! (`schema_version`, `workspaces`, `agents`, `git_activity`,
//! `git_activity_count`, `last_seq`, `event_count`, `malformed`). Workspace/
//! agent/branch/target identifiers are redacted at the view boundary; malformed
//! payloads are COUNTED (`malformed`), never fabricated into "unknown" entries.
//! On a log with no fleet events the workspaces/agents are honestly empty —
//! never invented. `git_activity` carries the captured raw git trace
//! (`ref.update` / `ref.delete` from the silent hooks + the push path), one
//! entry per record in log order, so an operator sees what agents committed.
//!
//! Every output is stable JSON on stdout under the WB0 one-error/one-exit law
//! ([`crate::porcelain`]): `log_not_found` / `parse_log` are the canonical
//! `{"error":{…}}` envelopes, exit `2`.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::Value;

use hugit_ledger::FleetState;

use crate::checks::load_event_log;
use crate::porcelain::PorcelainError;

/// `hugit fleet` — emit the machine-readable fleet state (Phase D).
#[derive(clap::Args, Debug)]
pub struct FleetArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`) — the one
    /// `--log` seam every porcelain verb shares.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
}

/// Dispatch `hugit fleet`, emitting stable JSON on stdout and returning the
/// process exit code under the WB0 one-exit-code law.
pub fn run(args: FleetArgs) -> ExitCode {
    match project(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}

/// Project the fleet state from the canonical log.
fn project(args: &FleetArgs) -> Result<Value, PorcelainError> {
    let log_path = crate::log_resolve::resolve_log(args.log.clone());
    let log = load_event_log(&log_path)?;
    let projection = crate::projection::views::load(&log_path).map_err(|error| {
        PorcelainError::new(
            "projection_invalid",
            error,
            "repair projection status or re-run capture; views never invent derived state",
        )
    })?;
    let state = FleetState::from_records(log.records());
    // Honour the schema doc-claim ("validated on every emission") at the verb
    // boundary — a validation failure is an internal fault (exit 1), never a
    // partial/garbage body.
    state
        .validate()
        .map_err(|e| PorcelainError::internal(format!("fleet state invalid: {e}")))?;
    let mut value = serde_json::to_value(&state)
        .map_err(|e| PorcelainError::internal(format!("serialise fleet state: {e}")))?;
    value["projection"] = serde_json::to_value(projection)
        .map_err(|e| PorcelainError::internal(format!("serialise projection view: {e}")))?;
    Ok(value)
}
