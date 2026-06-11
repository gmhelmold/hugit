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
//!   error, and a generic non-zero with a `hugit: error:` line on stderr for an
//!   I/O / parse failure of the `--log` seam itself.
//! - **D14 at the door** — `--author-kind` is constrained to `orchestrator |
//!   human` by [`AuthorKindArg`]'s value parser; `subagent` (or anything else)
//!   is rejected with the structured `subagent_author` error before any event
//!   is appended.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::EventLog;
use serde_json::Value;

use super::filelock::{self, FileLock, LockError};
use super::{
    AbandonArgs, AuthorKind, LandArgs, ListArgs, OpenArgs, PrError, ShowArgs, abandon, land, list,
    open, show,
};

/// `hugit pr <subcommand>` — the pull-request lifecycle (WP-PC3).
#[derive(clap::Args, Debug)]
pub struct PrArgs {
    #[command(subcommand)]
    pub command: PrCommand,
}

/// The pr subcommand surface — `open` / `land` / `show` / `list` / `abandon`.
#[derive(Subcommand, Debug)]
pub enum PrCommand {
    /// Open a PR: bundle intents → PROPOSED (D14 author authz at the door).
    Open(OpenCliArgs),
    /// Land a PR: enter the union-testing landing queue, report position.
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
    #[arg(long)]
    log: PathBuf,
    /// PR id (`--pr <id>`).
    #[arg(long = "pr")]
    pr_id: String,
    /// Campaign key the PR belongs to.
    #[arg(long)]
    campaign: String,
    /// Author kind (D14 at the door): `orchestrator` or `human`. `subagent` is
    /// rejected — a PR author is never a subagent.
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

/// `hugit pr land` flags.
#[derive(clap::Args, Debug)]
pub struct LandCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long)]
    log: PathBuf,
    /// PR id to land.
    #[arg(long = "pr")]
    pr_id: String,
    /// Unix-ms timestamp to stamp the appended `pr.queued` event with.
    #[arg(long = "recorded-at", default_value_t = 0)]
    recorded_at: u64,
}

/// `hugit pr show` flags.
#[derive(clap::Args, Debug)]
pub struct ShowCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long)]
    log: PathBuf,
    /// PR id to show.
    #[arg(long = "pr")]
    pr_id: String,
}

/// `hugit pr list` flags.
#[derive(clap::Args, Debug)]
pub struct ListCliArgs {
    /// Path to the JSON event log (a `[EventRecord, …]` array).
    #[arg(long)]
    log: PathBuf,
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
    #[arg(long)]
    log: PathBuf,
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
        PrCommand::Land(a) => run_land(a),
        PrCommand::Show(a) => run_show(a),
        PrCommand::List(a) => run_list(a),
        PrCommand::Abandon(a) => run_abandon(a),
    }
}

fn run_open(a: OpenCliArgs) -> ExitCode {
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
    let _lock = match acquire_lock(&a.log) {
        Ok(lock) => lock,
        Err(code) => return code,
    };

    // `open` is the one verb that may start from an absent log (it creates it).
    let mut log = match load_log_or_empty(&a.log) {
        Ok(log) => log,
        Err(code) => return code,
    };

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
        Ok(value) => match persist_log(&a.log, &log) {
            Ok(()) => emit_ok(&value),
            Err(code) => code,
        },
        Err(e) => emit_error(&e),
    }
}

fn run_land(a: LandCliArgs) -> ExitCode {
    let _lock = match acquire_lock(&a.log) {
        Ok(lock) => lock,
        Err(code) => return code,
    };
    let mut log = match load_log(&a.log) {
        Ok(log) => log,
        Err(code) => return code,
    };
    let land_args = LandArgs {
        pr_id: a.pr_id,
        recorded_at: a.recorded_at,
    };
    match land(&mut log, &land_args) {
        Ok(value) => match persist_log(&a.log, &log) {
            Ok(()) => emit_ok(&value),
            Err(code) => code,
        },
        Err(e) => emit_error(&e),
    }
}

fn run_show(a: ShowCliArgs) -> ExitCode {
    let log = match load_log(&a.log) {
        Ok(log) => log,
        Err(code) => return code,
    };
    match show(&log, &ShowArgs { pr_id: a.pr_id }) {
        Ok(value) => emit_ok(&value),
        Err(e) => emit_error(&e),
    }
}

fn run_list(a: ListCliArgs) -> ExitCode {
    let log = match load_log(&a.log) {
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
    let _lock = match acquire_lock(&a.log) {
        Ok(lock) => lock,
        Err(code) => return code,
    };
    let mut log = match load_log(&a.log) {
        Ok(log) => log,
        Err(code) => return code,
    };
    let abandon_args = AbandonArgs {
        pr_id: a.pr_id,
        reason: a.reason,
        recorded_at: a.recorded_at,
    };
    match abandon(&mut log, &abandon_args) {
        Ok(value) => match persist_log(&a.log, &log) {
            Ok(()) => emit_ok(&value),
            Err(code) => code,
        },
        Err(e) => emit_error(&e),
    }
}

// ── the `--log` seam ───────────────────────────────────────────────────────────

/// Load the event log from `path` (a JSON `[EventRecord, …]` array),
/// rehydrating the hash chain through [`EventLog::push_record`]. Returns the
/// generic-error exit code (with a `hugit: error:` line on stderr) on an I/O /
/// parse / chain-rehydrate failure — these are seam faults, not domain errors.
fn load_log(path: &PathBuf) -> Result<EventLog, ExitCode> {
    let bytes = std::fs::read(path).map_err(|e| io_fail(format!("read log {path:?}: {e}")))?;
    let records: Vec<EventRecord> =
        serde_json::from_slice(&bytes).map_err(|e| io_fail(format!("parse log {path:?}: {e}")))?;
    let mut log = EventLog::new();
    for record in records {
        log.push_record(record)
            .map_err(|e| io_fail(format!("rehydrate log {path:?}: {e}")))?;
    }
    Ok(log)
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
fn persist_log(path: &PathBuf, log: &EventLog) -> Result<(), ExitCode> {
    let json = serde_json::to_string_pretty(log.records())
        .map_err(|e| io_fail(format!("serialize log: {e}")))?;
    filelock::atomic_write(path, json.as_bytes()).map_err(|e| match e {
        LockError::Busy { .. } => emit_log_busy(path),
        LockError::Io { .. } => io_fail(format!("write log {path:?}: {e}")),
    })
}

/// Acquire the advisory exclusive lock for `path` (WP-WC1), to be held across a
/// mutating verb's load→mutate→persist. On a live holder, emits the structured
/// `log_busy` error on stdout (exit `2` — a retry-able domain condition); on an
/// I/O fault, the generic seam-fault path (`hugit: error:` on stderr, exit `1`).
fn acquire_lock(path: &PathBuf) -> Result<FileLock, ExitCode> {
    FileLock::acquire(path).map_err(|e| match e {
        LockError::Busy { .. } => emit_log_busy(path),
        LockError::Io { .. } => io_fail(format!("lock log {path:?}: {e}")),
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

/// Build the generic seam-fault exit code: a `hugit: error:` line on stderr and
/// a generic [`ExitCode::FAILURE`] (`1`) — distinct from the structured domain
/// error code (`2`), consistent with `main.rs`'s library-verb error path.
fn io_fail(msg: String) -> ExitCode {
    eprintln!("hugit: error: {msg}");
    ExitCode::FAILURE
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
        log.append("pr.opened", vec![], "{}".to_string(), 1);
        persist_log(&path, &log).expect("persist");

        let reloaded = load_log(&path).expect("reload");
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded.records()[0].kind, "pr.opened");
        let _ = std::fs::remove_file(&path);
    }
}
