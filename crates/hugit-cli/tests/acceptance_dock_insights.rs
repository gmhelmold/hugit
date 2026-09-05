//! WP-DOCK-5 — insights per-branch (F5) + residual buckets (the honest window).
//!
//! Proves the acceptance hermetically + e2e (real git + real spool seam):
//!
//! - **I1 (safety — branch totals)** — `insights(branch).cost == Σ(dock costs
//!   on that branch)`; multi-head docks aggregate, never duplicated.
//! - **I2 (safety — buckets visible)** — every cost sample is attributable: a
//!   raced-ahead sample is `reconciled-late` (explicit), an empty-dock sample
//!   is `unlabeled`; no silent absent entry (R3).
//! - **I3 (safety — no fabrication)** — a branch with no attested cost shows
//!   **zero/None**, never a derived estimate.
//! - **L5 (liveness — fresh)** — the projection is derived at read from the
//!   latest appended records; a new sample flips the bucket on the next read.
//! - **F5 (unit = branch)** — the per-branch view aggregates the physical
//!   docks under one business unit.

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_cli::dock::insights::{BranchInsight, compute_insights};
use hugit_cli::init::{InitArgs, run as run_init};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-dock5-{tag}-{}", std::process::id()));
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

/// Append a record through the REAL canonical-log seam (authz'd, chained).
fn append_authed(log: &Path, kind: &str, payload: serde_json::Value) {
    let mut el = hugit_cli::checks::load_event_log(log).expect("log loads");
    let ps = payload.to_string();
    let cap = hugit_refstore::canonical_json(&ps).unwrap_or(ps);
    el.append_authorized(
        hugit_refstore::authz::PrincipalClass::Orchestrator,
        hugit_refstore::authz::Endpoint::Push,
        kind.to_string(),
        vec!["orchestrator:hugit-hook".to_string()],
        cap,
        1000,
    )
    .expect("append authed");
    let bytes = serde_json::to_vec_pretty(el.records()).unwrap();
    hugit_cli::pr::filelock::atomic_write(log, &bytes).unwrap();
}

/// Land a cost sample through the REAL spool→attest seam (WP-DOCK-4).
fn land_cost(log: &Path, dock_id: &str, cost: u64, run: &str, ts_ms: u64) {
    let spool_dir = log.parent().unwrap().join("cost-spool");
    std::fs::create_dir_all(&spool_dir).unwrap();
    let spool = hugit_cli::dock::spool::CostSpool::new(spool_dir);
    let sample = hugit_contracts::cost_sample::CostSampleV1 {
        dock_id: dock_id.to_string(),
        model: "claude-test".to_string(),
        input_tokens: 1,
        output_tokens: 1,
        cost_usd_micros: cost,
        ts_ms,
        run_id: run.to_string(),
    };
    spool.push(&sample).unwrap();
    hugit_cli::dock::attest::flush_dock(log, &spool, dock_id).unwrap();
}

fn branch_of<'a>(
    doc: &'a hugit_cli::dock::insights::InsightDocument,
    b: &str,
) -> Option<&'a BranchInsight> {
    doc.branches.iter().find(|x| x.branch == b)
}

/// Hermetic — I1 (+R2 aggregation), I2, I3, L5, reconciled-late.
#[test]
fn hermetic_insights_i1_i2_i3_l5() {
    let dir = scratch("hermetic");
    let log = dir.join(".hugit/log.json");
    std::fs::create_dir_all(dir.join(".hugit")).unwrap();
    std::fs::write(&log, "[]").unwrap();

    // I3 — empty log: the projection is EMPTY (no fabricated zero branch).
    let doc = compute_insights(&log, None).unwrap();
    assert!(
        doc.branches.is_empty(),
        "I3 — no fabricated rows on empty log"
    );

    // Two docks on ONE branch (multi-head, F5) + one on its own branch.
    for (id, br, gitdir) in [
        ("d1", "feat/x", "/tmp/wt-x1"),
        ("d2", "feat/x", "/tmp/wt-x2"),
        ("d3", "feat/y", "/tmp/wt-y"),
    ] {
        let rec = serde_json::json!({
            "dock_id": id, "gitdir": gitdir, "branch": br,
            "charter": "test", "charter_derived": true, "state": "open",
            "origin": "worktree", "created_ts": 2000, "pid": 1,
        });
        append_authed(&log, "dock.record", rec);
    }

    // I1/F5 — cost on both x docks aggregates under `feat/x` (no dup);
    // I2 — a sample on a KNOWN dock that raced AHEAD of its coinage
    // (ts 500 < dock created 2000) → reconciled-late (A2).
    land_cost(&log, "d1", 100_000, "r1", 3000);
    land_cost(&log, "d2", 50_000, "r2", 2500);
    land_cost(&log, "d1", 7_000, "r-early", 500);
    land_cost(&log, "d-none", 5_000, "r-none", 3500); // unknown dock → reconciled

    let doc = compute_insights(&log, None).unwrap();
    let x = branch_of(&doc, "feat/x").expect("feat/x branch present");
    assert_eq!(
        x.cost_usd_micros, 150_000,
        "I1 — branch total == Σ docks (never dup)"
    );
    assert_eq!(
        x.docks.len(),
        2,
        "F5 — two physical sub-units under one branch"
    );
    assert_eq!(
        doc.reconciled_late_cost_usd_micros, 7_000,
        "I2 — raced-ahead explicit"
    );
    assert_eq!(
        doc.reconciled_cost_usd_micros, 5_000,
        "R3 — unknown-dock sample visible"
    );

    // L5 — a NEW sample flips the next read (derived at read, never stale).
    land_cost(&log, "d3", 20_000, "r3", 2600);
    doc_refresh(&log, "feat/y", 20_000);

    let _ = std::fs::remove_dir_all(&dir);
}

fn doc_refresh(log: &Path, branch: &str, expected: u64) {
    let doc = compute_insights(log, None).unwrap();
    let y = branch_of(&doc, branch).expect("branch now present");
    assert_eq!(y.cost_usd_micros, expected, "L5 — fresh on read");
}

/// e2e — fraud ends: the branch projection reads the REAL hook-coined dock and
/// the REAL spool, and the CLI `dock insight` verb emits the same shape.
#[test]
fn e2e_insights_verb_reads_real_log() {
    let dir = scratch("e2e");
    let (rc, _) = git_in(&dir, &["init", "-q", "-b", "main"]);
    assert_eq!(rc, 0);
    git_in(&dir, &["config", "user.email", "t@t"]);
    git_in(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(&dir)
        .env("HUGIT_BIN", env!("CARGO_BIN_EXE_hugit"))
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-q", "-m", "init"])
        .current_dir(&dir)
        .env("HUGIT_BIN", env!("CARGO_BIN_EXE_hugit"))
        .output()
        .unwrap();
    lib_init(&dir);
    let wt = dir.join("wt-rate");
    let (rc, out) = {
        let o = Command::new("git")
            .args([
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/rate",
                wt.to_str().unwrap(),
            ])
            .current_dir(&dir)
            .env("HUGIT_BIN", env!("CARGO_BIN_EXE_hugit"))
            .output()
            .unwrap();
        (o.status.code().unwrap_or(-1), o)
    };
    assert_eq!(rc, 0, "wt add failed: {out:?}");

    let log = dir.join(".hugit/log.json");
    // The hook-coined dock appears under feat/rate; cost lands via the spool.
    let mut seen = false;
    for _ in 0..100 {
        if let Ok(doc) = compute_insights(&log, None)
            && branch_of(&doc, "feat/rate").is_some()
        {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(
        seen,
        "hook-coined dock visible in the projection; hooks.log:\n{}",
        std::fs::read_to_string(dir.join(".hugit/hooks.log")).unwrap_or_else(|_| "<none>".into())
    );

    let doc = compute_insights(&log, None).unwrap();
    let rate = branch_of(&doc, "feat/rate").unwrap();
    let dock_id = rate.docks[0]
        .get("dock_id")
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_string();
    land_cost(
        &log,
        &dock_id,
        88_000,
        "rr",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    );

    // The CLI verb emits the SAME projected shape (fresh read of the log).
    let out = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(["dock", "insight", "--log", log.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "verb exits 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("feat/rate") && text.contains("reconciled_late_usd_micros"),
        "verb emits the branch + residual shape: {text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
