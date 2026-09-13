//! Dock coinage — the hook-born physical binding (WP-DOCK-1).
//!
//! The dock is the physical unit of agent work: it is coined (created) at the
//! moment the work is born — a worktree/clone checkout (post-checkout hook) —
//! and carries an identity stable across re-branch / detached-HEAD / path
//! re-creation: the **gitdir** (`.git/worktrees/<name>` or the repo's `.git`).
//!
//! ## The shape (FROZEN here — WP-DOCK-1 owns it; every other dock WP consumes)
//!
//! A dock is a `dock.record` appended to the canonical `.hugit/log.json`
//! (the SAME hash-chained EventLog the hooks/capture use), carrying:
//!
//! ```json
//! {
//!   "dock_id": "<sha256(gitdir\0branch)>",
//!   "gitdir": "<absolute gitdir path>",
//!   "branch": "feat/rate-limit",
//!   "charter": "add rate limit",        // derived from the branch name
//!   "charter_derived": true,             // ALWAYS true at coinage
//!   "state": "open",
//!   "origin": "worktree" | "repo",
//!   "created_ts": <ms>,
//!   "pid": <coining process pid>,
//!   "parent_id": "<env dock if it differed>"   // R2 — never silent divergence
//! }
//! ```
//!
//! The marker file `.git/worktrees/<name>/hugit-dock` (or `<gitdir>/hugit-dock`
//! for the main repo) is the on-disk "this worktree has a dock" stamp, carrying
//! `{created_ts, pid}` — the A3 reborn-detection key (a re-created worktree on
//! the same path gets a NEW ts ⇒ new dock, never a re-open).
//!
//! ## The silent contract (inherited from capture — never blocks git)
//!
//! - `hugit dock coin` exits 0 ALWAYS. Any error goes to the best-effort hooks
//!   log via `--hook-log`; a hook failure NEVER fails/blocks the git operation.
//! - Append under the same `FileLock` + retry discipline as capture (concurrent
//!   hooks serialize; B4 closed).
//! - Coinage is idempotent: an existing marker ⇒ no-op (S1 — at most ONE open
//!   dock per (gitdir, branch)).
//! - R2: if `HUGIT_DOCK_ID` is set and its gitdir differs from the cwd gitdir,
//!   the env dock is NOT adopted — `parent_id` is recorded + a warning is
//!   written to the hooks log (never a silent divergence; never a fail).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

pub mod attest;
pub mod close;
pub mod insights;
pub mod land;
pub mod reconcile;
pub mod resolve;
pub mod spool;

use crate::checks::load_event_log;
use crate::pr::filelock::FileLock;

use hugit_refstore::authz::{Endpoint, PrincipalClass};
use hugit_refstore::canonical_json;

/// The recorder identity for dock coinage (hook-born).
const DOCK_PRINCIPAL: &str = "orchestrator:hugit-hook";

/// The frozen on-wire event kind for a dock.
pub const DOCK_RECORD_KIND: &str = "dock.record";

/// The marker filename inside the gitdir ("this worktree has a dock").
pub const DOCK_MARKER: &str = "hugit-dock";

/// Scrub user-controlled strings before a dock read model crosses stdout.
/// Canonical records stay unchanged so resolution and attribution retain raw keys.
pub(crate) fn sanitized_view(mut value: Value) -> Value {
    crate::porcelain::scrub_payload(&mut value);
    value
}

/// Milliseconds now — the wall-clock stamp.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Best-effort hooks log (never fails the verb — silent by contract).
fn write_hook_log(_top_level: &Path, path: Option<&Path>, message: &str) {
    let Some(p) = path else { return };
    let mut line = message.to_string();
    if !line.ends_with('\n') {
        line.push('\n');
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

/// `sha256(s) |> hex` — the dock id is a content hash of (gitdir, branch).
fn dock_id(gitdir: &str, branch: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(gitdir.as_bytes());
    h.update([0]);
    h.update(branch.as_bytes());
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Derive a human-ish charter from the branch name.
///
/// `feat/rate-limit` → `add rate limit`; `fix/BA-123` → `fix ba 123`. ALWAYS
/// marked `derived` (a real charter can override later).
fn derive_charter(branch: &str) -> String {
    let mut rest = branch;
    let verb = if let Some(r) = branch
        .strip_prefix("feat/")
        .or_else(|| branch.strip_prefix("feature/"))
    {
        rest = r;
        "add"
    } else if let Some(r) = branch
        .strip_prefix("fix/")
        .or_else(|| branch.strip_prefix("bugfix/"))
        .or_else(|| branch.strip_prefix("hotfix/"))
    {
        rest = r;
        "fix"
    } else if let Some(r) = branch.strip_prefix("refactor/") {
        rest = r;
        "refactor"
    } else if let Some(r) = branch.strip_prefix("docs/") {
        rest = r;
        "docs"
    } else if let Some(r) = branch.strip_prefix("test/") {
        rest = r;
        "test"
    } else {
        ""
    };
    let cleaned: String = rest
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.is_empty() {
        branch.to_string()
    } else if verb.is_empty() {
        words.join(" ")
    } else {
        format!("{verb} {}", words.join(" "))
    }
}

/// Acquire the log lock with bounded retry (same discipline as capture).
fn acquire_with_retry(log_path: &Path) -> Result<FileLock, String> {
    let attempts = 30;
    let mut last = String::new();
    for _ in 0..attempts {
        match FileLock::acquire(log_path) {
            Ok(lock) => return Ok(lock),
            Err(e) => {
                last = format!("{e}");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    Err(format!(
        "lock {log_path:?} still busy after {attempts} attempts: {last}"
    ))
}

/// Append a `dock.record` under the FileLock (dedupe by (kind,payload)).
fn append_dock_record(log_path: &Path, payload: Value, recorded_at: u64) -> Result<(), String> {
    let _lock = acquire_with_retry(log_path)?;
    let mut log = load_event_log(log_path).map_err(|e| format!("load log: {}", e.to_json()))?;

    let payload_str = payload.to_string();
    let payload_canonical = canonical_json(&payload_str).unwrap_or_else(|| payload_str.clone());

    let dup = log.records().iter().any(|r| {
        r.kind == DOCK_RECORD_KIND && canonical_json(&r.payload) == Some(payload_canonical.clone())
    });
    if dup {
        return Ok(());
    }

    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Push,
        DOCK_RECORD_KIND.to_string(),
        vec![DOCK_PRINCIPAL.to_string()],
        payload_canonical,
        recorded_at,
    )
    .map_err(|denied| format!("authz denied: {:?}", denied.reason))?;

    let bytes = serde_json::to_vec_pretty(log.records()).map_err(|e| format!("serialize: {e}"))?;
    crate::pr::filelock::atomic_write(log_path, &bytes).map_err(|e| format!("persist: {e}"))
}

/// The `hugit dock coin` args — used directly by the post-checkout hook.
#[derive(clap::Args, Debug)]
pub struct CoinArgs {
    /// The repo top-level (`git rev-parse --show-toplevel`).
    #[arg(long)]
    pub top_level: PathBuf,
    /// The absolute gitdir (`git rev-parse --absolute-git-dir`) — THE identity.
    #[arg(long)]
    pub gitdir: PathBuf,
    /// The branch currently checked out (`git branch --show-current`).
    #[arg(long)]
    pub branch: String,
    /// The canonical log path.
    #[arg(long)]
    pub log: PathBuf,
    /// Best-effort hooks log.
    #[arg(long)]
    pub hook_log: Option<PathBuf>,
}

/// The dock subcommand surface (WP-DOCK-1 coin; WP-DOCK-2 ls/show; WP-DOCK-3
/// close/reconcile/insight).
#[derive(clap::Subcommand, Debug)]
pub enum DockCommand {
    /// Coin a dock for a worktree/repo at checkout time (hooks call this).
    Coin(CoinArgs),
    /// List the docks (WP-DOCK-2): id, branch, charter, state (incl. ghosts).
    Ls(LsArgs),
    /// Show one dock's full record (WP-DOCK-2).
    Show(ShowArgs),
    /// Close a dock (WP-DOCK-3 A4): finalize + reconcile, idempotent.
    Close(close::CloseArgs),
    /// Auto-close all ghost docks (WP-DOCK-3 L3) — no open dock with a dead
    /// gitdir remains forever.
    Reconcile(close::ReconcileArgs),
    /// The per-branch cost projection (WP-DOCK-3 F5) + residual buckets.
    Insight(insights::InsightArgs),
    /// Land a dock (WP-DOCK-6): byte-identity + acceptance → union verdict.
    Land(land::DockLandArgs),
}

/// The `hugit dock` root args.
#[derive(clap::Args, Debug)]
pub struct DockArgs {
    #[command(subcommand)]
    pub command: DockCommand,
}

/// `hugit dock ls` args.
#[derive(clap::Args, Debug)]
pub struct LsArgs {
    /// The canonical log path (default: $HUGIT_LOG, else .hugit/log.json).
    #[arg(long)]
    pub log: Option<PathBuf>,
}

/// `hugit dock show <id>` args.
#[derive(clap::Args, Debug)]
pub struct ShowArgs {
    /// The dock id.
    pub id: String,
    /// The canonical log path.
    #[arg(long)]
    pub log: Option<PathBuf>,
}

/// The dock's resolved state (WP-DOCK-2): a dock whose gitdir vanished is
/// `ghost`; a resolved "current" dock is `open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockState {
    Open,
    Ghost,
}

/// A resolved dock (the resolver's answer) — one dock id for a cwd.
#[derive(Debug, Clone)]
pub struct ResolvedDock {
    /// The stable dock id.
    pub dock_id: String,
    /// The gitdir identity.
    pub gitdir: String,
    /// The branch at coin time.
    pub branch: String,
    /// The resolved state (ghost when the gitdir vanished).
    pub state: DockState,
    /// `worktree` | `repo` | `clone`.
    pub origin: String,
}

/// The resolver's fail-closed errors (never a "guessed" dock).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// cwd is not inside a git repository.
    NotARepo,
    /// The canonical log does not exist (no `hugit init`).
    NoLog,
    /// More than one OPEN dock shares the cwd's gitdir — never guess.
    AmbiguousDock { ids: Vec<String> },
    /// cwd is a git repo + log exists but no dock is bound there.
    NoDock,
}

/// The parameters for a dock coinage — grouped so the shared coin path stays
/// under the arg-count lint and the call sites read as a spec, not a list.
pub struct CoinSpec<'a> {
    /// The canonical log path.
    pub log: &'a Path,
    /// Best-effort hooks log (hook call sites).
    pub hook_log: Option<&'a Path>,
    /// The repo top-level (used for R2 warning routing only in hook mode).
    pub top_level: &'a Path,
    /// The absolute gitdir — the dock's physical identity.
    pub gitdir: &'a str,
    /// The branch at coin time.
    pub branch: &'a str,
    /// The charter (derived by the hook; M5 uses "repository scope (…)").
    pub charter: &'a str,
    /// `worktree` | `repo` | `clone`.
    pub origin: &'a str,
    /// An HUGIT_DOCK_ID env value, if set (R2 divergence handling).
    pub env_dock_id: Option<String>,
}

/// Coin a dock (idempotent) — the shared coinage used by the hook AND the
/// resolver's self-heal / M5 auto-coin.
pub fn coin_dock(spec: &CoinSpec<'_>) -> Result<String, String> {
    let gitdir = spec.gitdir;
    let log = spec.log;
    let marker_path = Path::new(gitdir).join(DOCK_MARKER);
    if marker_path.exists() {
        // Idempotent: a marker exists for this gitdir ⇒ no-op.
        //
        // F9 (cold-verify) — SEMANTIC DECISION: idempotency is PER-GITDIR
        // (a worktree = one lifelong dock regardless of branch switches), NOT
        // per-(gitdir, branch). The dock_id carries the branch at COIN time;
        // a later `git switch` in the same worktree keeps the original dock
        // (the worktree's physical identity does not change). Per-branch cost
        // attribution stays correct because the DOCK record's branch is what
        // reconcile/insights read; the worktree is the durable physical unit.
        // The acceptance e2e asserts this contract explicitly.
        // A marker-orphan (marker present, record lost) is the R1 self-heal
        // case: re-coin — the marker attests the intent, the record is the
        // truth that was lost.
        if let Some(existing) = find_payload_by_gitdir(log, gitdir)? {
            return Ok(existing
                .get("dock_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string());
        }
        // fall through → re-coin (marker-overwrite below)
    }

    let mut parent_id = None;
    if let Some(env_id) = spec.env_dock_id.clone().filter(|id| !id.is_empty()) {
        let env_gitdir = find_dock_gitdir_by_id(log, &env_id)?;
        if env_gitdir.as_deref() != Some(gitdir) {
            parent_id = Some(env_id.clone());
            write_hook_log(
                spec.top_level,
                spec.hook_log,
                &format!(
                    "[hugit dock] cwd gitdir {gitdir} differs from HUGIT_DOCK_ID's \
                     ({env_id} at {:?}); coin child dock, parent_id={env_id}",
                    env_gitdir
                ),
            );
        }
    }

    let created_ts = now_unix_ms();
    let pid = std::process::id();
    let payload = json!({
        "dock_id": dock_id(gitdir, spec.branch),
        "gitdir": gitdir,
        "branch": spec.branch,
        "charter": spec.charter,
        "charter_derived": true,
        "state": "open",
        "origin": spec.origin,
        "created_ts": created_ts,
        "pid": pid,
        "parent_id": parent_id,
    });

    let marker = json!({ "created_ts": created_ts, "pid": pid });
    std::fs::write(&marker_path, marker.to_string())
        .map_err(|e| format!("write marker {}: {e}", marker_path.display()))?;

    append_dock_record(log, payload, created_ts)?;
    Ok(dock_id(gitdir, spec.branch))
}

/// Cross-platform worktree-gitdir detection: a linked worktree's gitdir is the
/// path `<main-git>/worktrees/<name>`. The old `gitdir.contains("/worktrees/")`
/// broke on Windows (`\worktrees\`), which misclassified a worktree as the main
/// repo and made the resolver auto-coin a repo-scope dock where it must stay
/// NoDock (L2). Parse the path by components — separator-agnostic.
#[must_use]
pub fn is_worktree_gitdir(gitdir: &str) -> bool {
    if gitdir.is_empty() {
        return false;
    }
    // Split on BOTH separators so it works on Unix (`/worktrees/`) and Windows
    // (`\worktrees\`). A linked worktree's gitdir has a `worktrees` component.
    let norm: String = gitdir
        .chars()
        .map(|c| if c == '\\' { '/' } else { c })
        .collect();
    norm.split('/').any(|seg| seg == "worktrees")
}

fn coin_inner(args: &CoinArgs, env_dock_id: Option<String>) -> Result<(), String> {
    let gitdir = args.gitdir.to_string_lossy().to_string();
    let origin = if is_worktree_gitdir(&gitdir) {
        "worktree"
    } else {
        "repo"
    };
    let spec = CoinSpec {
        log: &args.log,
        hook_log: args.hook_log.as_deref(),
        top_level: &args.top_level,
        gitdir: &gitdir,
        branch: &args.branch,
        charter: &derive_charter(&args.branch),
        origin,
        env_dock_id,
    };
    coin_dock(&spec).map(|_| ())
}

/// Look up a dock payload from the log by gitdir (self-heal / marker reuse).
pub fn find_payload_by_gitdir(log_path: &Path, gitdir: &str) -> Result<Option<Value>, String> {
    Ok(all_dock_payloads(log_path)?
        .into_iter()
        .find(|p| p.get("gitdir").and_then(|v| v.as_str()) == Some(gitdir)))
}

/// Look up a dock's gitdir from the log by its dock_id (R2).
fn find_dock_gitdir_by_id(log_path: &Path, dock_id: &str) -> Result<Option<String>, String> {
    Ok(find_dock_payload(log_path, dock_id)?
        .and_then(|p| p.get("gitdir").and_then(|v| v.as_str()).map(String::from)))
}

pub fn all_dock_payloads(log: &Path) -> Result<Vec<Value>, String> {
    let log = load_event_log(log).map_err(|e| format!("load log: {}", e.to_json()))?;
    Ok(log
        .records()
        .iter()
        .filter(|r| r.kind == DOCK_RECORD_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .collect())
}

/// Look up a dock by id across the records (returns the payload).
pub fn find_dock_payload(log: &Path, dock_id: &str) -> Result<Option<Value>, String> {
    Ok(all_dock_payloads(log)?
        .into_iter()
        .find(|p| p.get("dock_id").and_then(|v| v.as_str()) == Some(dock_id)))
}

/// Run `hugit dock` — silent, exit 0 ALWAYS (hook contract).
pub fn run(args: DockArgs) -> ExitCode {
    match args.command {
        DockCommand::Coin(coin) => run_coin(coin),
        DockCommand::Ls(ls) => run_ls(ls),
        DockCommand::Show(show) => run_show(show),
        DockCommand::Close(close) => close::run_close(close),
        DockCommand::Reconcile(reconcile) => close::run_reconcile(reconcile),
        DockCommand::Insight(insight) => insights::run_insight(insight),
        DockCommand::Land(land) => land::run(land),
    }
}

fn run_coin(coin: CoinArgs) -> ExitCode {
    let env_dock_id = std::env::var("HUGIT_DOCK_ID").ok();
    match coin_inner(&coin, env_dock_id) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            write_hook_log(&coin.top_level, coin.hook_log.as_deref(), &e);
            // SILENT CONTRACT: never exit non-zero.
            ExitCode::SUCCESS
        }
    }
}

fn run_ls(ls: LsArgs) -> ExitCode {
    let log = match crate::log_resolve::resolve_log(ls.log) {
        Ok(log) => log,
        Err(error) => {
            println!("{}", error.to_json());
            return error.exit_code();
        }
    };
    // R4 observation (cold-verify F8): `dock ls` is the enumeration horizon —
    // mark every vanished-gitdir dock as ghost ONCE before listing (a closing
    // gitdir never waits for a specific dock's read).
    let _ = resolve::mark_ghosts(&log);
    let payloads = all_dock_payloads(&log);
    match payloads {
        Ok(list) => {
            let out: Vec<Value> = list
                .into_iter()
                .map(|p| {
                    let gitdir = p.get("gitdir").and_then(|v| v.as_str()).unwrap_or("");
                    let state = if std::path::Path::new(gitdir).exists() {
                        p.get("state")
                            .and_then(|v| v.as_str())
                            .unwrap_or("open")
                            .to_string()
                    } else {
                        "ghost".to_string()
                    };
                    json!({
                        "dock_id": p.get("dock_id").and_then(|v| v.as_str()).unwrap_or(""),
                        "branch": p.get("branch").and_then(|v| v.as_str()).unwrap_or(""),
                        "charter": p.get("charter").and_then(|v| v.as_str()).unwrap_or(""),
                        "state": state,
                        "origin": p.get("origin").and_then(|v| v.as_str()).unwrap_or(""),
                        "created_ts": p.get("created_ts").and_then(|v| v.as_u64()).unwrap_or(0),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string(&sanitized_view(json!(out))).unwrap_or_else(|_| "[]".into())
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run_show(show: ShowArgs) -> ExitCode {
    let log = match crate::log_resolve::resolve_log(show.log) {
        Ok(log) => log,
        Err(error) => {
            println!("{}", error.to_json());
            return error.exit_code();
        }
    };
    match find_dock_payload(&log, &show.id) {
        Ok(Some(p)) => {
            let gitdir = p
                .get("gitdir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut p = p;
            if !std::path::Path::new(&gitdir).exists() {
                p["state"] = json!("ghost");
            }
            println!("{}", sanitized_view(p));
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!(
                "{{\"error\":\"dock_not_found\",\"id\":{}}}",
                serde_json::to_string(&show.id).unwrap()
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
