//! WP-DOCK-3 — dock lifecycle + reconciliation (A4: cost↔commit by branch).
//!
//! Proves the acceptance hermetically AND end-to-end:
//!
//! - **A4 (close reconcile by branch)** — committing on a dock's branch makes
//!   its cost `matched`; cost without a landing on the branch is
//!   `investigated`; work with no measured cost is `unlabeled` (visible, never
//!   silently zero).
//! - **F5 (unit of insight = branch)** — two worktrees on the SAME branch
//!   aggregate under the branch; `Σ(docks on branch)` == branch total, cost
//!   never duplicated (R2).
//! - **R4 + L3 (ghosts reconciled)** — a dock whose gitdir vanished is a
//!   `ghost`; `dock reconcile` closes it on the durable log (idempotent); the
//!   listing stays `ghost` (the physical truth) but no open dock with a dead
//!   gitdir remains.
//! - **M5 (unbound intents)** — a commit on a branch with no per-branch dock
//!   links to the repo-scope dock.
//!
//! The e2e scenarios run REAL git worktrees whose post-checkout hook coins the
//! docks; cost samples land through the REAL spool→attest seam (WP-DOCK-4);
//! commits land through the REAL hook capture seam.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use hugit_cli::dock::close::reconcile_ghosts;
use hugit_cli::dock::insights::compute_insights;
use hugit_cli::dock::reconcile::{AttributionSummary, Bucket, attribute};
use hugit_cli::init::{InitArgs, run as run_init};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-dock3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lib_init(dir: &Path) {
    let code = run_init(InitArgs {
        dir: Some(dir.to_path_buf()),
    });
    assert_eq!(code, std::process::ExitCode::SUCCESS, "lib init exits 0");
}

fn git_in(cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn git_with_hugit(cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("HUGIT_BIN", env!("CARGO_BIN_EXE_hugit"))
        .output()
        .expect("git runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn wait_until(timeout_ms: u64, f: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    f()
}

/// Resolve the SHARED canonical log path (the main repo's `.hugit/log.json`)
/// from any cwd — worktree-safe, mirrors the hooks' `--git-common-dir` logic.
fn shared_log(cwd: &Path) -> PathBuf {
    let common = String::from_utf8_lossy(
        &Command::new("git")
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .current_dir(cwd)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    PathBuf::from(common)
        .join("..")
        .join(".hugit")
        .join("log.json")
}

/// Commit on the current branch through the REAL post-commit hook (captures a
/// `ref.update` commit-kind record onto the canonical log). Waits for the
/// async capture to land — a commit returns only when the record is durable.
fn commit(cwd: &Path, msg: &str) -> String {
    std::fs::write(cwd.join("tmp.txt"), format!("{msg}-{}", std::process::id())).unwrap();
    let (rc, out) = git_with_hugit(cwd, &["add", "-A"]);
    assert_eq!(rc, 0, "add: {out}");
    let (rc, out) = git_with_hugit(cwd, &["commit", "-q", "-m", msg]);
    assert_eq!(rc, 0, "commit: {out}");
    let branch = String::from_utf8_lossy(
        &Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(cwd)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    let log = shared_log(cwd);
    // Wait for the ASYNC post-commit capture to land: a commit-kind ref.update
    // on THIS branch (payload has target+branch, no checkout/attempt/merge).
    let landed = wait_until(25000, || {
        let rows: Vec<Value> = std::fs::read(&log)
            .map(|b| serde_json::from_slice(&b).unwrap_or_default())
            .unwrap_or_default();
        rows.iter().any(|r| {
            r.get("kind").and_then(|k| k.as_str()) == Some("ref.update")
                && r.get("payload")
                    .and_then(Value::as_str)
                    .map(|p| {
                        let v: Value = serde_json::from_str(p).unwrap_or(Value::Null);
                        v.get("branch").and_then(Value::as_str) == Some(branch.as_str())
                            && v.get("target").is_some()
                            && v.get("checkout").is_none()
                            && v.get("attempt").is_none()
                            && v.get("merged_from").is_none()
                    })
                    .unwrap_or(false)
        })
    });
    assert!(
        landed,
        "post-commit capture lands (async hook) on {branch}; log:\n{}",
        std::fs::read_to_string(&log).unwrap_or_else(|_| "<no log>".into())
    );
    String::from_utf8_lossy(
        &Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(cwd)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string()
}

/// Push a cost sample through the REAL spool→attest seam for a dock id.
fn land_cost(log: &Path, spool_dir: &Path, dock_id: &str, cost_usd_micros: u64, run: &str) {
    let spool = hugit_cli::dock::spool::CostSpool::new(spool_dir.to_path_buf());
    let sample = hugit_contracts::cost_sample::CostSampleV1 {
        dock_id: dock_id.to_string(),
        model: "claude-test".to_string(),
        input_tokens: 1,
        output_tokens: 1,
        cost_usd_micros,
        ts_ms: 1_700_000_000_000,
        run_id: run.to_string(),
    };
    spool.push(&sample).expect("spool push");
    hugit_cli::dock::attest::flush_dock(log, &spool, dock_id).expect("attest lands");
}

// ── Hermetic: A4 matched/investigated/unlabeled + F5 aggregation ────────────

#[test]
fn hermetic_a4_f5_and_residuals() {
    let dir = scratch("hermetic-a4");
    let log = dir.join(".hugit/log.json");
    std::fs::create_dir_all(dir.join(".hugit")).unwrap();
    std::fs::write(&log, "[]").unwrap();
    let spool_dir = dir.join(".hugit/cost-spool");
    std::fs::create_dir_all(&spool_dir).unwrap();

    let builder = TestLog {
        path: &log,
        principal: "orchestrator:hugit-hook".to_string(),
    };
    // A dock that lands (cost + commit) → matched.
    let dock_a = "dock-a";
    builder.dock(dock_a, "feat/landed", "worktree", "/tmp/wt-landed");
    // A dock that spends but never lands → investigated (A4, R3).
    let dock_b = "dock-b";
    builder.dock(dock_b, "feat/broken", "worktree", "/tmp/wt-broken");
    // A dock with a commit but zero measured cost → unlabeled (R3).
    let dock_c = "dock-c";
    builder.dock(dock_c, "feat/free", "worktree", "/tmp/wt-free");

    land_cost(&log, &spool_dir, dock_a, 100_000, "run-a");
    land_cost(&log, &spool_dir, dock_b, 999_000, "run-b");
    builder.commit("feat/landed", "aaaa");
    builder.commit("feat/free", "cccc");

    let a = attribute(&log).unwrap();
    assert_eq!(bucket_of(&a, dock_a), Bucket::Matched);
    assert_eq!(bucket_of(&a, dock_b), Bucket::Investigated);
    assert_eq!(cost_of(&a, dock_b), 999_000);
    // R1 — the buckets sum to EVERY sample, nothing double-counted, nothing
    // hidden (reconciled/unlabeled residuals visible).
    assert_eq!(
        matched(&a) + investigated(&a) + a.reconciled_cost_usd_micros + a.unlabeled_cost_usd_micros,
        cost_sum(&a)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── e2e: two worktrees on the SAME branch aggregate, never duplicate (R2/F5) ─
//
// NOTE (git physical constraint): a git branch can be checked out in at most
// ONE worktree (`git worktree add wt2 <branch>` → fatal "already used by
// worktree"). Multi-head on one branch is therefore a LOG-level arrangement:
// two `dock.record` entries carrying the SAME branch string, exactly what F5
// aggregates. Worktree #1 is coined by the REAL post-checkout hook; worktree
// #2's dock is coined through the SAME seam the hook invokes (`coin_dock`) on
// a second real gitdir — proving the aggregation + A4 end to end.

#[test]
fn e2e_two_worktrees_same_branch_aggregate() {
    let dir = scratch("e2e-two");
    let (rc, _) = git_in(&dir, &["init", "-q", "-b", "main"]);
    assert_eq!(rc, 0);
    git_in(&dir, &["config", "user.email", "t@t"]);
    git_in(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git_with_hugit(&dir, &["add", "-A"]);
    git_with_hugit(&dir, &["commit", "-q", "-m", "init"]);
    lib_init(&dir);
    std::thread::sleep(Duration::from_millis(400)); // hooks settle

    // Worktree #1 — the REAL hook coins its dock on `feat/deploy`.
    let wt1 = dir.join("wt-one");
    let (rc, out) = git_with_hugit(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/deploy",
            wt1.to_str().unwrap(),
        ],
    );
    assert_eq!(rc, 0, "wt1: {out}");

    let log = dir.join(".hugit/log.json");
    // Worktree #2 — its gitdir is real; the dock is coined through the exact
    // seam the hook uses, on the SAME branch (the multi-head arrangement).
    let wt2_gitdir = dir.join(".git/worktrees/wt-sim");
    std::fs::create_dir_all(&wt2_gitdir).unwrap();
    let dock2 = hugit_cli::dock::coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &wt2_gitdir.to_string_lossy(),
        branch: "feat/deploy",
        charter: "deploy pipeline",
        origin: "worktree",
        env_dock_id: None,
    })
    .expect("worktree #2 dock coined (same seam as the hook)");

    let both = wait_until(25000, || {
        attribute(&log)
            .map(|a| a.docks.iter().filter(|d| d.branch == "feat/deploy").count())
            .unwrap_or(0)
            == 2
    });
    assert!(
        both,
        "both worktrees coined a dock on feat/deploy; logs:\n{}",
        std::fs::read_to_string(dir.join(".hugit/hooks.log"))
            .unwrap_or_else(|_| "<no hooks.log>".into())
    );

    // Cost lands on BOTH docks (the multi-head spend) + a commit in wt1.
    let spool_dir = dir.join(".hugit/cost-spool");
    std::fs::create_dir_all(&spool_dir).unwrap();
    let a = attribute(&log).unwrap();
    let first_dock = a
        .docks
        .iter()
        .find(|d| d.branch == "feat/deploy" && d.dock_id != dock2)
        .unwrap();
    land_cost(&log, &spool_dir, &first_dock.dock_id, 200_000, "run-1");
    land_cost(&log, &spool_dir, &dock2, 50_000, "run-2");
    commit(&wt1, "deploy step");

    let a = attribute(&log).unwrap();
    let deploy: Vec<_> = a
        .docks
        .iter()
        .filter(|d| d.branch == "feat/deploy")
        .collect();
    assert_eq!(deploy.len(), 2, "two physical docks, one branch (F5)");
    let branch_cost: u64 = deploy.iter().map(|d| d.cost_usd_micros).sum();
    assert_eq!(
        branch_cost, 250_000,
        "R2 — multi-head aggregated, never duplicated"
    );
    assert!(
        deploy.iter().all(|d| d.bucket == Bucket::Matched),
        "the branch's landing explains BOTH docks' cost (A4)"
    );

    // F5 — the branch projection shows exactly the dock sum as its cost.
    let doc = compute_insights(&log, Some("feat/deploy")).unwrap();
    assert_eq!(doc.branches.len(), 1);
    assert_eq!(
        doc.branches[0].cost_usd_micros, 250_000,
        "branch total == Σ docks"
    );
    assert_eq!(
        doc.branches[0].docks.len(),
        2,
        "two physical sub-units under the branch"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── e2e: ghost close (L3/R4) through the REAL hook-coined dock ─────────────

#[test]
fn e2e_ghost_is_reconciled_onto_the_durable_log() {
    let dir = scratch("e2e-ghost");
    let (rc, _) = git_in(&dir, &["init", "-q", "-b", "main"]);
    assert_eq!(rc, 0);
    git_in(&dir, &["config", "user.email", "t@t"]);
    git_in(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git_with_hugit(&dir, &["add", "-A"]);
    git_with_hugit(&dir, &["commit", "-q", "-m", "init"]);
    lib_init(&dir);

    let wt = dir.join("wt-doomed");
    let (rc, out) = git_with_hugit(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature/doomed",
            wt.to_str().unwrap(),
        ],
    );
    assert_eq!(rc, 0, "worktree lands: {out}");
    let log = dir.join(".hugit/log.json");
    assert!(
        wait_until(25000, || {
            attribute(&log)
                .map(|a| a.docks.iter().any(|d| d.branch == "feature/doomed"))
                .unwrap_or(false)
        }),
        "the doomed worktree coined its dock"
    );

    // Kill the worktree — the gitdir disappears — the dock becomes a ghost
    // with NO explicit close hook (git worktree remove).
    let (rc, out) = git_with_hugit(
        &dir,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    assert_eq!(rc, 0, "wt remove: {out}");

    // L3 — `dock reconcile` closes the ghost on the durable log (idempotent);
    // R4 — the listing still tells the physical truth (ghost).
    let closed = reconcile_ghosts(&log).expect("reconcile runs");
    assert_eq!(closed.len(), 1, "L3 — the ghost is closed");
    let c = &closed[0];
    assert!(c.cost_usd_micros == 0 && c.commit_count == 0);
    assert_eq!(
        c.bucket_name, "unlabeled",
        "no cost/commits → honest unlabeled"
    );

    let again = reconcile_ghosts(&log).expect("reconcile idempotent");
    assert!(again.is_empty(), "no double close");
    let a = attribute(&log).unwrap();
    assert_eq!(a.docks.len(), 1);
    assert_eq!(a.docks[0].state, "ghost", "R4 — physical truth in listings");
    let closed_count = repo_records(&log, "dock.close").len();
    assert_eq!(closed_count, 1, "exactly one durable close record");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── e2e: M5 — unbound intents link to the repo-scope dock ──────────────────

#[test]
fn e2e_unbound_commit_links_to_repo_scope_dock() {
    let dir = scratch("e2e-m5");
    let (rc, _) = git_in(&dir, &["init", "-q", "-b", "main"]);
    assert_eq!(rc, 0);
    git_in(&dir, &["config", "user.email", "t@t"]);
    git_in(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git_with_hugit(&dir, &["add", "-A"]);
    git_with_hugit(&dir, &["commit", "-q", "-m", "init"]);
    lib_init(&dir);
    std::thread::sleep(Duration::from_millis(400));

    let log = dir.join(".hugit/log.json");
    // The repo-scope dock is auto-coined on first RESOLVE (M5, WP-DOCK-2) — a
    // resolve from the main worktree (origin=="repo") triggers it.
    let resolved = hugit_cli::dock::resolve::resolve(&dir, None, None);
    assert!(resolved.is_ok(), "resolve from the main worktree works");
    let has_repo_scope = attribute(&log)
        .map(|a| a.docks.iter().any(|d| d.origin == "repo"))
        .unwrap();
    assert!(
        has_repo_scope,
        "M5 — repo-scope dock auto-coined on first resolve"
    );

    // A straight commit on the MAIN worktree (already the dock root) — its
    // capture is an unbound intent in the DOCK-2 sense when the branch has no
    // per-branch dock; it must link to the repo-scope dock (M5).
    commit(&dir, "direct commit on main");
    let a = attribute(&log).unwrap();
    assert!(
        a.repo_scope_linked_branches.iter().any(|b| b == "main")
            || a.docks
                .iter()
                .any(|d| d.origin == "repo" && d.commit_count >= 1),
        "unbound intents link to the repo-scope dock (M5)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── helpers ────────────────────────────────────────────────────────────────

struct TestLog<'a> {
    path: &'a Path,
    principal: String,
}

impl TestLog<'_> {
    fn dock(&self, id: &str, branch: &str, origin: &str, gitdir: &str) {
        self.record(
            "dock.record",
            json!({
                "dock_id": id, "gitdir": gitdir, "branch": branch,
                "charter": "test", "charter_derived": true, "state": "open",
                "origin": origin, "created_ts": 1000, "pid": 1,
            }),
        );
    }

    fn commit(&self, branch: &str, target: &str) {
        self.record(
            "ref.update",
            json!({"ref": format!("refs/heads/{branch}"), "target": target, "branch": branch}),
        );
    }

    fn record(&self, kind: &str, payload: Value) {
        let mut log = hugit_cli::checks::load_event_log(self.path).expect("log loads");
        let ps = payload.to_string();
        let cap = hugit_refstore::canonical_json(&ps).unwrap_or(ps);
        log.append_authorized(
            hugit_refstore::authz::PrincipalClass::Orchestrator,
            hugit_refstore::authz::Endpoint::Push,
            kind.to_string(),
            vec![self.principal.clone()],
            cap,
            1000,
        )
        .expect("append authorized");
        let bytes = serde_json::to_vec_pretty(log.records()).unwrap();
        hugit_cli::pr::filelock::atomic_write(self.path, &bytes).unwrap();
    }
}

fn bucket_of(a: &AttributionSummary, id: &str) -> Bucket {
    a.docks
        .iter()
        .find(|d| d.dock_id == id)
        .map(|d| d.bucket)
        .unwrap_or(Bucket::Unlabeled)
}

fn cost_of(a: &AttributionSummary, id: &str) -> u64 {
    a.docks
        .iter()
        .find(|d| d.dock_id == id)
        .map(|d| d.cost_usd_micros)
        .unwrap_or(0)
}

fn cost_sum(a: &AttributionSummary) -> u64 {
    a.docks.iter().map(|d| d.cost_usd_micros).sum::<u64>()
        + a.reconciled_cost_usd_micros
        + a.unlabeled_cost_usd_micros
}

fn matched(a: &AttributionSummary) -> u64 {
    a.docks
        .iter()
        .filter(|d| d.bucket == Bucket::Matched)
        .map(|d| d.cost_usd_micros)
        .sum()
}

fn investigated(a: &AttributionSummary) -> u64 {
    a.docks
        .iter()
        .filter(|d| d.bucket == Bucket::Investigated)
        .map(|d| d.cost_usd_micros)
        .sum()
}

fn repo_records(log: &Path, kind: &str) -> Vec<Value> {
    let bytes = std::fs::read(log).unwrap_or_else(|_| b"[]".to_vec());
    serde_json::from_slice::<Vec<Value>>(&bytes)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.get("kind").and_then(|k| k.as_str()) == Some(kind))
        .collect()
}
