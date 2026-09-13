//! dock::close — the dock lifecycle finalizer (WP-DOCK-3 A4, R4, L3).
//!
//! Closing a dock runs its reconciliation (A4) and appends a `dock.close`
//! record to the canonical log. The close is **idempotent**: a dock already
//! closed returns its existing result (never a second record, never a re-run).
//!
//! Liveness (L3): a dock whose gitdir vanished (`ghost`) is closed by
//! [`reconcile_ghosts`] — dead worktrees eventually finalize, never linger
//! open forever. A ghosts' reconciliation is the branch-level attribution from
//! [`super::reconcile::attribute`]; the close record carries the lane verdict.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::checks::load_event_log;
use crate::pr::filelock::FileLock;
use hugit_refstore::authz::{Endpoint, PrincipalClass};
use hugit_refstore::canonical_json;

use super::reconcile::{Bucket, attribute};
use super::sanitized_view;

/// The frozen on-wire event kind for a finalized dock.
pub const DOCK_CLOSE_KIND: &str = "dock.close";
/// The recorder identity (hook/dock principal — closers are orchestrators).
const CLOSE_PRINCIPAL: &str = "orchestrator:hugit-hook";

/// The `hugit dock close` args (WP-DOCK-3).
#[derive(clap::Args, Debug)]
pub struct CloseArgs {
    /// The dock id; omitted ⇒ the cwd-resolved dock.
    #[arg(long)]
    pub id: Option<String>,
    /// The canonical log path.
    #[arg(long)]
    pub log: Option<PathBuf>,
}

/// The `hugit dock reconcile` args (WP-DOCK-3 L3 — auto-close ghosts).
#[derive(clap::Args, Debug)]
pub struct ReconcileArgs {
    /// The canonical log path.
    #[arg(long)]
    pub log: Option<PathBuf>,
}

/// The close verdict (the A4 lane + its counters) — the SEAL-able result.
#[derive(Debug, Clone)]
pub struct CloseResult {
    pub dock_id: String,
    pub branch: String,
    pub closed_at_ms: u64,
    pub bucket: Bucket,
    pub bucket_name: String,
    pub cost_usd_micros: u64,
    pub commit_count: u64,
    /// Already closed before this call (idempotent replay).
    pub already_closed: bool,
}

impl CloseResult {
    /// Mark this as an already-closed replay (never a second append).
    fn into_replay(mut self) -> Self {
        self.already_closed = true;
        self
    }

    fn to_json(&self) -> Value {
        sanitized_view(json!({
            "dock_id": self.dock_id,
            "branch": self.branch,
            "closed_at_ms": self.closed_at_ms,
            "bucket": self.bucket_name,
            "cost_usd_micros": self.cost_usd_micros,
            "commit_count": self.commit_count,
            "already_closed": self.already_closed,
        }))
    }
}

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

/// The existing close record for a dock, if any (idempotent replay source).
fn existing_close(log: &Path, dock_id: &str) -> Result<Option<Value>, String> {
    let log = load_event_log(log).map_err(|e| format!("load log: {}", e.to_json()))?;
    Ok(log
        .records()
        .iter()
        .filter(|r| r.kind == DOCK_CLOSE_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .find(|p| p.get("dock_id").and_then(Value::as_str) == Some(dock_id)))
}

/// Close a dock (A4) — idempotent. Runs the branch reconciliation, then
/// appends `dock.close`. Re-playing a closed dock returns its existing result.
pub fn close_dock(log_path: &Path, dock_id: &str) -> Result<CloseResult, String> {
    // Fast-path replay (read-only, best-effort): a closed dock returns its
    // verdict WITHOUT attribute or lock. This is NOT the race guard — the
    // authoritative check is FIFO inside the lock below (F6).
    if let Some(existing) = existing_close(log_path, dock_id)? {
        return Ok(existing.into_close_result(true));
    }

    let attribution = attribute(log_path)?;
    let lane = attribution
        .docks
        .iter()
        .find(|d| d.dock_id == dock_id)
        .cloned()
        .ok_or_else(|| format!("dock_not_found:{dock_id}"))?;

    let closed_at_ms = now_unix_ms();
    let result = CloseResult {
        dock_id: lane.dock_id.clone(),
        branch: lane.branch.clone(),
        closed_at_ms,
        bucket: lane.bucket,
        bucket_name: lane.bucket.as_str().to_string(),
        cost_usd_micros: lane.cost_usd_micros,
        commit_count: lane.commit_count,
        already_closed: false,
    };

    // Durable close record under the same FileLock + retry discipline (B4 —
    // coinage/reconcile serialize; 2 concurrent closes serialize too).
    let _lock = acquire_with_retry(log_path)?;
    let mut event_log =
        load_event_log(log_path).map_err(|e| format!("load log: {}", e.to_json()))?;

    // AUTHORITATIVE exact-once INSIDE the lock (F6 — cold-verify TOCTOU):
    // whoever wins the lock re-checks; a concurrent close that slipped the
    // fast-path above is seen here and skipped — never two `dock.close`.
    if event_log
        .records()
        .iter()
        .filter(|r| r.kind == DOCK_CLOSE_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|p| p.get("dock_id").and_then(Value::as_str) == Some(dock_id))
    {
        return Ok(result.into_replay());
    }

    let payload = json!({
        "dock_id": result.dock_id,
        "branch": result.branch,
        "closed_at_ms": result.closed_at_ms,
        "bucket": result.bucket_name,
        "cost_usd_micros": result.cost_usd_micros,
        "commit_count": result.commit_count,
    });
    let payload_str = payload.to_string();
    let payload_canonical = canonical_json(&payload_str).unwrap_or(payload_str);
    event_log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Push,
            DOCK_CLOSE_KIND.to_string(),
            vec![CLOSE_PRINCIPAL.to_string()],
            payload_canonical,
            closed_at_ms,
        )
        .map_err(|denied| format!("authz denied: {:?}", denied.reason))?;
    let bytes =
        serde_json::to_vec_pretty(event_log.records()).map_err(|e| format!("serialize: {e}"))?;
    crate::pr::filelock::atomic_write(log_path, &bytes).map_err(|e| format!("persist: {e}"))?;

    Ok(result)
}

trait IntoCloseResult {
    fn into_close_result(self, already_closed: bool) -> CloseResult;
}
impl IntoCloseResult for Value {
    fn into_close_result(self, already_closed: bool) -> CloseResult {
        CloseResult {
            dock_id: self
                .get("dock_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            branch: self
                .get("branch")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            closed_at_ms: self
                .get("closed_at_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            bucket: bucket_from(
                self.get("bucket")
                    .and_then(Value::as_str)
                    .unwrap_or("unlabeled"),
            ),
            bucket_name: self
                .get("bucket")
                .and_then(Value::as_str)
                .unwrap_or("unlabeled")
                .to_string(),
            cost_usd_micros: self
                .get("cost_usd_micros")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            commit_count: self
                .get("commit_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            already_closed,
        }
    }
}

fn bucket_from(name: &str) -> Bucket {
    match name {
        "matched" => Bucket::Matched,
        "investigated" => Bucket::Investigated,
        "reconciled" => Bucket::Reconciled,
        _ => Bucket::Unlabeled,
    }
}

/// Reconcile every ghost (L3 — a dock whose gitdir vanished is `ghost`; close
/// it so no open dock with a dead gitdir remains). Idempotent; only docks
/// without a close record are closed. Returns the newly-closed results.
pub fn reconcile_ghosts(log: &Path) -> Result<Vec<CloseResult>, String> {
    let attribution = attribute(log)?;
    let mut closed = Vec::new();
    for dock in &attribution.docks {
        if dock.state == "ghost" {
            let res = close_dock(log, &dock.dock_id)?;
            if !res.already_closed {
                closed.push(res);
            }
        }
    }
    Ok(closed)
}

/// Run `hugit dock close` — close the cwd-resolved dock (or `--id`).
pub fn run_close(args: CloseArgs) -> std::process::ExitCode {
    let log = crate::log_resolve::resolve_log(args.log.clone());
    let id = match args.id {
        Some(id) => id,
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match crate::dock::resolve::resolve(
                &cwd,
                args.log.as_deref(),
                std::env::var("HUGIT_DOCK_ID").ok(),
            ) {
                Ok(d) => d.dock_id,
                Err(e) => {
                    println!(
                        "{{\"error\":{},\"dock_id\":null}}",
                        serde_json::to_string(&format!("resolve_failed:{e:?}")).unwrap()
                    );
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
    };
    match close_dock(&log, &id) {
        Ok(result) => {
            println!("{}", result.to_json());
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            println!(
                "{{\"error\":{},\"dock_id\":{}}}",
                serde_json::to_string(&e).unwrap(),
                serde_json::to_string(&id).unwrap()
            );
            std::process::ExitCode::FAILURE
        }
    }
}

/// Run `hugit dock reconcile` — auto-close all ghosts (L3).
pub fn run_reconcile(args: ReconcileArgs) -> std::process::ExitCode {
    let log = crate::log_resolve::resolve_log(args.log);
    match reconcile_ghosts(&log) {
        Ok(closed) => {
            let out: Vec<Value> = closed.iter().map(|c| c.to_json()).collect();
            println!(
                "{}",
                serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
            );
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            let esc = serde_json::to_string(&e).unwrap_or_else(|_| format!("\"{e}\""));
            println!("{{\"error\":{}}}", esc);
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::DOCK_RECORD_KIND;
    use crate::dock::reconcile::attribute;
    use hugit_refstore::authz::{Endpoint, PrincipalClass};
    use hugit_refstore::canonical_json;
    use serde_json::{Value, json};

    struct Builder {
        path: std::path::PathBuf,
    }

    impl Builder {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("hugit-close-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("log.json");
            std::fs::write(&path, b"[]\n").unwrap();
            Self { path }
        }

        fn record(&self, kind: &str, payload: Value) {
            let mut log = load_event_log(&self.path).unwrap();
            let ps = payload.to_string();
            let cap = canonical_json(&ps).unwrap_or(ps);
            log.append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Push,
                kind.to_string(),
                vec![CLOSE_PRINCIPAL.to_string()],
                cap,
                1000,
            )
            .unwrap();
            let bytes = serde_json::to_vec_pretty(log.records()).unwrap();
            crate::pr::filelock::atomic_write(&self.path, &bytes).unwrap();
        }

        fn dock(&self, id: &str, branch: &str, origin: &str, gitdir: &str) {
            self.record(
                DOCK_RECORD_KIND,
                json!({
                    "dock_id": id, "gitdir": gitdir, "branch": branch,
                    "charter": "test", "charter_derived": true, "state": "open",
                    "origin": origin, "created_ts": 900, "pid": 1,
                }),
            );
        }

        fn sample(&self, dock_id: &str, cost: u64, run: &str) {
            // Real M3 hash (re-derivable) — a fake one would be flagged tampered.
            let sample = hugit_contracts::cost_sample::CostSampleV1 {
                dock_id: dock_id.to_string(),
                model: "m".to_string(),
                input_tokens: 1,
                output_tokens: 1,
                cost_usd_micros: cost,
                ts_ms: 1000,
                run_id: run.to_string(),
            };
            let rendered = serde_json::to_string(&sample).unwrap();
            let hash = {
                use sha2::Digest;
                let h = sha2::Sha256::digest(rendered.as_bytes());
                h.iter().map(|b| format!("{b:02x}")).collect::<String>()
            };
            self.record(
                super::super::attest::COST_SAMPLE_KIND,
                json!({
                    "run_id": run, "dock_id": dock_id, "model": "m",
                    "input_tokens": 1, "output_tokens": 1,
                    "cost_usd_micros": cost, "ts_ms": 1000,
                    "content_hash": hash, "is_unlabeled": dock_id.is_empty(),
                }),
            );
        }

        fn commit(&self, branch: &str, target: &str) {
            self.record(
                "ref.update",
                json!({"ref": format!("refs/heads/{branch}"), "target": target, "branch": branch}),
            );
        }
    }

    #[test]
    fn close_is_idempotent_and_finalizes_matched_lane() {
        let b = Builder::new("idem");
        // A LIVE gitdir (exists on disk) so the closed dock projects `closed`
        // (a closed dock whose gitdir vanished would show `ghost` — R4).
        let gitdir = b.path.with_file_name("wt-good");
        std::fs::create_dir_all(&gitdir).unwrap();
        b.dock("d", "feat/good", "worktree", &gitdir.to_string_lossy());
        b.sample("d", 420, "r1");
        b.commit("feat/good", "aaaa");

        let first = close_dock(&b.path, "d").unwrap();
        assert!(!first.already_closed);
        assert_eq!(first.bucket, Bucket::Matched);
        assert_eq!(first.cost_usd_micros, 420);
        assert_eq!(first.commit_count, 1);

        // Replay = no-op, returns the existing verdict (A4 — exact-once close).
        let replay = close_dock(&b.path, "d").unwrap();
        assert!(replay.already_closed);
        assert_eq!(replay.bucket_name, "matched");

        // Exactly ONE dock.close record on the log (never a second).
        let att = attribute(&b.path).unwrap();
        let d = &att.docks[0];
        assert_eq!(d.state, "closed");
        let log = load_event_log(&b.path).unwrap();
        let n = log
            .records()
            .iter()
            .filter(|r| r.kind == DOCK_CLOSE_KIND)
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn l3_ghost_is_reconciled_on_durable_log() {
        let b = Builder::new("l3");
        // A dock whose gitdir VANISHED — the dead worktree case. The gitdir is
        // a path we never create, so the resolver/ls state is ghost.
        b.dock("gone", "feat/dead", "worktree", "/nonexistent/wt-dead");

        let closed = reconcile_ghosts(&b.path).unwrap();
        assert_eq!(
            closed.len(),
            1,
            "L3 — the ghost is closed, none remains open"
        );
        let c = &closed[0];
        assert_eq!(
            c.bucket,
            Bucket::Unlabeled,
            "no cost, no commits → unlabeled"
        );
        assert_eq!(c.dock_id, "gone");

        // Re-run: nothing new (idempotent), the ghost stays reconciled on the
        // durable log and the state projects to `ghost` (R4 — listing truth).
        let again = reconcile_ghosts(&b.path).unwrap();
        assert!(again.is_empty(), "idempotent — no double close");
        let att = attribute(&b.path).unwrap();
        assert_eq!(
            att.docks[0].state, "ghost",
            "R4 — ghost in listings until reconciled"
        );
    }

    #[test]
    fn close_unknown_dock_fails_fail_closed() {
        let b = Builder::new("unknown");
        b.dock("real", "feat/x", "worktree", "/tmp/wt-x");
        let err = close_dock(&b.path, "nope").unwrap_err();
        assert!(err.contains("dock_not_found:nope"));
    }
}
