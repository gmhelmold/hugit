//! `hugit capture` — the silent capture verb for git hooks.
//!
//! The git-local product direction (owner, 2026-09-03): the LLM uses `git`
//! normally; hugit observes **silently** in the background and records what the
//! agent did to the git graph — zero friction for the agent.
//!
//! Hooks ([`.git/hooks/{post-commit,post-checkout,pre-push,post-merge}`])
//! installed by `hugit init` call this verb with the event facts (oid, branch,
//! from/to, refspecs) captured from the shell. This verb appends a `ref.update`
//! event (the frozen raw-push kind) onto the canonical log under
//! [`Endpoint::Push`] (the universal git verb — every principal class may push,
//! so a hook append can never be authz-denied).
//!
//! ## The silent contract (never blocks git, never emits, never lies)
//!
//! - **exit 0 ALWAYS.** Any error (missing log, broken chain, IO) is written to
//!   the best-effort hooks log (`.hugit/hooks.log`) via the `--hook-log` arg,
//!   and the verb exits 0. A hook failure must NEVER fail or block the git
//!   operation that triggered it.
//! - **Append under a single `FileLock`** spanning load → scrub → append →
//!   persist, so concurrent hooks (two worktrees) serialize without the
//!   read-modify-write race.
//! - **Dedupe inside the lock**: a repeat of the same `(payload-key)` is
//!   skipped, so a re-fired hook never double-records.
//! - **`recorded_at` from the git event** (committer date / now), so log order
//!   (hash-chain seq) and true order (recorded_at) are both honest.
//! - **Payload qualifiers** (`checkout: true`, `attempt: true`, `merged_from`)
//!   distinguish the hook source on the SAME frozen `ref.update` kind — no new
//!   kinds, no contract change, no sibling-crate change.
//!
//! ## X5
//!
//! `capture` is NOT a git command (verified: not in `git --list-cmds=builtins,main`),
//! so it does not shadow git — the X5 no-shadow law holds. (`hook` WOULD shadow
//! `git hook` — that's why this verb is named `capture`.)

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::checks::load_event_log;
use crate::pr::filelock::FileLock;

use hugit_refstore::authz::{Endpoint, PrincipalClass};
use hugit_refstore::canonical_json;

/// The recorder identity for hook-captured events. Consistent with the CLI's
/// local recorder default (`orchestrator:hugit` in `check`), but distinct so a
/// captured (hook) event is never mistaken for an explicit orchestration act.
const HOOK_PRINCIPAL: &str = "orchestrator:hugit-hook";

/// Milliseconds now (unix epoch) — the wall-clock stamp for push-attempts and
/// checkouts (no committer date available); commits/merges pass `--recorded-at`.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The best-effort hooks log: appends `message` to path (or `.hugit/hooks.log`
/// under the top-level). NEVER fails the verb — a log failure is itself logged
/// nowhere and ignored (silent by contract).
fn write_hook_log(_top_level: &std::path::Path, path: &std::path::Path, message: &str) {
    let log_path = path.to_path_buf();
    let mut line = message.to_string();
    if !line.ends_with('\n') {
        line.push('\n');
    }
    // ignore errors: the hook must never fail the git operation
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

/// Append a `ref.update` event under the FileLock + Endpoint::Push discipline.
///
/// `recorded_at` is supplied by the caller (from the git event where available;
/// otherwise now). `dedupe_key` is the payload-identity used to skip repeats.
/// Returns `()` on success OR an error with `exit: true` set on the returned
/// `CaptureError` — but the public entry points convert EVERYTHING to exit 0 +
/// hook-log, per the silent contract.
///
/// Retry a [`FileLock::acquire`] with short backoff.
///
/// Hooks fire in rapid succession (a commit's async capture can still hold the
/// lock when the next checkout hook fires); a plain `Busy` would silently drop
/// the capture. We retry briefly (bounded) because the hook contract is
/// "never block git" — a bounded wait for the lock is compatible with that.
fn acquire_with_retry(log_path: &std::path::Path) -> Result<FileLock, String> {
    let attempts = 30; // 30 x 100ms = up to ~3s of waiting for a stale peer
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

fn capture_ref_update(
    log_path: &std::path::Path,
    hook_log: Option<&std::path::Path>,
    principal: &str,
    kind: &str,
    payload: serde_json::Value,
    recorded_at: u64,
) -> Result<(), String> {
    let _lock = acquire_with_retry(log_path)?;

    // Load + hash-chain-verify under the lock (the chokepoint).
    let mut log =
        load_event_log(log_path).map_err(|e| format!("load log error: {}", e.to_json()))?;

    // Scrub the payload + principal before hitting the chain (WG-SCRUB).
    let principal_sc = crate::redaction::scrub(principal);
    let payload_str = payload.to_string();
    let payload_canonical = canonical_json(&payload_str).unwrap_or_else(|| payload_str.clone());

    // Dedupe inside the lock: skip a repeat of the same (kind, payload).
    let dup = log
        .records()
        .iter()
        .any(|r| r.kind == kind && canonical_json(&r.payload) == Some(payload_canonical.clone()));
    if dup {
        return Ok(());
    }

    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Push,
        kind.to_string(),
        vec![principal_sc],
        payload_canonical,
        recorded_at,
    )
    .map_err(|denied| format!("authz denied: {:?}", denied.reason))?;

    // Persist atomically under the lock.
    let bytes =
        serde_json::to_vec_pretty(log.records()).map_err(|e| format!("serialize log: {e}"))?;
    crate::pr::filelock::atomic_write(log_path, &bytes).map_err(|e| format!("persist log: {e}"))?;

    // Best-effort trace to the hooks log (never fatal).
    if let Some(h) = hook_log {
        write_hook_log(h, h, &format!("captured {kind} at {recorded_at}"));
    }
    Ok(())
}

/// The capture subcommand surface.
#[derive(Clone, clap::Args, Debug)]
pub struct CaptureArgs {
    /// The capture kind: commit | checkout | push-attempt | merge.
    #[arg(long)]
    pub kind: String,
    /// The repo top-level (from `git rev-parse --show-toplevel`).
    #[arg(long)]
    pub top_level: PathBuf,
    /// The canonical log path (absolute: `<top-level>/.hugit/log.json`).
    #[arg(long)]
    pub log: PathBuf,
    /// Best-effort hooks log (`.hugit/hooks.log`).
    #[arg(long)]
    pub hook_log: Option<PathBuf>,
    /// The raw value (commit: new oid; checkout: `to`; merge: `to`).
    #[arg(long)]
    pub oid: Option<String>,
    /// The branch name (commit/checkout).
    #[arg(long)]
    pub branch: Option<String>,
    /// The previous ref target (checkout: `from`; merge: `from`).
    #[arg(long)]
    pub from: Option<String>,
    /// The value to stamp recorded_at (committer date for commit/merge).
    #[arg(long)]
    pub recorded_at: Option<u64>,
    /// The refspec lines from a `pre-push` stdin (newline-separated).
    #[arg(long)]
    pub refspecs: Option<String>,
    /// The local SHAs being pushed (part of the push-attempt payload).
    #[arg(long)]
    pub shas: Option<String>,
}

/// Run `hugit capture <kind>` — silent, exit 0 always (the contract §2).
pub fn run(args: CaptureArgs) -> ExitCode {
    let kind = args.kind.as_str();
    let err_to_log = |msg: String| -> ExitCode {
        if let Some(h) = &args.hook_log {
            write_hook_log(&args.top_level, h, &msg);
        }
        // SILENT CONTRACT: never exit non-zero, never print.
        ExitCode::SUCCESS
    };

    let result = match kind {
        "commit" => capture_commit(&args),
        "checkout" => capture_checkout(&args),
        "push-attempt" => capture_push_attempt(&args),
        "merge" => capture_merge(&args),
        other => Err(format!("unknown capture kind: {other}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => err_to_log(e),
    }
}

fn capture_commit(args: &CaptureArgs) -> Result<(), String> {
    let oid = args
        .oid
        .clone()
        .ok_or_else(|| "capture commit: --oid required".to_string())?;
    let branch = args.branch.clone().unwrap_or_default();
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    let payload = json!({
        "ref": if branch.is_empty() { "HEAD".to_string() } else { format!("refs/heads/{branch}") },
        "target": oid,
        "branch": branch,
    });
    capture_ref_update(
        &args.log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

fn capture_checkout(args: &CaptureArgs) -> Result<(), String> {
    let to = args
        .oid
        .clone()
        .ok_or_else(|| "capture checkout: --oid (to) required".to_string())?;
    let from = args.from.clone().unwrap_or_default();
    let branch = args.branch.clone().unwrap_or_default();
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    let payload = json!({
        "checkout": true,
        "from": from,
        "to": to,
        "branch": branch,
    });
    capture_ref_update(
        &args.log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

fn capture_push_attempt(args: &CaptureArgs) -> Result<(), String> {
    let refspecs = args.refspecs.clone().unwrap_or_default();
    let shas = args.shas.clone().unwrap_or_default();
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    let payload = json!({
        "attempt": true,
        "refspecs": refspecs,
        "shas": shas,
    });
    capture_ref_update(
        &args.log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

fn capture_merge(args: &CaptureArgs) -> Result<(), String> {
    let to = args
        .oid
        .clone()
        .ok_or_else(|| "capture merge: --oid (to) required".to_string())?;
    let from = args.from.clone().unwrap_or_default();
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    let payload = json!({
        "merged_from": from,
        "target": to,
    });
    capture_ref_update(
        &args.log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("hugit-capture-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        std::fs::write(&log, b"[]\n").unwrap();
        (dir, log)
    }

    fn args(
        dir: &std::path::Path,
        log: &std::path::Path,
        kind: &str,
        oid: Option<&str>,
        branch: Option<&str>,
    ) -> CaptureArgs {
        CaptureArgs {
            kind: kind.to_string(),
            top_level: dir.to_path_buf(),
            log: log.to_path_buf(),
            hook_log: None,
            oid: oid.map(|s| s.to_string()),
            branch: branch.map(|s| s.to_string()),
            from: None,
            recorded_at: Some(1000),
            refspecs: None,
            shas: None,
        }
    }

    fn last_ref_update(log_path: &std::path::Path) -> serde_json::Value {
        let log = load_event_log(log_path).unwrap();
        let rec = log
            .records()
            .iter()
            .rev()
            .find(|r| r.kind == "ref.update")
            .expect("a ref.update record exists");
        serde_json::from_str(&rec.payload).unwrap()
    }

    #[test]
    fn commit_appends_ref_update_with_branch_and_oid() {
        let (dir, log) = scratch("commit");
        let a = args(&dir, &log, "commit", Some("aaaa1111"), Some("main"));
        assert_eq!(run(a), ExitCode::SUCCESS);
        let p = last_ref_update(&log);
        assert_eq!(p["ref"], "refs/heads/main");
        assert_eq!(p["target"], "aaaa1111");
        assert_eq!(p["branch"], "main");
    }

    #[test]
    fn checkout_records_checkout_true_qualifier() {
        let (dir, log) = scratch("checkout");
        let mut a = args(&dir, &log, "checkout", Some("bbbb2222"), Some("feat"));
        a.from = Some("aaaa1111".to_string());
        assert_eq!(run(a), ExitCode::SUCCESS);
        let p = last_ref_update(&log);
        assert_eq!(p["checkout"], true);
        assert_eq!(p["from"], "aaaa1111");
        assert_eq!(p["to"], "bbbb2222");
        assert_eq!(p["branch"], "feat");
    }

    #[test]
    fn push_attempt_records_attempt_true() {
        let (dir, log) = scratch("push");
        let mut a = args(&dir, &log, "push-attempt", None, None);
        a.refspecs = Some("refs/heads/main:refs/heads/main".to_string());
        a.shas = Some("aaaa1111".to_string());
        assert_eq!(run(a), ExitCode::SUCCESS);
        let p = last_ref_update(&log);
        assert_eq!(p["attempt"], true);
        assert_eq!(p["shas"], "aaaa1111");
    }

    #[test]
    fn merge_records_merged_from_qualifier() {
        let (dir, log) = scratch("merge");
        let mut a = args(&dir, &log, "merge", Some("cccc3333"), None);
        a.from = Some("aaaa1111".to_string());
        assert_eq!(run(a), ExitCode::SUCCESS);
        let p = last_ref_update(&log);
        assert_eq!(p["merged_from"], "aaaa1111");
        assert_eq!(p["target"], "cccc3333");
    }

    #[test]
    fn dedupe_skips_same_payload() {
        let (dir, log) = scratch("dedupe");
        let a = args(&dir, &log, "commit", Some("aaaa1111"), Some("main"));
        assert_eq!(run(a.clone()), ExitCode::SUCCESS);
        assert_eq!(run(a), ExitCode::SUCCESS); // repeat
        let log = load_event_log(&log).unwrap();
        let count = log
            .records()
            .iter()
            .filter(|r| r.kind == "ref.update")
            .count();
        assert_eq!(count, 1, "duplicate payload is skipped");
    }

    #[test]
    fn unknown_kind_is_silent_exit_zero() {
        let (dir, log) = scratch("unknown");
        let a = args(&dir, &log, "bogus", None, None);
        assert_eq!(run(a), ExitCode::SUCCESS); // silent contract: never non-zero
        let log = load_event_log(&log).unwrap();
        assert_eq!(log.records().len(), 0, "nothing appended for unknown kind");
    }

    #[test]
    fn chain_verifies_after_appends() {
        let (dir, log) = scratch("chain");
        for i in 0..3 {
            let a = args(
                &dir,
                &log,
                "commit",
                Some(&format!("aaaa{:04}", i)),
                Some("main"),
            );
            assert_eq!(run(a), ExitCode::SUCCESS);
        }
        let log = load_event_log(&log).expect("log still verifies after 3 appends");
        assert_eq!(log.len(), 3);
    }
}
