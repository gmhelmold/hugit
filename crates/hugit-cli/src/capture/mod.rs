//! `hugit capture` — the silent capture verb for git hooks.
//!
//! The git-local product direction (owner, 2026-09-03): the LLM uses `git`
//! normally; hugit observes **silently** in the background and records what the
//! agent did to the git graph — zero friction for the agent.
//!
//! Hooks ([`.git/hooks/{post-commit,post-checkout,pre-push,post-merge}`])
//! installed by `hugit init` call this verb with the event facts (oid, branch,
//! from/to, refspecs) captured from the shell. This verb publishes a `ref.update`
//! receipt (the frozen raw-push kind) for detached canonical projection under
//! [`Endpoint::Push`] (the universal git verb — every principal class may push,
//! so a hook projection can never be authz-denied).
//!
//! ## The silent contract (never blocks git, never emits, never lies)
//!
//! - **exit 0 ALWAYS.** Any error (missing log, broken chain, IO) is written to
//!   the best-effort hooks log (`.hugit/hooks.log`) via the `--hook-log` arg,
//!   and the verb exits 0. A hook failure must NEVER fail or block the git
//!   operation that triggered it.
//! - **Receipt before projection**: immutable, scrubbed facts are durably
//!   published before a detached worker alone acquires the canonical writer lock.
//! - **Receipt-ID dedupe** prevents a re-fired worker from double-recording.
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
use std::process::{Command, ExitCode};

use serde_json::json;

#[cfg(test)]
use crate::checks::load_event_log;

pub mod capability;
pub mod drain;
pub mod receipt;

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
fn write_hook_log(path: &std::path::Path, message: &str) {
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

/// Publish a scrubbed immutable `ref.update` receipt, then detach projection.
///
/// `recorded_at` is supplied by the caller (from the git event where available;
/// otherwise now).
/// Returns `()` on success OR an error with `exit: true` set on the returned
/// `CaptureError` — but the public entry points convert EVERYTHING to exit 0 +
/// hook-log, per the silent contract.
///
/// Only stable codes reach hooks.log. Capture input, paths, and lower-level
/// errors can contain credentials or caller-controlled filesystem locations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureFault {
    LogResolve,
    RuntimePrepare,
    ReceiptDirectory,
    ReceiptIdentity,
    ReceiptPublish,
    WorkerSpawn,
    WorkerDrain,
    MissingOid,
    UnknownKind,
}

impl CaptureFault {
    const fn code(self) -> &'static str {
        match self {
            Self::LogResolve => "capture.log_resolve",
            Self::RuntimePrepare => "capture.runtime_prepare",
            Self::ReceiptDirectory => "capture.receipt_directory",
            Self::ReceiptIdentity => "capture.receipt_identity",
            Self::ReceiptPublish => "capture.receipt_publish",
            Self::WorkerSpawn => "capture.worker_spawn",
            Self::WorkerDrain => "capture.worker_drain",
            Self::MissingOid => "capture.missing_oid",
            Self::UnknownKind => "capture.unknown_kind",
        }
    }
}
fn capture_ref_update(
    log_path: &std::path::Path,
    hook_log: Option<&std::path::Path>,
    principal: &str,
    kind: &str,
    payload: serde_json::Value,
    recorded_at: u64,
) -> Result<(), CaptureFault> {
    crate::runtime_store::prepare_runtime_log(log_path)
        .map_err(|_| CaptureFault::RuntimePrepare)?;
    let root = log_path.parent().ok_or(CaptureFault::ReceiptDirectory)?;
    receipt::ensure_private_dir(root).map_err(|_| CaptureFault::ReceiptDirectory)?;
    let repo_id =
        crate::runtime_store::repository_id(log_path).map_err(|_| CaptureFault::ReceiptIdentity)?;
    let invocation_id =
        receipt::random_invocation_id().map_err(|_| CaptureFault::ReceiptIdentity)?;
    let receipt_id = receipt::receipt_id(&repo_id, &invocation_id);
    let mut payload = payload;
    crate::porcelain::scrub_payload(&mut payload);
    let item = receipt::ReceiptV1 {
        version: 1,
        repo_id,
        invocation_id,
        receipt_id,
        kind: kind.to_string(),
        principal: crate::porcelain::structural_secret_scrub(principal),
        payload,
        recorded_at,
    };
    receipt::write_receipt(&root.join(crate::runtime_store::RECEIPTS_DIR), &item)
        .map_err(|_| CaptureFault::ReceiptPublish)?;

    // Receipt publication is complete. Projection runs only in detached worker;
    // hook path never waits on lock retries, log I/O, or canonical append.
    drain::spawn_worker(log_path, hook_log).map_err(|_| CaptureFault::WorkerSpawn)?;

    // Best-effort trace to the hooks log (never fatal).
    if let Some(h) = hook_log {
        write_hook_log(h, "capture.recorded");
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
    /// Optional explicit event log. Defaults to `top_level` Git common-dir
    /// runtime state, migrating verified legacy state when present.
    #[arg(long)]
    pub log: Option<PathBuf>,
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
    /// Bounded, shell-parsed `pre-push` tuples: local-ref, local-oid,
    /// remote-ref, remote-oid, tab-delimited and newline-separated. Never raw stdin.
    #[arg(long)]
    pub push_tuples: Option<String>,
    /// Bounded old/new/object-type tuples from post-rewrite.
    #[arg(long)]
    pub rewrite_tuples: Option<String>,
    /// Rewrite source supplied by post-rewrite (`amend` or `rebase`).
    #[arg(long)]
    pub rewrite_type: Option<String>,
    /// Bounded old/new/ref tuples from reference-transaction stdin.
    #[arg(long)]
    pub reference_tuples: Option<String>,
    /// reference-transaction phase: prepared, committed, or aborted.
    #[arg(long)]
    pub transaction_phase: Option<String>,
    /// Internal detached receipt worker switch. Hook input payload remains unchanged.
    #[arg(long, hide = true)]
    pub drain_worker: bool,
}

/// Run `hugit capture <kind>` — silent, exit 0 always (the contract §2).
pub fn run(args: CaptureArgs) -> ExitCode {
    let kind = args.kind.as_str();
    let err_to_log = |fault: CaptureFault| -> ExitCode {
        if let Some(h) = &args.hook_log {
            write_hook_log(h, fault.code());
        }
        // SILENT CONTRACT: never exit non-zero, never print.
        ExitCode::SUCCESS
    };

    let log = match crate::log_resolve::resolve_log_for_repo(args.log.clone(), &args.top_level) {
        Ok(log) => log,
        Err(_) => return err_to_log(CaptureFault::LogResolve),
    };
    if args.drain_worker {
        // Hooks never wait for projection. A detached worker can outlive one
        // bounded lock handoff, so retry its own `log_busy` result rather than
        // stranding a final receipt when no later hook fires.
        for attempt in 0..3 {
            match drain::drain(&log, 32) {
                Ok(status)
                    if status.pending > 0
                        && status
                            .last_error
                            .as_ref()
                            .is_some_and(|error| error.code == "log_busy")
                        && attempt < 2 =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Ok(_) => return ExitCode::SUCCESS,
                Err(_) => {
                    if let Some(h) = &args.hook_log {
                        write_hook_log(h, CaptureFault::WorkerDrain.code());
                    }
                    return ExitCode::SUCCESS;
                }
            }
        }
        if let Some(h) = &args.hook_log {
            write_hook_log(h, CaptureFault::WorkerDrain.code());
        }
        return ExitCode::SUCCESS;
    }
    let result = match kind {
        "commit" => capture_commit(&args, &log),
        "checkout" => capture_checkout(&args, &log),
        "push-attempt" => capture_push_attempt(&args, &log),
        "merge" => capture_merge(&args, &log),
        "rewrite" => capture_rewrite(&args, &log),
        "reference-transaction" => capture_reference_transaction(&args, &log),
        _ => Err(CaptureFault::UnknownKind),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => err_to_log(e),
    }
}

fn capture_commit(args: &CaptureArgs, log: &std::path::Path) -> Result<(), CaptureFault> {
    let oid = args.oid.clone().ok_or(CaptureFault::MissingOid)?;
    let branch = args.branch.clone().unwrap_or_default();
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    // Object may already be pruned by time worker runs. Keep immutable ref fact;
    // omit only derived paths, never fall back to moving HEAD.
    let files = changed_paths(&args.top_level, &oid, false).unwrap_or_default();
    // `files` is included only when non-empty so `payload_attribution` (the
    // `why` resolver) can attribute a path to this captured commit; an empty
    // list is omitted (payload stays lean for pure ref events).
    let mut payload = json!({
        "ref": if branch.is_empty() { "HEAD".to_string() } else { format!("refs/heads/{branch}") },
        "target": crate::porcelain::structural_secret_scrub(&oid),
        "branch": branch,
    });
    if !files.is_empty() {
        payload["files"] = json!(files);
    }
    capture_ref_update(
        log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

fn capture_checkout(args: &CaptureArgs, log: &std::path::Path) -> Result<(), CaptureFault> {
    let to = args.oid.clone().ok_or(CaptureFault::MissingOid)?;
    let from = args.from.clone().unwrap_or_default();
    let branch = args.branch.clone().unwrap_or_default();
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    let clone_like = is_zero_oid(&from);
    let payload = json!({
        // `post-checkout` only proves these hook arguments. A zero old OID is
        // clone-like, not proof of clone, so retain fact and truth separately.
        "checkout": {"fact": if clone_like { "zero_old_oid" } else { "nonzero_old_oid" }, "truth": if clone_like { "may_be_clone_or_initial_checkout" } else { "branch_checkout_observed" }},
        "from": crate::porcelain::structural_secret_scrub(&from),
        "to": crate::porcelain::structural_secret_scrub(&to),
        // Additive aliases preserve Git hook's old/new schema for consumers
        // which replay checkout movement separately from refstore's ref view.
        "old": crate::porcelain::structural_secret_scrub(&from),
        "new": crate::porcelain::structural_secret_scrub(&to),
        "branch": branch,
    });
    capture_ref_update(
        log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

fn capture_push_attempt(args: &CaptureArgs, log: &std::path::Path) -> Result<(), CaptureFault> {
    let parsed = parse_push_tuples(args.push_tuples.as_deref().unwrap_or_default());
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    let mut payload = json!({
        "attempt": true,
        "updates": parsed.updates(),
    });
    if let Some(reason) = parsed.incomplete_reason() {
        payload["capture"] = json!({"status": "incomplete", "reason": reason});
    }
    capture_ref_update(
        log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

/// Resolve paths from captured immutable OID. Fixed argv + NUL parsing preserves
/// spaces, tabs, newlines, globs, Unicode, and leading dashes as data.
fn changed_paths(
    top_level: &std::path::Path,
    oid: &str,
    union_parents: bool,
) -> Result<Vec<String>, String> {
    if !is_git_oid(oid) {
        return Err("captured OID is invalid".to_string());
    }
    let mut command = Command::new("git");
    command.current_dir(top_level).args([
        "diff-tree",
        "--root",
        "-r",
        "--name-only",
        "-z",
        "--no-commit-id",
    ]);
    if union_parents {
        // `-m` emits one diff per parent; dedupe below yields explicit union.
        command.arg("-m");
    }
    let output = command
        .arg(oid)
        .output()
        .map_err(|e| format!("changed-path git invocation failed: {}", e.kind()))?;
    if !output.status.success() {
        return Err("changed-path git query failed".to_string());
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec()).map_err(|_| "changed path is not UTF-8".to_string())
        })
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .map(|paths| paths.into_iter().collect())
}

fn is_git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_zero_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte == b'0')
}

/// Parse only bounded typed push tuples. Reject malformed input rather than
/// persisting an ambiguous raw pre-push stream.
enum PushTuples {
    Complete(Vec<serde_json::Value>),
    Incomplete(&'static str),
}

impl PushTuples {
    fn updates(&self) -> Vec<serde_json::Value> {
        match self {
            Self::Complete(updates) => updates.clone(),
            Self::Incomplete(_) => Vec::new(),
        }
    }

    const fn incomplete_reason(&self) -> Option<&'static str> {
        match self {
            Self::Complete(_) => None,
            Self::Incomplete(reason) => Some(reason),
        }
    }
}

fn parse_push_tuples(input: &str) -> PushTuples {
    if input.len() > 8192 {
        return PushTuples::Incomplete("oversize");
    }
    let mut tuples = Vec::new();
    for line in input.lines() {
        if line.is_empty() {
            continue;
        }
        if tuples.len() == 64 {
            return PushTuples::Incomplete("oversize");
        }
        // Git pre-push emits four space-delimited fields. Unit/MCP callers may
        // provide tab-delimited tuples; accept either exact separator, never a
        // whitespace-normalized/raw record.
        let fields: Vec<&str> = if line.contains('\t') {
            line.split('\t').collect()
        } else {
            line.split(' ').collect()
        };
        let [local_ref, local_oid, remote_ref, remote_oid] = fields.as_slice() else {
            return PushTuples::Incomplete("malformed");
        };
        if [local_ref, local_oid, remote_ref, remote_oid]
            .iter()
            .any(|field| field.is_empty() || field.contains(['\n', '\r', '\t']))
            || !is_push_ref(local_ref, true)
            || !is_push_ref(remote_ref, false)
            || !is_git_oid(local_oid)
            || !is_git_oid(remote_oid)
        {
            return PushTuples::Incomplete("malformed");
        }
        tuples.push(json!({
            "local_ref": local_ref,
            "local_oid": local_oid,
            "remote_ref": remote_ref,
            "remote_oid": remote_oid,
        }));
    }
    PushTuples::Complete(tuples)
}

/// Git's pre-push protocol permits `(delete)` only for its local side. Reject
/// every other non-ref name: this capture seam has no reason to retain URLs,
/// userinfo, or arbitrary stdin fields.
fn is_push_ref(value: &str, local: bool) -> bool {
    (local && value == "(delete)")
        || (value.starts_with("refs/")
            && !value.contains([' ', '\\', '~', '^', ':', '?', '*', '['])
            && !value.contains("..")
            && !value.ends_with('/')
            && !value.ends_with('.'))
}

fn capture_merge(args: &CaptureArgs, log: &std::path::Path) -> Result<(), CaptureFault> {
    let to = args.oid.clone().ok_or(CaptureFault::MissingOid)?;
    let recorded_at = args.recorded_at.unwrap_or_else(now_unix_ms);
    // A merge has multiple parents. `diff-tree -m` then BTreeSet union gives
    // exact changed-path union without guessing a source parent from moving HEAD.
    let files = changed_paths(&args.top_level, &to, true).unwrap_or_default();
    let parents = commit_parents(&args.top_level, &to).unwrap_or_default();
    let fast_forward =
        parents.len() == 1 && args.from.as_deref() == parents.first().map(String::as_str);
    let merge_kind = if parents.len() > 1 {
        "merge_commit"
    } else if fast_forward {
        "fast_forward"
    } else {
        "unknown"
    };
    let mut payload = json!({"target": crate::porcelain::structural_secret_scrub(&to), "parents": parents, "merge_kind": merge_kind, "fast_forward": fast_forward});
    if !files.is_empty() {
        payload["files"] = json!(files);
    }
    capture_ref_update(
        log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        payload,
        recorded_at,
    )
}

fn commit_parents(top_level: &std::path::Path, oid: &str) -> Result<Vec<String>, String> {
    if !is_git_oid(oid) {
        return Err("captured OID is invalid".into());
    }
    let output = Command::new("git")
        .current_dir(top_level)
        .args(["rev-list", "--parents", "-n", "1", oid])
        .output()
        .map_err(|_| "parent query failed")?;
    if !output.status.success() {
        return Err("parent query failed".into());
    }
    let output = String::from_utf8(output.stdout).map_err(|_| "parent output is not UTF-8")?;
    let mut fields = output.split_whitespace();
    if fields.next() != Some(oid) {
        return Err("parent output does not match captured OID".into());
    }
    let parents: Vec<String> = fields.map(str::to_owned).collect();
    if parents.iter().any(|parent| !is_git_oid(parent)) {
        return Err("parent output contains invalid OID".into());
    }
    Ok(parents)
}

fn bounded_tuples(input: &str, fields: usize) -> Result<Vec<Vec<String>>, &'static str> {
    if input.len() > 8192 {
        return Err("oversize");
    }
    let mut tuples = Vec::new();
    for line in input.lines().filter(|line| !line.is_empty()) {
        if tuples.len() == 64 {
            return Err("oversize");
        }
        let tuple: Vec<String> = if line.contains('\t') {
            line.split('\t').map(str::to_owned).collect()
        } else {
            line.split(' ').map(str::to_owned).collect()
        };
        if tuple.len() != fields
            || tuple
                .iter()
                .any(|value| value.is_empty() || value.contains(['\n', '\r', '\t']))
        {
            return Err("malformed");
        }
        tuples.push(tuple);
    }
    Ok(tuples)
}

fn capture_rewrite(args: &CaptureArgs, log: &std::path::Path) -> Result<(), CaptureFault> {
    let rewrite_type = args
        .rewrite_type
        .as_deref()
        .filter(|value| matches!(*value, "amend" | "rebase"))
        .ok_or(CaptureFault::UnknownKind)?;
    let mappings = bounded_tuples(args.rewrite_tuples.as_deref().unwrap_or_default(), 2)
        .map_err(|_| CaptureFault::UnknownKind)?;
    if mappings
        .iter()
        .any(|mapping| !is_git_oid(&mapping[0]) || !is_git_oid(&mapping[1]))
    {
        return Err(CaptureFault::UnknownKind);
    }
    capture_ref_update(
        log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        json!({"rewrite": {"type": rewrite_type, "mappings": mappings.into_iter().map(|mapping| json!({"from": mapping[0], "to": mapping[1]})).collect::<Vec<_>>()}}),
        args.recorded_at.unwrap_or_else(now_unix_ms),
    )
}

fn capture_reference_transaction(
    args: &CaptureArgs,
    log: &std::path::Path,
) -> Result<(), CaptureFault> {
    let phase = args
        .transaction_phase
        .as_deref()
        .filter(|phase| matches!(*phase, "prepared" | "committed" | "aborted"))
        .ok_or(CaptureFault::UnknownKind)?;
    let updates = bounded_tuples(args.reference_tuples.as_deref().unwrap_or_default(), 3)
        .map_err(|_| CaptureFault::UnknownKind)?;
    if updates.iter().any(|tuple| {
        !is_transaction_oid(&tuple[0])
            || !is_transaction_oid(&tuple[1])
            || !is_push_ref(&tuple[2], false)
    }) {
        return Err(CaptureFault::UnknownKind);
    }
    let fingerprint = crate::runtime_store::sha256_hex(
        serde_json::to_string(&updates)
            .map_err(|_| CaptureFault::UnknownKind)?
            .as_bytes(),
    );
    let updates: Vec<serde_json::Value> = updates
        .into_iter()
        .map(|tuple| json!({"from": tuple[0], "to": tuple[1], "ref": tuple[2]}))
        .collect();
    capture_ref_update(
        log,
        args.hook_log.as_deref(),
        HOOK_PRINCIPAL,
        "ref.update",
        json!({"reference_transaction": {"phase": phase, "updates": updates, "fingerprint": fingerprint}}),
        args.recorded_at.unwrap_or_else(now_unix_ms),
    )
}

fn is_transaction_oid(value: &str) -> bool {
    is_git_oid(value) || matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte == b'0')
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
            log: Some(log.to_path_buf()),
            hook_log: None,
            oid: oid.map(|s| s.to_string()),
            branch: branch.map(|s| s.to_string()),
            from: None,
            recorded_at: Some(1000),
            push_tuples: None,
            rewrite_tuples: None,
            rewrite_type: None,
            reference_tuples: None,
            transaction_phase: None,
            drain_worker: false,
        }
    }

    fn last_ref_update(log_path: &std::path::Path) -> serde_json::Value {
        drain::drain(log_path, 32).expect("explicit worker drain");
        let log = load_event_log(log_path).unwrap();
        let rec = log
            .records()
            .iter()
            .rev()
            .find(|r| r.kind == "ref.update")
            .expect("a ref.update record exists");
        serde_json::from_str(&rec.payload).unwrap()
    }

    fn transaction_pairing(log_path: &std::path::Path) -> &'static str {
        drain::drain(log_path, 32).expect("explicit worker drain");
        let log = load_event_log(log_path).unwrap();
        let payload = log
            .records()
            .iter()
            .rev()
            .find_map(|record| {
                let payload: serde_json::Value = serde_json::from_str(&record.payload).ok()?;
                payload
                    .get("reference_transaction")
                    .is_some()
                    .then_some(payload)
            })
            .expect("transaction payload");
        let fingerprint = payload["reference_transaction"]["fingerprint"]
            .as_str()
            .expect("transaction fingerprint");
        drain::reference_transaction_pairing(log.records(), fingerprint)
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
        assert_eq!(p["checkout"]["fact"], "nonzero_old_oid");
        assert_eq!(p["from"], "aaaa1111");
        assert_eq!(p["to"], "bbbb2222");
        assert_eq!(p["branch"], "feat");
    }

    #[test]
    fn push_attempt_records_attempt_true() {
        let (dir, log) = scratch("push");
        let mut a = args(&dir, &log, "push-attempt", None, None);
        a.push_tuples = Some(format!(
            "refs/heads/main\t{}\trefs/heads/main\t{}\n",
            "a".repeat(40),
            "0".repeat(40)
        ));
        assert_eq!(run(a), ExitCode::SUCCESS);
        let p = last_ref_update(&log);
        assert_eq!(p["attempt"], true);
        assert_eq!(p["updates"][0]["local_oid"], "a".repeat(40));
    }

    #[test]
    fn secret_shaped_push_field_is_scrubbed_before_persistence_and_diagnostics() {
        let (dir, log) = scratch("bad-push");
        let hook_log = dir.join("hooks.log");
        let secrets = [
            ["gh", "p_16C7e42F292c6912E7710c838347Ae178B4a"].concat(),
            ["github", "_pat_11AA22BB33CC44DD55EE66FF77"].concat(),
            "sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF".to_string(),
            ["AK", "IAIOSFODNN7EXAMPLE"].concat(),
            ["xox", "b-123456789012-123456789012-abcdefghijklmnop"].concat(),
        ];
        for secret in &secrets {
            let mut a = args(&dir, &log, "push-attempt", None, None);
            a.push_tuples = Some(format!("{secret}\taaaa1111\trefs/heads/main\t00000000\n"));
            a.hook_log = Some(hook_log.clone());
            assert_eq!(run(a), ExitCode::SUCCESS);
            let persisted = std::fs::read_to_string(&log).unwrap();
            assert!(
                !persisted.contains(secret),
                "raw token cannot reach runtime log: {secret:?}"
            );
            assert!(
                !std::fs::read_to_string(&hook_log)
                    .unwrap_or_default()
                    .contains(secret),
                "raw token cannot reach hook diagnostics: {secret:?}"
            );
        }
    }

    #[test]
    fn malformed_push_tuples_persist_explicit_incomplete_without_input() {
        let (dir, log) = scratch("malformed-push");
        let hook_log = dir.join("hooks.log");
        for input in [
            "https://user:token@example.test/x\taaaa1111\trefs/heads/main\t00000000",
            "refs/heads/main\tnot-an-oid\trefs/heads/main\t0000000000000000000000000000000000000000",
            "refs/heads/main\t0000000000000000000000000000000000000000\trefs/heads/main",
        ] {
            let mut a = args(&dir, &log, "push-attempt", None, None);
            a.push_tuples = Some(input.to_string());
            a.hook_log = Some(hook_log.clone());
            assert_eq!(run(a), ExitCode::SUCCESS);
            let payload = last_ref_update(&log);
            assert_eq!(payload["capture"]["status"], "incomplete");
            assert_eq!(payload["capture"]["reason"], "malformed");
            assert_eq!(payload["updates"], json!([]));
            assert!(!std::fs::read_to_string(&log).unwrap().contains(input));
            assert!(
                !std::fs::read_to_string(&hook_log)
                    .unwrap_or_default()
                    .contains(input)
            );
        }
    }

    #[test]
    fn oversize_push_tuple_persists_explicit_incomplete_without_truncation() {
        let (dir, log) = scratch("oversize-push");
        let input = format!(
            "refs/heads/{}\t{}\trefs/heads/main\t{}",
            "x".repeat(8192),
            "a".repeat(40),
            "0".repeat(40)
        );
        let mut a = args(&dir, &log, "push-attempt", None, None);
        a.push_tuples = Some(input.clone());
        assert_eq!(run(a), ExitCode::SUCCESS);
        let payload = last_ref_update(&log);
        assert_eq!(payload["capture"]["status"], "incomplete");
        assert_eq!(payload["capture"]["reason"], "oversize");
        assert_eq!(payload["updates"], json!([]));
        assert!(!std::fs::read_to_string(&log).unwrap().contains(&input));
    }

    #[test]
    fn path_secret_never_reaches_typed_hook_diagnostic() {
        let (dir, log) = scratch("path-secret");
        let secret = ["gh", "p_16C7e42F292c6912E7710c838347Ae178B4a"].concat();
        let hook_log = dir.join("hooks.log");
        let mut a = args(&dir, &log, "commit", Some("a"), None);
        a.log = Some(dir.join(&secret).join("event-log.json"));
        a.hook_log = Some(hook_log.clone());
        assert_eq!(run(a), ExitCode::SUCCESS);
        let diagnostic = std::fs::read_to_string(hook_log).unwrap();
        assert!(diagnostic.starts_with("capture."));
        assert!(!diagnostic.contains(&secret));
    }

    #[test]
    fn caller_secret_never_reaches_runtime_payload() {
        let (dir, log) = scratch("runtime-secret");
        let secret = ["gh", "p_16C7e42F292c6912E7710c838347Ae178B4a"].concat();
        let a = args(&dir, &log, "commit", Some(&secret), None);
        assert_eq!(run(a), ExitCode::SUCCESS);
        assert!(!std::fs::read_to_string(&log).unwrap().contains(&secret));
        assert_eq!(last_ref_update(&log)["target"], "[REDACTED]");
    }

    #[test]
    fn nul_parser_preserves_hostile_paths() {
        let root = scratch("nul-paths").0;
        let status = Command::new("git").arg("init").arg(&root).status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            return;
        }
        Command::new("git")
            .args([
                "-C",
                root.to_str().unwrap(),
                "config",
                "user.email",
                "t@example.com",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .args(["-C", root.to_str().unwrap(), "config", "user.name", "t"])
            .status()
            .unwrap();
        for path in ["space name", "tab\tname", "line\nname", "--flag", "unicodé"] {
            std::fs::write(root.join(path), "x").unwrap();
        }
        Command::new("git")
            .args(["-C", root.to_str().unwrap(), "add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .args(["-C", root.to_str().unwrap(), "commit", "-m", "paths"])
            .status()
            .unwrap();
        let oid = String::from_utf8(
            Command::new("git")
                .args(["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let paths = changed_paths(&root, oid.trim(), false).unwrap();
        for path in ["space name", "tab\tname", "line\nname", "--flag", "unicodé"] {
            assert!(
                paths.contains(&path.to_string()),
                "NUL parser preserves {path:?}: {paths:?}"
            );
        }
    }

    #[test]
    fn merge_never_guesses_source_parent() {
        let (dir, log) = scratch("merge");
        let mut a = args(&dir, &log, "merge", Some("cccc3333"), None);
        a.from = Some("aaaa1111".to_string());
        assert_eq!(run(a), ExitCode::SUCCESS);
        let p = last_ref_update(&log);
        assert_eq!(p["target"], "cccc3333");
        assert_eq!(p["parents"], serde_json::json!([]));
        assert_eq!(p["merge_kind"], "unknown");
        assert_eq!(p["fast_forward"], false);
    }

    #[test]
    fn rewrite_records_old_new_mapping_with_declared_type() {
        let (dir, log) = scratch("rewrite");
        let mut a = args(&dir, &log, "rewrite", None, None);
        a.rewrite_type = Some("amend".into());
        a.rewrite_tuples = Some(format!("{} {}\n", "a".repeat(40), "b".repeat(40)));
        assert_eq!(run(a), ExitCode::SUCCESS);
        let p = last_ref_update(&log);
        assert_eq!(p["rewrite"]["type"], "amend");
        assert_eq!(p["rewrite"]["mappings"][0]["from"], "a".repeat(40));
        assert_eq!(p["rewrite"]["mappings"][0]["to"], "b".repeat(40));
    }

    #[test]
    fn reference_terminal_pairs_one_prepare_but_not_identical_concurrent_prepares() {
        let (dir, log) = scratch("reference-pairing");
        let tuples = format!("{} {} refs/heads/topic\n", "0".repeat(40), "a".repeat(40));
        for phase in ["prepared", "committed"] {
            let mut a = args(&dir, &log, "reference-transaction", None, None);
            a.transaction_phase = Some(phase.into());
            a.reference_tuples = Some(tuples.clone());
            assert_eq!(run(a), ExitCode::SUCCESS);
            drain::drain(&log, 32).unwrap();
        }
        assert_eq!(transaction_pairing(&log), "paired");

        for phase in ["prepared", "prepared", "committed"] {
            let mut a = args(&dir, &log, "reference-transaction", None, None);
            a.transaction_phase = Some(phase.into());
            a.reference_tuples = Some(tuples.clone());
            assert_eq!(run(a), ExitCode::SUCCESS);
            drain::drain(&log, 32).unwrap();
        }
        assert_eq!(transaction_pairing(&log), "ambiguous");
    }

    #[test]
    fn terminal_first_prepared_converges_without_guessing_identity() {
        let (dir, log) = scratch("terminal-first-pairing");
        let tuples = format!("{} {} refs/heads/topic\n", "0".repeat(40), "a".repeat(40));
        for phase in ["committed", "prepared"] {
            let mut a = args(&dir, &log, "reference-transaction", None, None);
            a.transaction_phase = Some(phase.into());
            a.reference_tuples = Some(tuples.clone());
            assert_eq!(run(a), ExitCode::SUCCESS);
            drain::drain(&log, 32).unwrap();
        }
        assert_eq!(transaction_pairing(&log), "paired");
    }

    #[test]
    fn clone_like_checkout_keeps_fact_separate_from_truth() {
        let (dir, log) = scratch("clone-checkout");
        let mut a = args(&dir, &log, "checkout", Some("b"), Some("main"));
        a.from = Some("0".repeat(40));
        assert_eq!(run(a), ExitCode::SUCCESS);
        let checkout = last_ref_update(&log)["checkout"].clone();
        assert_eq!(checkout["fact"], "zero_old_oid");
        assert_eq!(checkout["truth"], "may_be_clone_or_initial_checkout");
    }

    #[test]
    fn identical_observations_get_distinct_receipts_and_events() {
        let (dir, log) = scratch("dedupe");
        let a = args(&dir, &log, "commit", Some("aaaa1111"), Some("main"));
        assert_eq!(run(a.clone()), ExitCode::SUCCESS);
        assert_eq!(run(a), ExitCode::SUCCESS); // repeat
        drain::drain(&log, 32).expect("explicit worker drain");
        let log = load_event_log(&log).unwrap();
        let count = log
            .records()
            .iter()
            .filter(|r| r.kind == "ref.update")
            .count();
        assert_eq!(
            count, 2,
            "distinct hook invocations remain distinct observations"
        );
    }

    #[test]
    fn receipt_is_published_before_worker_projection() {
        let (dir, log) = scratch("no-inline");
        let a = args(&dir, &log, "commit", Some("aaaa1111"), Some("main"));
        assert_eq!(run(a), ExitCode::SUCCESS);
        assert_eq!(
            load_event_log(&log).unwrap().len(),
            0,
            "capture never drains inline"
        );
        let receipts = log
            .parent()
            .unwrap()
            .join(crate::runtime_store::RECEIPTS_DIR);
        assert_eq!(
            std::fs::read_dir(receipts).unwrap().count(),
            1,
            "published receipt survives worker launch"
        );
    }

    #[test]
    fn receipt_boundary_scrubs_before_detached_drain() {
        let (dir, log) = scratch("receipt-scrub");
        let secret = ["gh", "p_16C7e42F292c6912E7710c838347Ae178B4a"].concat();
        assert_eq!(
            run(args(&dir, &log, "commit", Some(&secret), Some("main"))),
            ExitCode::SUCCESS
        );
        let receipts = log
            .parent()
            .unwrap()
            .join(crate::runtime_store::RECEIPTS_DIR);
        let entry = std::fs::read_dir(receipts)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let receipt = receipt::read_receipt(&entry).unwrap();
        assert_eq!(receipt.payload["target"], "[REDACTED]");
        assert!(
            !String::from_utf8(receipt::read_receipt_bytes(&entry).unwrap())
                .unwrap()
                .contains(&secret)
        );
        assert_eq!(load_event_log(&log).unwrap().len(), 0);
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
        drain::drain(&log, 32).expect("explicit worker drain");
        let log = load_event_log(&log).expect("log still verifies after 3 appends");
        assert_eq!(log.len(), 3);
    }
}
