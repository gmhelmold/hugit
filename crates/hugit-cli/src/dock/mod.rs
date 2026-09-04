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

/// The dock subcommand surface (WP-DOCK-1 scope: coinage only; ls/show are
/// WP-DOCK-2).
#[derive(clap::Subcommand, Debug)]
pub enum DockCommand {
    /// Coin a dock for a worktree/repo at checkout time (hooks call this).
    Coin(CoinArgs),
}

/// The `hugit dock` root args.
#[derive(clap::Args, Debug)]
pub struct DockArgs {
    #[command(subcommand)]
    pub command: DockCommand,
}

fn coin_inner(args: &CoinArgs, env_dock_id: Option<String>) -> Result<(), String> {
    let gitdir = args.gitdir.to_string_lossy().to_string();
    let branch = args.branch.clone();

    // ── S1 idempotency: a marker already present ⇒ no-op ────────────────
    let marker_path = args.gitdir.join(DOCK_MARKER);
    if marker_path.exists() {
        return Ok(());
    }

    // ── R2: env-vs-cwd divergence (never silent; never adopts the env) ───
    let mut parent_id = None;
    if let Some(env_id) = env_dock_id.filter(|id| !id.is_empty()) {
        // The env dock's gitdir comes from the log; if it differs from the
        // cwd gitdir, record parent_id + warn (never adopt).
        let env_gitdir = find_dock_gitdir(&args.log, &env_id)?;
        if env_gitdir.as_deref() != Some(gitdir.as_str()) {
            parent_id = Some(env_id.clone());
            write_hook_log(
                &args.top_level,
                args.hook_log.as_deref(),
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
    let origin = if gitdir.contains("/worktrees/") {
        "worktree"
    } else {
        "repo"
    };

    let payload = json!({
        "dock_id": dock_id(&gitdir, &branch),
        "gitdir": gitdir,
        "branch": branch,
        "charter": derive_charter(&args.branch),
        "charter_derived": true,
        "state": "open",
        "origin": origin,
        "created_ts": created_ts,
        "pid": pid,
        "parent_id": parent_id,
    });

    // ── A3 marker (reborn detection): created_ts + pid ──────────────────
    let marker = json!({ "created_ts": created_ts, "pid": pid });
    std::fs::write(&marker_path, marker.to_string())
        .map_err(|e| format!("write marker {}: {e}", marker_path.display()))?;

    append_dock_record(&args.log, payload, created_ts)?;
    Ok(())
}

/// Look up a dock's gitdir from the log by id (R2).
fn find_dock_gitdir(log_path: &Path, dock_id: &str) -> Result<Option<String>, String> {
    let log = load_event_log(log_path).map_err(|e| format!("load log: {}", e.to_json()))?;
    Ok(log
        .records()
        .iter()
        .filter(|r| r.kind == DOCK_RECORD_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter_map(|payload| {
            payload
                .get("dock_id")
                .and_then(|v| v.as_str())
                .and_then(|id| {
                    if id == dock_id {
                        payload
                            .get("gitdir")
                            .and_then(|g| g.as_str().map(String::from))
                    } else {
                        None
                    }
                })
        })
        .next())
}

/// Run `hugit dock` — silent, exit 0 ALWAYS (hook contract).
pub fn run(args: DockArgs) -> ExitCode {
    match args.command {
        DockCommand::Coin(coin) => {
            let err_to_log = |top: &Path, hook: Option<&Path>, msg: String| -> ExitCode {
                write_hook_log(top, hook, &msg);
                // SILENT CONTRACT: never exit non-zero.
                ExitCode::SUCCESS
            };
            let env_dock_id = std::env::var("HUGIT_DOCK_ID").ok();
            match coin_inner(&coin, env_dock_id) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => err_to_log(&coin.top_level, coin.hook_log.as_deref(), e),
            }
        }
    }
}
