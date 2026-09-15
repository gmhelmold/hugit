#![cfg(unix)]

//! `hugit fleet` — the FULL-FLEET journey: the fleet operator sees what N
//! agents did with zero friction (W4, 2026-09-03).
//!
//! The prove-by-using evidence for the whole git-local product: the silent
//! hooks (#336) capture real agent git activity; `hugit fleet` (W1) folds the
//! raw git trace (`ref.update`, the same classification `watch` uses) into the
//! machine-readable fleet state; the operator runs ONE verb and sees BOTH
//! agents' branches.
//!
//! The journey:
//!
//! 1. `hugit init` (via lib) installs the silent hooks.
//! 2. TWO agents/workspaces work in parallel: REAL `git checkout -b agent-a`,
//!    REAL commit; REAL `git checkout -b agent-b`, REAL commit on a different
//!    file. HUGIT_BIN points at the real binary, so the hooks capture.
//! 3. The operator runs `hugit fleet --log …` → `git_activity` carries entries
//!    for BOTH branches and `git_activity_count >= 2`.
//! 4. `hugit watch --log … --class git-activity` renders the trace.
//! 5. Zero friction: every git op exits 0; captured events carry the
//!    `orchestrator:hugit-hook` principal — never blocked, never mixed up with
//!    explicit orchestration acts.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use hugit_cli::init::{InitArgs, run as run_init};

fn scratch(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("hugit-fleet-journey-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit init` via the library (the binary verb is X5-reserved).
fn lib_init(dir: &Path) -> Value {
    let code = run_init(InitArgs {
        dir: Some(dir.to_path_buf()),
    });
    assert_eq!(code, std::process::ExitCode::SUCCESS, "lib init exits 0");
    Value::Null
}

/// Run git with HUGIT_BIN pointing at THIS test build's binary, so the hook's
/// `${HUGIT_BIN:-hugit}` resolves to the real binary under test.
fn git_with_hugit(cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("HUGIT_BIN", env!("CARGO_BIN_EXE_hugit"))
        .output()
        .expect("git runs with HUGIT_BIN set");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Run `hugit <args>` in `cwd` (the real binary via CARGO_BIN_EXE_hugit) —
/// returns (exit, parsed stdout JSON).
fn run_in(cwd: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hugit runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
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

fn log_records(log: &Path) -> Vec<Value> {
    let bytes = std::fs::read(log).expect("log readable");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| vec![])
}

/// Poll the log until a `ref.update` payload satisfying `pred` appears (or
/// timeout). Hooks are ASYNC by design; a fixed sleep is flaky under CI load.
fn wait_for_ref_update(log: &Path, pred: impl Fn(&Value) -> bool, timeout_ms: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        for r in ref_updates(log) {
            if let Some(p) = r["payload"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                && pred(&p)
            {
                return true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// Configure a local git identity (name/email) so `git commit` works on a
/// fresh CI runner where the global config is absent.
fn set_git_identity(dir: &Path) {
    git_in(dir, &["config", "user.name", "hugit-test"]);
    git_in(dir, &["config", "user.email", "hugit-test@example.com"]);
}

fn ref_updates(log: &Path) -> Vec<Value> {
    log_records(log)
        .into_iter()
        .filter(|r| r["kind"] == "ref.update")
        .collect()
}

/// The FULL-FLEET journey: one repo, TWO agents working on parallel branches,
/// the operator sees BOTH in `hugit fleet` + `hugit watch` — all git ops
/// friction-free and every capture hook-principalled.
#[test]
fn fleet_operator_sees_agent_fleet_activity() {
    let root = scratch("fleet");
    lib_init(&root);
    set_git_identity(&root);
    let log = root.join(".git/hugit/event-log.json");

    // Seed a base commit so both agents build on a shared start point.
    std::fs::write(root.join("base.txt"), "base").unwrap();
    let (code, _) = git_with_hugit(&root, &["add", "base.txt"]);
    assert_eq!(code, 0, "git add exits 0");
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "base", "--no-gpg-sign"]);
    assert_eq!(code, 0, "base commit exits 0");

    // Agent A: real branch + real commit — the LLM's normal actions.
    let (code, _) = git_with_hugit(&root, &["checkout", "-b", "agent-a"]);
    assert_eq!(code, 0, "agent-a branch creation exits 0");
    std::fs::write(root.join("agent-a.txt"), "agent-a work").unwrap();
    let (code, _) = git_with_hugit(&root, &["add", "agent-a.txt"]);
    assert_eq!(code, 0, "agent-a add exits 0");
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "agent-a work", "--no-gpg-sign"]);
    assert_eq!(code, 0, "agent-a commit exits 0");

    // Agent B: a different branch + a different file — parallel workspaces.
    let (code, _) = git_with_hugit(&root, &["checkout", "-b", "agent-b"]);
    assert_eq!(code, 0, "agent-b branch creation exits 0");
    std::fs::write(root.join("agent-b.txt"), "agent-b work").unwrap();
    let (code, _) = git_with_hugit(&root, &["add", "agent-b.txt"]);
    assert_eq!(code, 0, "agent-b add exits 0");
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "agent-b work", "--no-gpg-sign"]);
    assert_eq!(code, 0, "agent-b commit exits 0");

    // Hooks fire ASYNC; wait for BOTH agents' captures to land.
    let got_a = wait_for_ref_update(
        &log,
        |p| {
            p["branch"].as_str() == Some("agent-a")
                && !p["target"].as_str().unwrap_or("").is_empty()
        },
        25000,
    );
    assert!(got_a, "agent-a commit captured");
    let got_b = wait_for_ref_update(
        &log,
        |p| {
            p["branch"].as_str() == Some("agent-b")
                && !p["target"].as_str().unwrap_or("").is_empty()
        },
        25000,
    );
    assert!(got_b, "agent-b commit captured");

    // The operator's ONE verb: `hugit fleet --log …` — BOTH agents visible.
    let (code, v) = run_in(&root, &["fleet", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "hugit fleet exits 0");
    let activity = v["git_activity"].as_array().expect("git_activity array");
    assert!(
        activity.iter().any(|e| e["branch"] == "agent-a"),
        "fleet sees agent-a: {v}"
    );
    assert!(
        activity.iter().any(|e| e["branch"] == "agent-b"),
        "fleet sees agent-b: {v}"
    );
    assert!(
        v["git_activity_count"].as_u64().unwrap_or(0) >= 2,
        "git_activity_count >= 2: {v}"
    );
    // Ordered by log seq — the trace reads in the order the agents acted.
    let seqs: Vec<u64> = activity
        .iter()
        .map(|e| e["seq"].as_u64().unwrap_or(0))
        .collect();
    assert_eq!(
        seqs,
        {
            let mut s = seqs.clone();
            s.sort_unstable();
            s
        },
        "git_activity entries are in log order"
    );

    // `hugit watch --class git-activity` renders the same trace.
    let (code, w) = run_in(
        &root,
        &[
            "watch",
            "--log",
            log.to_str().unwrap(),
            "--class",
            "git-activity",
        ],
    );
    assert_eq!(code, 0, "hugit watch exits 0");
    assert!(
        w["count"].as_u64().unwrap_or(0) >= 2,
        "watch renders the git-activity lines: {w}"
    );
    assert_eq!(
        w["lines"][0]["class"].as_str(),
        Some("git-activity"),
        "line class is git-activity"
    );

    // Zero friction, proven at the byte level: every captured event is
    // hook-principalled — none mistaken for an explicit orchestration act.
    let records = log_records(&log);
    let hook_events: Vec<&Value> = records
        .iter()
        .filter(|r| r["kind"] == "ref.update")
        .collect();
    assert!(hook_events.len() >= 2, "at least both commits captured");
    for r in &hook_events {
        let chain = r["principal_chain"].as_array().expect("principal_chain");
        assert!(
            chain
                .iter()
                .any(|p| p.as_str() == Some("orchestrator:hugit-hook")),
            "captured event carries the hook principal: {r}"
        );
    }
}
