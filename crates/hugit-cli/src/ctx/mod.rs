//! `hugit ctx` — short-horizon session resume (D11), graduated from RESERVED with
//! REAL wiring (not a stub).
//!
//! `hugit ctx resume --log <path> --workspace <id> --intent <id> [--tenant <id>]
//! [--now-ms <ms>]` reconstructs a crashed/replaced agent's session from the
//! `journal.note` records on the canonical event log, WITHIN the supported resume
//! horizon. It reuses the engine's OWN reconstruction
//! ([`hugit_ledger::journal::ctx_resume_from_journal`]) + horizon enforcement, so
//! the CLI and any future serve mirror agree by construction.
//!
//! # Honest refusal (D11③) — never a silent stale reconstruction
//! Beyond the horizon the verb REFUSES with the documented `beyond_horizon`
//! error; an empty binding REFUSES with `empty_journal`. These are the canonical
//! [`crate::porcelain`] envelopes (exit 2), never a fabricated context.
//!
//! # The scrubbed-binding join (the silent-miss guard)
//! `hugit note` scrubs `workspace_id`/`intent_id` through
//! [`crate::redaction::scrub`] BEFORE they land on the forever-log, so the stored
//! binding is the SCRUBBED form. This verb scrubs the caller's `--workspace` /
//! `--intent` the SAME way before matching — otherwise a secret-shaped binding
//! (stored as `[REDACTED]`) would never match the raw input and resume would
//! silently reconstruct nothing.
//!
//! # The time base (`--now-ms`) — an honest caveat
//! The horizon is `now_ms − last_note_recorded_at`. `hugit note` writes
//! its records with `recorded_at = 0` (the canonical forever-log is
//! clock-untrusted — time is not part of the content address), so against a
//! LOCALLY-written log the wall-clock default for `--now-ms` (~1.7e12 ms) is
//! always beyond the 7-day horizon and resume honestly REFUSES. To resume a local
//! log, pass `--now-ms` in the notes' own time base (e.g. a small value within
//! `DEFAULT_HORIZON_MS` of `0`). When the notes carry a REAL `recorded_at` (the
//! live runner/serve path), the wall-clock default is the right one. The verb
//! enforces the horizon faithfully either way — it never fabricates recency.
//!
//! # `ctx usage` — the authoring-`/usage` capture verb (WP-COST-2)
//! `ctx usage` records an authoring run's REAL token usage onto the canonical
//! log at authoring finish (a `ctx.usage` record), so land can price it
//! (`tokens × rate`) + submit on close. hugit records VERBATIM — it prices
//! nothing and calls no provider. See [`usage`] for the full contract.
//!
//! # Scope
//! `resume` + `usage` are REAL. `ctx snap` (the writer that persists a snapshot)
//! is GATED on the P2 DO/R2-backed `JournalStore` and is deliberately NOT offered
//! here (an honest partial surface, not a stub).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use serde_json::{Value, json};

use hugit_ledger::journal::{Journal, JournalKey, ResumeError, ctx_resume_from_journal};

use crate::checks::load_event_log;
use crate::journal::note::JOURNAL_NOTE_KIND;
use crate::porcelain::PorcelainError;
use crate::redaction::scrub;

pub mod usage;

pub use usage::{CTX_USAGE_KIND, UsageArgs};

/// `hugit ctx <subcommand>`.
#[derive(clap::Args, Debug)]
pub struct CtxArgs {
    #[command(subcommand)]
    pub command: CtxCommand,
}

/// Context subcommand surface (D11). `resume` + `usage` this wave — `snap` is
/// P2-gated.
#[derive(Subcommand, Debug)]
pub enum CtxCommand {
    /// Reconstruct a crashed/replaced session from its journal within the horizon.
    Resume(ResumeArgs),
    /// Record an authoring run's REAL token usage onto the canonical log
    /// (`ctx.usage`) at authoring finish — recorded verbatim, priced nowhere.
    Usage(UsageArgs),
}

/// `hugit ctx resume` — reconstruct a session from the log's `journal.note`s.
#[derive(clap::Args, Debug)]
pub struct ResumeArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// The workspace id binding to reconstruct (matched scrubbed — see module doc).
    #[arg(long)]
    pub workspace: String,
    /// The intent id binding to reconstruct.
    #[arg(long)]
    pub intent: String,
    /// The tenant id for the reconstructed key (not stored on the log; echoed in
    /// the result). Defaults to empty (no tenant bound on a local log).
    #[arg(long)]
    pub tenant: Option<String>,
    /// Override "now" (unix ms) for the horizon check — for deterministic tests
    /// and for resuming against a recorded clock. Defaults to the wall clock.
    #[arg(long)]
    pub now_ms: Option<u64>,
}

/// Dispatch `hugit ctx`.
pub fn run(args: CtxArgs) -> ExitCode {
    match args.command {
        CtxCommand::Resume(a) => resume_run(a),
        CtxCommand::Usage(a) => usage::run(a),
    }
}

/// Dispatch `hugit ctx resume` under the WB0 one-exit-code law.
fn resume_run(args: ResumeArgs) -> ExitCode {
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

/// Project the reconstructed context from the chain-verified log.
fn project(args: &ResumeArgs) -> Result<Value, PorcelainError> {
    let log_path = crate::log_resolve::resolve_log(args.log.clone());
    let log = load_event_log(&log_path)?;

    // Scrub the caller's binding the SAME way the writer did, so the match lands
    // on the stored (scrubbed) value rather than silently missing.
    let want_ws = scrub(&args.workspace);
    let want_intent = scrub(&args.intent);
    let tenant = args.tenant.clone().unwrap_or_default();

    let mut journal = Journal::new(JournalKey::new(
        tenant,
        want_ws.clone(),
        want_intent.clone(),
    ));

    // Fold the matching `journal.note` records (in log order) into the journal.
    for r in log.records() {
        if r.kind != JOURNAL_NOTE_KIND {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<Value>(&r.payload) else {
            continue; // a malformed note never fabricates an entry
        };
        let ws = payload.get("workspace_id").and_then(Value::as_str);
        let it = payload.get("intent_id").and_then(Value::as_str);
        if ws != Some(want_ws.as_str()) || it != Some(want_intent.as_str()) {
            continue;
        }
        let principal = payload
            .get("principal")
            .and_then(Value::as_str)
            .unwrap_or("");
        let note = payload.get("note").and_then(Value::as_str).unwrap_or("");
        journal.append(r.recorded_at, principal, note);
    }

    let now_ms = args.now_ms.unwrap_or_else(now_unix_ms);
    let ctx = ctx_resume_from_journal(&journal, now_ms).map_err(resume_error)?;

    // The stored note/principal are already scrubbed-on-write; entries serialize
    // verbatim (JournalEntry is Serialize). reconstructed:true marks a real fold.
    Ok(json!({
        "workspace_id": ctx.workspace_id,
        "intent_id": ctx.intent_id,
        "tenant_id": ctx.tenant_id,
        "entries": ctx.entries,
        "last_recorded_at": ctx.last_recorded_at,
        "horizon_ms": ctx.horizon_ms,
        "reconstructed": true,
    }))
}

/// Map a [`ResumeError`] to the canonical porcelain envelope — every variant is a
/// documented refusal (exit 2), never a silent stale reconstruction.
fn resume_error(e: ResumeError) -> PorcelainError {
    match e {
        ResumeError::EmptyJournal => PorcelainError::new(
            "empty_journal",
            "no journal.note entries match this workspace/intent binding on the log",
            "check --workspace/--intent match a recorded journal.note (record one with \
             `hugit note --workspace … --intent …`)",
        ),
        ResumeError::BeyondHorizon { age_ms, horizon_ms } => PorcelainError::new(
            "beyond_horizon",
            format!(
                "ctx resume refused: journal age {age_ms}ms exceeds the {horizon_ms}ms horizon \
                 (documented refusal — never a silent stale reconstruction)"
            ),
            "resume is only supported within the short horizon; start a fresh session instead",
        )
        .with_context("age_ms", json!(age_ms))
        .with_context("horizon_ms", json!(horizon_ms)),
        ResumeError::NotFound(je) => PorcelainError::new(
            "journal_not_found",
            format!("journal not found for the binding: {je}"),
            "check the --workspace/--intent/--tenant binding",
        ),
    }
}

/// Wall-clock unix epoch milliseconds (best-effort; `0` before the epoch).
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
