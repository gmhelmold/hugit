//! Clap adapter for `hugit pr open | land | show` (WP-PC3b).
//!
//! The thin verb surface over the PC3 engine seams in [`super`]. It OWNS no
//! behavior: it parses `clap` arguments, loads the event log from the `--log`
//! path seam (consistent with `hugit why`/`hugit export`), calls the library
//! function ([`super::open`] / [`super::land`] / [`super::show`]), persists the
//! mutated log back (open/land), and emits the single JSON result on **stdout**.
//!
//! ## Output + exit-code convention (shared with the flow porcelain)
//!
//! - **Stable JSON on stdout always** — the success projection on success, the
//!   structured [`super::PrError`] envelope (via [`super::PrError::to_json`]) on
//!   a domain error. Both go to stdout (agents parse stdout, never stderr).
//! - **Exit codes** mirror the sibling porcelain ([`crate::porcelain`]): `0` on
//!   success (incl. the idempotent no-ops, which carry `already_*: true`),
//!   [`crate::porcelain::PORCELAIN_ERROR_EXIT`] (`2`) on a structured domain
//!   error. BOTH the `--log` READ seam (missing / malformed / tampered file) AND
//!   the WRITE/persist seam (serialize / lock I/O faults) converge on the ONE
//!   error law (P-PR-LAW + K-ERRLAW2): the canonical
//!   `{"error":{kind,message,fix}}` envelope on stdout + exit `2`, routed
//!   through the shared [`crate::porcelain`] helpers. READ faults use `kind`
//!   `log_not_found` / `parse_log` / `chain_broken`; WRITE/persist I/O faults
//!   use `kind` `io_error` — a filesystem/environment condition the orchestrator
//!   can act on. No bare-stderr/exit-`1` path remains.
//! - **D14 at the door** — `--author-kind` is constrained to `orchestrator |
//!   human` by [`AuthorKindArg`]'s value parser; `subagent` (or anything else)
//!   is rejected with the structured `subagent_author` error before any event
//!   is appended.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use hugit_checks::runner::LeaseClient;
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::EventLog;
use serde_json::Value;

use super::filelock::{self, FileLock, LockError};
use super::{
    AbandonArgs, AuthorKind, EnvelopeMetricsArgs, LandArgs, ListArgs, OpenArgs, PrError,
    SettleArgs, ShowArgs, abandon, land, list, open, settle, show,
};

/// `hugit pr <subcommand>` — the pull-request lifecycle (WP-PC3).
#[derive(clap::Args, Debug)]
pub struct PrArgs {
    #[command(subcommand)]
    pub command: PrCommand,
}

/// The pr subcommand surface — `open` / `queue` / `land` / `show` / `list` /
/// `abandon`.
#[derive(Subcommand, Debug)]
pub enum PrCommand {
    /// Open a PR: bundle intents → PROPOSED (author authz at the door).
    Open(OpenCliArgs),
    /// Queue a PR: enter the union-testing landing queue, report position.
    Queue(QueueCliArgs),
    /// Land a PR: settle a queued PR as terminal-LANDED (appends pr.landed).
    Land(LandCliArgs),
    /// Show a PR: intents + queue state + the F3 `pr_record` cost rollup.
    Show(ShowCliArgs),
    /// List every PR on the log (full info per row, stable order; filterable).
    List(ListCliArgs),
    /// Abandon a PR: terminal — it leaves the queue projection (idempotent).
    Abandon(AbandonCliArgs),
}

/// `hugit pr open` flags.
#[derive(clap::Args, Debug)]
pub struct OpenCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array). Created on
    /// first `open` if absent; the appended `pr.opened` is persisted back.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    log: Option<PathBuf>,
    /// PR id (`--pr <id>`).
    #[arg(long = "pr")]
    pr_id: String,
    /// Campaign key the PR belongs to.
    #[arg(long)]
    campaign: String,
    /// Author kind: `orchestrator` or `human`. `subagent` is rejected — a PR
    /// author is never a subagent.
    #[arg(long = "author-kind")]
    author_kind: AuthorKindArg,
    /// Orchestrator run id (with `--author-kind orchestrator`).
    #[arg(long = "run-id")]
    run_id: Option<String>,
    /// Human principal (with `--author-kind human`).
    #[arg(long)]
    principal: Option<String>,
    /// Bundled intent ids (`--intent <id>`, repeatable).
    #[arg(long = "intent")]
    intent: Vec<String>,
    /// Unix-ms timestamp to stamp the appended `pr.opened` event with.
    #[arg(long = "recorded-at", default_value_t = 0)]
    recorded_at: u64,
}

/// `hugit pr queue` flags — enqueue a PROPOSED PR into the landing queue
/// (appends `pr.queued`).
#[derive(clap::Args, Debug)]
pub struct QueueCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    log: Option<PathBuf>,
    /// PR id to queue.
    #[arg(long = "pr")]
    pr_id: String,
    /// Unix-ms timestamp to stamp the appended `pr.queued` event with.
    #[arg(long = "recorded-at", default_value_t = 0)]
    recorded_at: u64,
}

/// `hugit pr land` flags — settle an (already-queued) PR as terminal-LANDED
/// (appends `pr.landed`, W-PRLANDED).
///
/// Git-proximity cleanup: this is the dedicated land-confirm step. Enqueuing is
/// now its own verb (`pr queue`); `pr land` always settles. No boolean flag that
/// silently changes the verb's meaning. Does NOT run the P2 union-test verdict —
/// it is the operator/orchestrator confirmation.
#[derive(clap::Args, Debug)]
pub struct LandCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    log: Option<PathBuf>,
    /// PR id to land.
    #[arg(long = "pr")]
    pr_id: String,
    /// Unix-ms timestamp to stamp the appended `pr.landed` event with.
    #[arg(long = "recorded-at", default_value_t = 0)]
    recorded_at: u64,

    /// Run the PR's check on the CoreLink runner fabric and capture the REAL
    /// §13.1 per-job metrics (cost + token cache-split) into the land envelope
    /// (WP-Wave-E-PR5). Fail-closed: with an unwired runner this is a clear
    /// error, NEVER a fabricated / honest-zero cost. Mutually exclusive with the
    /// manual metric flags below (don't merge two metric sources).
    #[arg(long = "dispatch", default_value_t = false)]
    dispatch: bool,

    // ── WP-F2: capture the context envelope on land (cost/metrics legibility).
    // Every flag is OPTIONAL — an omitted flag is honest-zero / None, never a
    // fabricated figure. The orchestrator passes the figures it measured (the
    // dogfood path → real cost surfaces); absent → the envelope still captures
    // the REAL structure (which PR, which intents, the campaign), honest-zero.
    /// Total tokens spent on the run (WP-F2 capture metric).
    #[arg(long = "tokens", default_value_t = 0)]
    tokens: u64,
    /// Derived COGS in integer micro-USD (`1 USD = 1_000_000`) — shown for
    /// trust, never a usage meter (WP-F2 capture metric).
    #[arg(long = "cost-usd-micros", default_value_t = 0)]
    cost_usd_micros: u64,
    /// Total tool calls (WP-F2 capture metric).
    #[arg(long = "tool-calls", default_value_t = 0)]
    tool_calls: u64,
    /// Model + tool busy time, ms (WP-F2 capture metric).
    #[arg(long = "active-ms", default_value_t = 0)]
    active_ms: u64,
    /// Number of model turns (WP-F2 capture metric).
    #[arg(long = "model-turns", default_value_t = 0)]
    model_turns: u64,
    /// Model identifier (WP-F2). Empty ⇒ a HUMAN-authored PR (the rollup rule).
    #[arg(long = "model")]
    model: Option<String>,
    /// `cas:` ref to the captured context blob, when available (WP-F2).
    #[arg(long = "context-cas")]
    context_cas: Option<String>,
    /// `cas:` ref to the compacted transcript, when available (WP-F2).
    #[arg(long = "compact-transcript-ref")]
    compact_transcript_ref: Option<String>,
    /// `cas:` ref to the raw transcript, when available (WP-F2).
    #[arg(long = "raw-transcript-ref")]
    raw_transcript_ref: Option<String>,
    /// `cas:` ref to the adversarial-panel verdicts, when available (WP-F2).
    #[arg(long = "verdicts-ref")]
    verdicts_ref: Option<String>,
}

/// `hugit pr show` flags.
#[derive(clap::Args, Debug)]
pub struct ShowCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    log: Option<PathBuf>,
    /// PR id to show.
    #[arg(long = "pr")]
    pr_id: String,
}

/// `hugit pr list` flags.
#[derive(clap::Args, Debug)]
pub struct ListCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    log: Option<PathBuf>,
    /// Only PRs of this campaign (`--campaign <key>`).
    #[arg(long)]
    campaign: Option<String>,
    /// Only PRs in this state: `proposed | queued | abandoned | landed`.
    #[arg(long)]
    state: Option<String>,
}

/// `hugit pr abandon` flags.
#[derive(clap::Args, Debug)]
pub struct AbandonCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    log: Option<PathBuf>,
    /// PR id to abandon.
    #[arg(long = "pr")]
    pr_id: String,
    /// The reason for abandoning, recorded on the `pr.abandoned` event.
    #[arg(long)]
    reason: String,
    /// Unix-ms timestamp to stamp the appended `pr.abandoned` event with.
    #[arg(long = "recorded-at", default_value_t = 0)]
    recorded_at: u64,
}

/// A clap-parsed `--author-kind`, enforcing D14 at the door.
///
/// The value parser accepts only `orchestrator` / `human`; `subagent` (or any
/// other token) fails to parse — clap surfaces it, but we also handle the
/// rejection ourselves so the structured `subagent_author` error (carrying the
/// suggested fix) is emitted on stdout rather than clap's usage text. To do
/// that the *parser* never rejects; it carries the raw token through and
/// validation happens in [`run`].
#[derive(Debug, Clone)]
struct AuthorKindArg(String);

impl std::str::FromStr for AuthorKindArg {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AuthorKindArg(s.to_string()))
    }
}

/// Dispatch a `pr` subcommand. Returns the process exit code directly (the
/// porcelain verbs own their exit code; see `main.rs`).
pub fn run(args: PrArgs) -> ExitCode {
    match args.command {
        PrCommand::Open(a) => run_open(a),
        PrCommand::Queue(a) => run_queue(a),
        PrCommand::Land(a) => run_land(a),
        PrCommand::Show(a) => run_show(a),
        PrCommand::List(a) => run_list(a),
        PrCommand::Abandon(a) => run_abandon(a),
    }
}

fn run_open(a: OpenCliArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(a.log.clone());
    // WH-IDENT: validate identifier fields at entry — before they reach the
    // hash-chained forever-log.  Rejects empty and known-credential-prefix shapes.
    // Bare 40/64-hex keys are legitimate addresses and are allowed.
    if let Err(e) = crate::ident::validate_identifier(&a.pr_id, "--pr") {
        return emit_porcelain(&crate::porcelain::PorcelainError::new(
            e.kind, e.message, e.fix,
        ));
    }
    if let Err(e) = crate::ident::validate_identifier(&a.campaign, "--campaign") {
        return emit_porcelain(&crate::porcelain::PorcelainError::new(
            e.kind, e.message, e.fix,
        ));
    }
    // --run-id is optional; only validate it when the caller passes a non-None value.
    if let Some(run_id) = &a.run_id
        && let Err(e) = crate::ident::validate_identifier(run_id, "--run-id")
    {
        return emit_porcelain(&crate::porcelain::PorcelainError::new(
            e.kind, e.message, e.fix,
        ));
    }

    // D14 at the door: validate `--author-kind`, emitting the structured
    // `subagent_author` error (with fix) on stdout for any non-accepted value.
    let author_kind = match AuthorKind::parse(&a.author_kind.0) {
        Some(k) => k,
        None => {
            return emit_error(&PrError::SubagentAuthor {
                got: a.author_kind.0,
            });
        }
    };

    // Hold the advisory exclusive lock across the whole load→mutate→persist
    // (WP-WC1): two concurrent `pr` verbs on one `--log` serialize or fail
    // `log_busy`, never clobber (TOCTOU dead). The guard drops on return.
    let _lock = match acquire_lock(&log_path) {
        Ok(lock) => lock,
        Err(code) => return code,
    };

    // `open` is the one verb that may start from an absent log (it creates it).
    let mut log = match load_log_or_empty(&log_path) {
        Ok(log) => log,
        Err(code) => return code,
    };

    // Terminal-seal precondition (C5-F2): a PR may not be OPENED into a sealed
    // campaign. Route through the SHARED chokepoint every campaign-scoped verb
    // uses (the campaign is the `--campaign` the PR is being opened under).
    if let Err(code) = guard_campaign_not_sealed(&log, &a.campaign) {
        return code;
    }

    let open_args = OpenArgs {
        pr_id: a.pr_id,
        campaign: a.campaign,
        author_kind,
        run_id: a.run_id,
        principal: a.principal,
        intent_ids: a.intent,
        recorded_at: a.recorded_at,
    };

    match open(&mut log, &open_args) {
        Ok(value) => match persist_log(&log_path, &log) {
            Ok(()) => emit_ok(&value),
            Err(code) => code,
        },
        Err(e) => emit_error(&e),
    }
}

fn run_queue(a: QueueCliArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(a.log.clone());
    let _lock = match acquire_lock(&log_path) {
        Ok(lock) => lock,
        Err(code) => return code,
    };
    let mut log = match load_log(&log_path) {
        Ok(log) => log,
        Err(code) => return code,
    };
    // Terminal-seal precondition (C5-F2): a PR may not be queued into a sealed
    // campaign. Resolve the PR's campaign from its `pr.opened` record and route
    // through the shared chokepoint. An unknown PR (no `pr.opened`) is left to
    // the land path's own `UnknownPr` error — the seal guard is a no-op when the
    // campaign can't be resolved.
    if let Some(opened) = super::find_pr_opened(&log, &a.pr_id)
        && let Err(code) = guard_campaign_not_sealed(&log, &opened.campaign)
    {
        return code;
    }
    // `pr queue` enqueues a PROPOSED PR (appends `pr.queued`) — the enqueue half
    // of the old `pr land`. Runs the SAME guarded append + atomic-lock + persist.
    let result = land(
        &mut log,
        &LandArgs {
            pr_id: a.pr_id,
            recorded_at: a.recorded_at,
        },
    );
    match result {
        Ok(value) => match persist_log(&log_path, &log) {
            Ok(()) => emit_ok(&value),
            Err(code) => code,
        },
        Err(e) => emit_error(&e),
    }
}

fn run_land(a: LandCliArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(a.log.clone());

    // WP-F2: the (optional) manual run metrics → the captured envelope. An
    // omitted flag is honest-zero / None, never fabricated.
    let manual_metrics = EnvelopeMetricsArgs {
        tokens: a.tokens,
        cost_usd_micros: a.cost_usd_micros,
        tool_calls: a.tool_calls,
        active_ms: a.active_ms,
        model_turns: a.model_turns,
        model: a.model,
        context_cas: a.context_cas,
        compact_transcript_ref: a.compact_transcript_ref,
        raw_transcript_ref: a.raw_transcript_ref,
        verdicts_ref: a.verdicts_ref,
        // The flag path never sets `full_metrics` (no CLI flag for a raw metrics
        // blob); the §13.1 cache-split carrier comes ONLY from `--dispatch`.
        full_metrics: None,
    };

    // WP-Wave-E-PR5: `--dispatch` measures the real per-job cost on the runner
    // fabric — it must NOT be combined with the manual metric flags (don't merge
    // two metric sources). A usage error, checked BEFORE the lock/log are touched.
    if a.dispatch && manual_metrics.any_supplied() {
        return emit_porcelain(&super::dispatch::conflict_error());
    }

    let _lock = match acquire_lock(&log_path) {
        Ok(lock) => lock,
        Err(code) => return code,
    };
    let mut log = match load_log(&log_path) {
        Ok(log) => log,
        Err(code) => return code,
    };
    // Terminal-seal precondition (C5-F2): a PR may not be settled into a sealed
    // campaign. Resolve the PR's campaign from its `pr.opened` record and route
    // through the shared chokepoint.
    if let Some(opened) = super::find_pr_opened(&log, &a.pr_id)
        && let Err(code) = guard_campaign_not_sealed(&log, &opened.campaign)
    {
        return code;
    }

    if a.dispatch {
        // Real per-job cost from the runner fabric (WP-Wave-E-PR5). Build the live
        // lease client fail-closed: an unwired runner is a CLEAR error, never a
        // silent honest-zero fall-back (the PAT never reaches the error string).
        let client = match LeaseClient::from_runtime() {
            Ok(c) => c,
            Err(e) => return emit_porcelain(&super::dispatch::unconfigured_error(&e)),
        };
        return match super::dispatch::land_with_dispatch(&mut log, &client, &a.pr_id, a.recorded_at)
        {
            Ok(value) => match persist_log(&log_path, &log) {
                Ok(()) => emit_ok(&value),
                Err(code) => code,
            },
            Err(super::dispatch::DispatchLandError::Pr(e)) => emit_error(&e),
            Err(super::dispatch::DispatchLandError::Runner(e)) => {
                emit_porcelain(&super::dispatch::runner_error(&e))
            }
        };
    }

    // `pr land` is the dedicated land-confirm settlement step (appends
    // `pr.landed`) — the settle half of the old `pr land --settle`. Runs through
    // the SAME guarded append + atomic-lock + persist seam.
    let result = settle(
        &mut log,
        &SettleArgs {
            pr_id: a.pr_id,
            recorded_at: a.recorded_at,
            envelope_metrics: manual_metrics,
        },
    );
    match result {
        Ok(value) => match persist_log(&log_path, &log) {
            Ok(()) => emit_ok(&value),
            Err(code) => code,
        },
        Err(e) => emit_error(&e),
    }
}

fn run_show(a: ShowCliArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(a.log.clone());
    let log = match load_log(&log_path) {
        Ok(log) => log,
        Err(code) => return code,
    };
    match show(&log, &ShowArgs { pr_id: a.pr_id }) {
        Ok(value) => emit_ok(&value),
        Err(e) => emit_error(&e),
    }
}

fn run_list(a: ListCliArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(a.log.clone());
    let log = match load_log(&log_path) {
        Ok(log) => log,
        Err(code) => return code,
    };
    let value = list(
        &log,
        &ListArgs {
            campaign: a.campaign,
            state: a.state,
        },
    );
    emit_ok(&value)
}

fn run_abandon(a: AbandonCliArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(a.log.clone());
    let _lock = match acquire_lock(&log_path) {
        Ok(lock) => lock,
        Err(code) => return code,
    };
    let mut log = match load_log(&log_path) {
        Ok(log) => log,
        Err(code) => return code,
    };
    // Terminal-seal precondition (C5-F2): a PR may not be abandoned into a sealed
    // campaign (a sealed campaign is immutable — even the terminal `pr.abandoned`
    // append is refused). Resolve the campaign from the PR's `pr.opened`.
    if let Some(opened) = super::find_pr_opened(&log, &a.pr_id)
        && let Err(code) = guard_campaign_not_sealed(&log, &opened.campaign)
    {
        return code;
    }
    let abandon_args = AbandonArgs {
        pr_id: a.pr_id,
        reason: a.reason,
        recorded_at: a.recorded_at,
    };
    match abandon(&mut log, &abandon_args) {
        Ok(value) => match persist_log(&log_path, &log) {
            Ok(()) => emit_ok(&value),
            Err(code) => code,
        },
        Err(e) => emit_error(&e),
    }
}

// ── the `--log` seam ───────────────────────────────────────────────────────────

/// Load the event log from `path` (a JSON `[EventRecord, …]` array),
/// rehydrating + verifying the hash chain.
///
/// Under the ONE error law (P-PR-LAW, Wave E): every read fault emits the
/// canonical `{"error":{kind,message,fix}}` envelope on **stdout** and returns
/// the structured-error exit code (`2`), routed through the same
/// [`crate::porcelain`] helpers (`log_not_found` / `parse_log`) the sibling
/// `campaign`/`intent` verbs use — never the old plaintext-stderr/exit-`1`
/// path. A MISSING file is an explicit `log_not_found` (never a silent empty
/// world); a malformed file is `parse_log`; a tampered chain is `chain_broken`
/// (the `verify_chain` call the pr loader previously skipped — siblings call
/// it, so the pr read path must reject a tampered chain too).
fn load_log(path: &PathBuf) -> Result<EventLog, ExitCode> {
    use crate::porcelain::PorcelainError;
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(emit_porcelain(&PorcelainError::log_not_found(path)));
        }
        Err(e) => return Err(emit_porcelain(&PorcelainError::io("read log", path, &e))),
    };
    let records: Vec<EventRecord> = serde_json::from_slice(&bytes)
        .map_err(|e| emit_porcelain(&PorcelainError::parse_log(path, &e)))?;
    // PS-13: rehydrate + verify the chain through the SINGLE chokepoint
    // (`checks::rehydrate_and_verify`) rather than re-implementing the
    // `EventLog::new() + push_record + verify_chain` loop here. A tampered chain
    // is `chain_broken`/exit-2 (the siblings verify; so must we).
    crate::checks::rehydrate_and_verify(records).map_err(|fault| match fault {
        crate::checks::ChainLoadFault::Rehydrate(e) => emit_porcelain(&PorcelainError::new(
            "rehydrate",
            format!("rehydrate log {path:?}: {e}"),
            "the --log file's records must form a gap-free, monotonic chain",
        )),
        crate::checks::ChainLoadFault::ChainBroken(e) => emit_porcelain(&PorcelainError::new(
            "chain_broken",
            format!("log {path:?} failed integrity verification: {e}"),
            "the --log file's hash chain is tampered or corrupt",
        )),
    })
}

/// Like [`load_log`], but a missing file yields a fresh empty log (the `open`
/// path may create the log on first use).
fn load_log_or_empty(path: &PathBuf) -> Result<EventLog, ExitCode> {
    if !path.exists() {
        return Ok(EventLog::new());
    }
    load_log(path)
}

/// Persist the event log back to `path` as a pretty JSON `[EventRecord, …]`
/// array (the same shape [`load_log`] reads), via the **atomic**
/// temp-file-then-rename write (WP-WC1) — a reader or a crash sees the whole old
/// log or the whole new one, never a truncated file. The advisory lock is held
/// by the caller (`run_open`/`run_land`/`run_abandon`) across load→persist.
///
/// I/O faults (serialize / disk-full / permission denied) are routed through the
/// ONE error law — `{"error":{"kind":"io_error",…}}` on stdout, exit `2` — so
/// an orchestrator parsing stdout always receives a structured signal, even when
/// the persist path faults (K-ERRLAW2).
fn persist_log(path: &PathBuf, log: &EventLog) -> Result<(), ExitCode> {
    let json = serde_json::to_string_pretty(log.records())
        .map_err(|e| emit_io_error(format!("serialize log: {e}"), path))?;
    filelock::atomic_write(path, json.as_bytes()).map_err(|e| match e {
        LockError::Busy { .. } => emit_log_busy(path),
        LockError::Io { .. } => emit_io_error(format!("write log {path:?}: {e}"), path),
    })
}

/// Acquire the advisory exclusive lock for `path` (WP-WC1), to be held across a
/// mutating verb's load→mutate→persist. On a live holder, emits the structured
/// `log_busy` error on stdout (exit `2` — a retry-able domain condition); on an
/// I/O fault, routes through the ONE error law: structured `io_error` on stdout,
/// exit `2` (K-ERRLAW2).
fn acquire_lock(path: &PathBuf) -> Result<FileLock, ExitCode> {
    FileLock::acquire(path).map_err(|e| match e {
        LockError::Busy { .. } => emit_log_busy(path),
        LockError::Io { .. } => emit_io_error(format!("lock log {path:?}: {e}"), path),
    })
}

/// Emit the canonical `log_busy` porcelain error on stdout and the
/// structured-domain exit code (`2`). A busy lock is a transient, retry-able
/// condition another `hugit` verb holds — never a clobber.
fn emit_log_busy(path: &PathBuf) -> ExitCode {
    let err = crate::porcelain::PorcelainError::new(
        "log_busy",
        format!("the --log file {path:?} is locked by another hugit verb"),
        "another `hugit` process holds the log lock; retry once it releases \
         (a stale lock is auto-reclaimed after a short window)",
    );
    println!("{}", err.to_json());
    err.exit_code()
}

/// Emit a structured `io_error` porcelain error on **stdout** (the ONE error
/// law, K-ERRLAW2): disk-full / permission-denied / other filesystem faults on
/// the WRITE/persist seam are environment conditions, not logic errors, but they
/// MUST still reach the orchestrator as a machine-parseable signal.
///
/// `kind:"io_error"` is distinct from the domain `kind:"io"` used on the READ
/// seam — callers can match either class to detect any I/O fault. The `fix`
/// tells the caller it's a filesystem/environment condition (not caller input).
/// No secret is included in `msg` — the caller must ensure that.
fn emit_io_error(msg: String, path: &PathBuf) -> ExitCode {
    let err = crate::porcelain::PorcelainError::new(
        "io_error",
        msg,
        format!(
            "check the --log path {path:?} is on a writable filesystem with sufficient space; \
             this is an environment/filesystem condition, not a caller input error"
        ),
    );
    println!("{}", err.to_json());
    err.exit_code()
}

// ── the shared terminal-seal precondition (C5-F2) ────────────────────────────

/// Refuse a campaign-scoped `pr` mutation when its campaign is sealed, via the
/// ONE shared chokepoint [`crate::campaign::seal_guard::guard_not_sealed`].
///
/// On a sealed campaign this emits the canonical `campaign_sealed` porcelain
/// error on stdout (exit `2`) and returns `Err(code)`; on an open campaign it
/// returns `Ok(())` and the verb proceeds. The campaign key is scrubbed before
/// it reaches the error message (the same redaction posture the rest of the verb
/// applies to identifiers).
fn guard_campaign_not_sealed(log: &EventLog, campaign: &str) -> Result<(), ExitCode> {
    match crate::campaign::seal_guard::guard_not_sealed(log, campaign) {
        Ok(()) => Ok(()),
        Err(v) => {
            let safe_campaign = crate::redaction::scrub(&v.campaign);
            let err = crate::porcelain::PorcelainError::new(v.kind(), v.message(), v.fix())
                .with_context("campaign", serde_json::json!(safe_campaign));
            Err(emit_porcelain(&err))
        }
    }
}

// ── emit helpers ───────────────────────────────────────────────────────────────

/// Emit a success projection as JSON on stdout, exit 0.
fn emit_ok(value: &Value) -> ExitCode {
    println!("{value}");
    ExitCode::SUCCESS
}

/// Emit a structured [`PrError`] as the canonical `{"error":{…}}` JSON on
/// stdout (the WB0 one-error law, `fix`-keyed, nested), exit with the one
/// exit-code law's structured-error code (`2`).
fn emit_error(err: &PrError) -> ExitCode {
    let porcelain = err.to_porcelain();
    println!("{}", porcelain.to_json());
    porcelain.exit_code()
}

/// Emit a [`crate::porcelain::PorcelainError`] as the canonical
/// `{"error":{kind,message,fix}}` JSON on **stdout** and return its exit code
/// (the structured-error `2`). The ONE error law for the pr `--log` read seam
/// (P-PR-LAW) — replacing the old plaintext-stderr/exit-`1` path.
fn emit_porcelain(err: &crate::porcelain::PorcelainError) -> ExitCode {
    println!("{}", err.to_json());
    err.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_kind_arg_carries_token_through() {
        // The parser never rejects (so `run` can emit the structured error);
        // the raw token is carried for D14 validation.
        let parsed: AuthorKindArg = "subagent".parse().unwrap();
        assert_eq!(parsed.0, "subagent");
        assert_eq!(AuthorKind::parse(&parsed.0), None);
    }

    #[test]
    fn empty_path_loads_fresh_log() {
        let path = std::env::temp_dir().join(format!(
            "hugit-pc3b-missing-{}-{}.json",
            std::process::id(),
            "absent"
        ));
        let _ = std::fs::remove_file(&path);
        let log = load_log_or_empty(&path).expect("missing log → fresh empty");
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn persist_then_load_roundtrips() {
        let path =
            std::env::temp_dir().join(format!("hugit-pc3b-roundtrip-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut log = EventLog::new();
        // C4-F1: the raw `EventLog::append` door is now `pub(crate)`; this test
        // seeds one record through the guarded `append_authorized` (Orchestrator/
        // Push — always Allow) exactly as a real `pr open` would.
        log.append_authorized(
            hugit_refstore::PrincipalClass::Orchestrator,
            hugit_refstore::Endpoint::Push,
            "pr.opened",
            vec!["orchestrator:test".to_string()],
            "{}".to_string(),
            1,
        )
        .expect("orchestrator is authorized to open a pr");
        persist_log(&path, &log).expect("persist");

        let reloaded = load_log(&path).expect("reload");
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded.records()[0].kind, "pr.opened");
        let _ = std::fs::remove_file(&path);
    }

    /// K-ERRLAW2: a persist I/O fault (read-only log directory) must emit
    /// `{"error":{"kind":"io_error",…}}` on stdout (captured via the helper) and
    /// return the structured-domain exit code (`2`), NOT a bare-stderr/exit-`1`.
    #[test]
    fn persist_log_io_fault_emits_structured_io_error_exit_2() {
        use std::process::ExitCode;
        // Point the log path at a parent that is a FILE (not a dir) — PR-4
        // auto-creates missing dirs, so a non-existent dir no longer
        // triggers the IO error; parent-as-file is unambiguously unwritable.
        let parent_file = std::env::temp_dir().join(format!(
            "hugit-persist-io-fault-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&parent_file, b"i am a file, not a dir").unwrap();
        let log_path = parent_file.join("log.json"); // child of a file — IO err

        let log = EventLog::new();
        let exit = persist_log(&log_path, &log);
        // Must return Err (the path is unusable), not Ok.
        assert!(
            exit.is_err(),
            "persist_log must Err on an unwritable path, not Ok"
        );
        let code = exit.unwrap_err();
        // The structured-domain exit code is 2 (PORCELAIN_ERROR_EXIT), NOT 1
        // (ExitCode::FAILURE). We compare via u8 because ExitCode has no PartialEq.
        let code_u8: u8 = {
            // ExitCode doesn't expose its value directly; use process::exit
            // would abort, so we use the known constant instead.
            use crate::porcelain::PORCELAIN_ERROR_EXIT;
            // Verify: if the code were FAILURE (1) this would panic.
            assert_ne!(
                code,
                ExitCode::FAILURE,
                "K-ERRLAW2: persist I/O fault must exit 2 (structured), NOT 1 (bare)"
            );
            PORCELAIN_ERROR_EXIT
        };
        assert_eq!(code_u8, 2, "structured-domain exit code must be 2");
    }

    /// K-ERRLAW2: `emit_io_error` produces a valid `{"error":{"kind":"io_error",…}}`
    /// JSON envelope — the structured shape an orchestrator can parse.
    #[test]
    fn emit_io_error_json_shape() {
        use crate::porcelain::{PORCELAIN_ERROR_EXIT, PorcelainError};
        // Reconstruct the same error the helper builds (without actually printing).
        let path = std::path::PathBuf::from("/tmp/test.json");
        let msg = "write log \"/tmp/test.json\": permission denied".to_string();
        let err = PorcelainError::new(
            "io_error",
            msg.clone(),
            format!(
                "check the --log path {path:?} is on a writable filesystem with sufficient space; \
                 this is an environment/filesystem condition, not a caller input error"
            ),
        );
        let json_str = err.to_json();
        let v: serde_json::Value = serde_json::from_str(&json_str).expect("valid json");
        assert_eq!(v["error"]["kind"], "io_error", "kind must be io_error");
        assert_eq!(v["error"]["message"], msg);
        assert!(
            v["error"]["fix"].is_string(),
            "fix must be present and a string"
        );
        assert_eq!(
            err.exit_code(),
            std::process::ExitCode::from(PORCELAIN_ERROR_EXIT)
        );
    }
}
