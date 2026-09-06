//! The dock resolver (WP-DOCK-2) — cwd → gitdir → dock.
//!
//! The correctness spine of the whole dock design. Answers "which dock am I
//! in?" from any cwd, honoring:
//!
//! - **env fast-path** — `HUGIT_DOCK_ID` set AND its gitdir == cwd gitdir ⇒ use
//!   it without a log read (orchestrator fast-path).
//! - **cwd as truth** — if the env dock's gitdir differs from the cwd gitdir,
//!   the env is IGNORED; the cwd dock wins, and a `dock.reconcile` record is
//!   appended once per (env→cwd) pair (R5 — never silent, exact-once).
//! - **P1 fail-closed** — exactly ONE dock per cwd, or a named error; never a
//!   "guessed" dock. Two OPEN docks sharing the gitdir ⇒ `AmbiguousDock`.
//! - **P3 no fabrication** — a dock whose gitdir is GONE is `ghost` (or
//!   Unbound), never a live dock. The resolver marks `dock.ghost` once.
//! - **R1 self-heal** — a gitdir with a marker but no record ⇒ re-coin via the
//!   shared `coin_dock` (restores a lost coin; never duplicates).
//! - **M5 auto-coin** — a repo with a log but no dock ⇒ coins a **repo-scope**
//!   dock on first read (the coarse-but-real fallback; `origin:"repo"`).
//!
//! The resolver NEVER fabricates a per-unit dock where the git model has none
//! (a worktree without a marker ⇒ `NoDock` — the honest "unlabeled" signal the
//! cost spool (WP-DOCK-4) turns into a visible bucket).

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::{DockState, ResolveError, ResolvedDock};
use crate::checks::load_event_log;
use crate::pr::filelock::FileLock;
use hugit_refstore::authz::{Endpoint, PrincipalClass};
use hugit_refstore::canonical_json;

/// The `dock.reconcile` event kind — records an env→cwd dock mismatch (R5).
pub const DOCK_RECONCILE_KIND: &str = "dock.reconcile";
/// The `dock.ghost` event kind — records that a dock's gitdir vanished (R4).
pub const DOCK_GHOST_KIND: &str = "dock.ghost";

/// Milliseconds now.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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

/// Append a record under the lock with dedupe (exact-once for kind+key).
fn append_deduped(log_path: &Path, kind: &str, payload: Value, key: &str) -> Result<(), String> {
    let _lock = acquire_with_retry(log_path)?;
    let mut log = load_event_log(log_path).map_err(|e| format!("load log: {}", e.to_json()))?;

    let payload_str = payload.to_string();
    let payload_canonical = canonical_json(&payload_str).unwrap_or_else(|| payload_str.clone());

    // Dedupe by (kind, the key value) — exact-once for a reconcile pair / ghost id.
    let key_val = payload.get(key).cloned();
    let existing = log.records().iter().any(|r| {
        r.kind == kind
            && serde_json::from_str::<Value>(&r.payload)
                .ok()
                .and_then(|p| p.get(key).cloned())
                == key_val
    });
    if existing {
        return Ok(());
    }

    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Push,
        kind.to_string(),
        vec!["orchestrator:hugit-hook".to_string()],
        payload_canonical,
        now_unix_ms(),
    )
    .map_err(|denied| format!("authz denied: {:?}", denied.reason))?;

    let bytes = serde_json::to_vec_pretty(log.records()).map_err(|e| format!("serialize: {e}"))?;
    crate::pr::filelock::atomic_write(log_path, &bytes).map_err(|e| format!("persist: {e}"))
}

/// Run `git rev-parse <arg>` in `cwd` — returns the trimmed stdout or None.
fn git_rev(cwd: &Path, arg: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", arg])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Resolve which dock a cwd belongs to. CWD IS THE TRUTH (P2/R5): the env
/// fast-path is honored ONLY when it agrees; a differing env is recorded as a
/// reconcile (never adopted, never silent).
pub fn resolve(
    cwd: &Path,
    log_override: Option<&Path>,
    env_dock_id: Option<String>,
) -> Result<ResolvedDock, ResolveError> {
    let top_level = git_rev(cwd, "--show-toplevel").ok_or(ResolveError::NotARepo)?;
    let gitdir = git_rev(cwd, "--absolute-git-dir").ok_or(ResolveError::NotARepo)?;

    let log = match log_override {
        Some(p) => p.to_path_buf(),
        None => std::path::PathBuf::from(&top_level).join(".hugit/log.json"),
    };
    if !log.exists() {
        return Err(ResolveError::NoLog);
    }

    // Env fast-path: ONLY when it agrees with the cwd gitdir (P2).
    if let Some(env_id) = env_dock_id.filter(|id| !id.is_empty()) {
        let env_dock = find_dock_payload(&log, &env_id).map_err(|_| ResolveError::NoDock)?;
        if let Some(ed) = env_dock
            && ed.get("gitdir").and_then(|v| v.as_str()) == Some(gitdir.as_str())
        {
            let state = current_state(&ed, &gitdir);
            return dock_from(&ed, state);
        }
        // env differs from cwd — RECORD the reconcile, never adopt (R5).
        let _ = append_deduped(
            &log,
            DOCK_RECONCILE_KIND,
            serde_json::json!({
                "env_dock": env_id,
                "cwd_gitdir": gitdir,
                // The exact-once key is the FULL (env→cwd) pair — a SECOND
                // diff worktree under the same env is a DIFFERENT divergence
                // and MUST be recorded (R5 never-silent; cold-verify F2).
                "pair_key": format!("{env_id}→{gitdir}"),
            }),
            "pair_key",
        );
        // fall through to the cwd-truth path
    }

    // Cwd-truth path (P1): the dock(s) bound to THIS gitdir.
    let payloads = all_dock_payloads(&log).map_err(|_| ResolveError::NoDock)?;
    let mine: Vec<&Value> = payloads
        .iter()
        .filter(|p| p.get("gitdir").and_then(|v| v.as_str()) == Some(gitdir.as_str()))
        .collect();

    match mine.len() {
        0 => {
            // Self-heal (R1): marker present but record missing ⇒ re-coin.
            let marker = std::path::Path::new(&gitdir).join(super::DOCK_MARKER);
            if marker.exists() {
                let branch = current_branch(cwd).unwrap_or_default();
                // Re-coin via the shared path (write), then re-read.
                let _ = crate::dock::coin_dock(&crate::dock::CoinSpec {
                    log: &log,
                    hook_log: None,
                    top_level: std::path::Path::new(&top_level),
                    gitdir: &gitdir,
                    branch: &branch,
                    charter: &format!("repository scope ({branch})"),
                    origin: repo_or_worktree(&gitdir),
                    env_dock_id: None,
                });
                let again: Vec<Value> =
                    all_dock_payloads(&log).map_err(|_| ResolveError::NoDock)?;
                return match again
                    .iter()
                    .find(|p| p.get("gitdir").and_then(|v| v.as_str()) == Some(gitdir.as_str()))
                {
                    Some(p) => {
                        let state = current_state(p, &gitdir);
                        dock_from(p, state)
                    }
                    None => Err(ResolveError::NoDock),
                };
            }
            // M5 auto-coin: repo with a log but no dock ⇒ repo-scope dock.
            // (A worktree WITHOUT a marker is NOT auto-coined — the git model
            // has no hook-born dock there; it stays NoDock → "unlabeled".)
            let main_gitdir = repo_or_worktree(&gitdir);
            if main_gitdir == "repo" {
                let branch = current_branch(cwd).unwrap_or_default();
                let _ = crate::dock::coin_dock(&crate::dock::CoinSpec {
                    log: &log,
                    hook_log: None,
                    top_level: std::path::Path::new(&top_level),
                    gitdir: &gitdir,
                    branch: &branch,
                    charter: &format!("repository scope ({branch})"),
                    origin: "repo",
                    env_dock_id: None,
                });
                let again: Vec<Value> =
                    all_dock_payloads(&log).map_err(|_| ResolveError::NoDock)?;
                return match again
                    .iter()
                    .find(|p| p.get("gitdir").and_then(|v| v.as_str()) == Some(gitdir.as_str()))
                {
                    Some(p) => {
                        let state = current_state(p, &gitdir);
                        dock_from(p, state)
                    }
                    None => Err(ResolveError::NoDock),
                };
            }
            Err(ResolveError::NoDock)
        }
        1 => {
            let p = mine[0];
            // Ghost-mark once (R4): gitdir gone ⇒ `dock.ghost`, never a live dock.
            if !std::path::Path::new(&gitdir).exists() {
                let id = p.get("dock_id").and_then(|v| v.as_str()).unwrap_or("");
                let _ = append_deduped(
                    &log,
                    DOCK_GHOST_KIND,
                    serde_json::json!({ "dock_id": id }),
                    "dock_id",
                );
            }
            let state = current_state(p, &gitdir);
            dock_from(p, state)
        }
        _ => {
            let ids: Vec<String> = mine
                .iter()
                .filter_map(|p| p.get("dock_id").and_then(|v| v.as_str()).map(String::from))
                .collect();
            Err(ResolveError::AmbiguousDock { ids })
        }
    }
}

fn dock_from(p: &Value, state: DockState) -> Result<ResolvedDock, ResolveError> {
    Ok(ResolvedDock {
        dock_id: p
            .get("dock_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        gitdir: p
            .get("gitdir")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        branch: p
            .get("branch")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        state,
        origin: p
            .get("origin")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// The dock's CURRENT state: live iff its gitdir exists on disk (P3).
fn current_state(_p: &Value, gitdir: &str) -> DockState {
    if std::path::Path::new(gitdir).exists() {
        DockState::Open
    } else {
        DockState::Ghost
    }
}

fn repo_or_worktree(gitdir: &str) -> &'static str {
    if super::is_worktree_gitdir(gitdir) {
        "worktree"
    } else {
        "repo"
    }
}

pub fn current_branch(cwd: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(cwd)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// R4 — mark EVERY dock whose gitdir vanished as `ghost` on the durable log,
/// exactly ONCE per dock. This is the OBSERVATION point for ghost-marking
/// (cold-verify F8): the resolver can only see its OWN cwd's gitdir (which by
/// construction exists); the enumeration horizon (`dock ls` / reconcile) sees
/// ALL docks and drives the mark. Idempotent via the `pair_key` dedupe.
pub fn mark_ghosts(log_path: &Path) -> Result<usize, String> {
    let payloads = super::all_dock_payloads(log_path)?;
    // Count the ghosts we are about to touch (already-marked are idempotent).
    let mut marked = 0;
    for p in &payloads {
        let id = p.get("dock_id").and_then(Value::as_str).unwrap_or("");
        let gitdir = p.get("gitdir").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() || gitdir.is_empty() {
            continue;
        }
        if std::path::Path::new(gitdir).exists() {
            continue;
        }
        let already = log_has_ghost(log_path, id)?;
        if !already {
            append_deduped(
                log_path,
                DOCK_GHOST_KIND,
                serde_json::json!({
                    "dock_id": id,
                    "gitdir": gitdir,
                }),
                "dock_id",
            )?;
            marked += 1;
        }
    }
    Ok(marked)
}

fn log_has_ghost(log_path: &Path, id: &str) -> Result<bool, String> {
    let log = load_event_log(log_path).map_err(|e| format!("load log: {}", e.to_json()))?;
    Ok(log
        .records()
        .iter()
        .filter(|r| r.kind == DOCK_GHOST_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|p| p.get("dock_id").and_then(Value::as_str) == Some(id)))
}

fn find_dock_payload(log: &Path, dock_id: &str) -> Result<Option<Value>, String> {
    super::find_dock_payload(log, dock_id)
}

fn all_dock_payloads(log: &Path) -> Result<Vec<Value>, String> {
    super::all_dock_payloads(log)
}
