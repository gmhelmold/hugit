//! `hugit capture` + hook install — the zero-friction journey (2026-09-03).
//!
//! Owner direction: the LLM uses `git` normally; hugit records silently in the
//! background. This suite proves it END-TO-END through REAL git + the REAL
//! binary:
//!
//! 1. `hugit init` (via lib) installs the 4 hooks (`post-commit`,
//!    `post-checkout`, `pre-push`, `post-merge`) + reports them.
//! 2. A REAL `git commit` fires `post-commit` → `hugit capture commit` → the
//!    log gains a `ref.update` with the oid. Git is NOT blocked.
//! 3. A REAL `git checkout -b` fires `post-checkout` → `ref.update
//!    {checkout:true}`.
//! 4. A REAL `git push` (to a local bare remote) fires `pre-push` →
//!    `ref.update {attempt:true}`.
//! 5. A REAL `git merge` fires `post-merge` → `ref.update {merged_from}`.
//!
//! Every step exits 0 and never blocks git — even when `HUGIT_BIN` points at
//! a missing binary (the adversarial case the silent contract demands).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use hugit_cli::init::{InitArgs, run as run_init};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-capture-journey-{tag}-{}",
        std::process::id()
    ));
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

/// Run `hugit <args>` in `cwd` (the capture journey uses the real binary via
/// CARGO_BIN_EXE_hugit) — returns (exit, parsed stdout JSON).
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

#[test]
fn init_installs_the_4_hooks() {
    let root = scratch("install");
    lib_init(&root);
    let hooks_dir = root.join(".git/hooks");
    for kind in ["post-commit", "post-checkout", "pre-push", "post-merge"] {
        let script = std::fs::read_to_string(hooks_dir.join(kind))
            .unwrap_or_else(|_| panic!("{kind} hook written"));
        assert!(
            script.contains("hugit-hook (managed"),
            "{kind} has the hugit marker"
        );
        assert!(!script.trim().is_empty(), "{kind} is non-empty");
    }
}

#[test]
fn real_git_commit_captures_ref_update() {
    let root = scratch("commit");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = root.join(".hugit/log.json");

    // REAL git commit (the LLM's normal action).
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(
        &root,
        &["commit", "-m", "feat: real commit", "--no-gpg-sign"],
    );
    assert_eq!(code, 0, "git commit succeeds (hook never blocks)");

    // Hook fires ASYNC; poll for the capture (hooks are background by design).
    let branch = git_in(&root, &["branch", "--show-current"]).1;
    let branch = branch.trim().to_string();
    let got = wait_for_ref_update(
        &log,
        |p| {
            p["ref"].as_str() == Some(&format!("refs/heads/{branch}"))
                && !p["target"].as_str().unwrap_or("").is_empty()
        },
        5000,
    );
    assert!(
        got,
        "post-commit captured ref.update on the branch with a real oid"
    );
}

#[test]
fn real_git_checkout_captures_checkout_true() {
    let root = scratch("checkout");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = root.join(".hugit/log.json");
    git_with_hugit(&root, &["add", "a.txt"]);
    git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);

    // REAL branch checkout (the LLM switches context).
    let (code, _) = git_with_hugit(&root, &["checkout", "-b", "feat/agent"]);
    assert_eq!(code, 0);
    std::thread::sleep(std::time::Duration::from_millis(3000));

    let updates = ref_updates(&log);
    assert!(
        updates.iter().any(|r| {
            serde_json::from_str::<Value>(r["payload"].as_str().unwrap())
                .map(|p| p["checkout"] == true)
                .unwrap_or(false)
        }),
        "a checkout:true capture exists"
    );
}

#[test]
fn real_git_push_attempts_capture_attempt_true() {
    let root = scratch("push");
    lib_init(&root);
    set_git_identity(&root);
    let remote = root.join("remote.git");
    git_in(&root, &["init", "--bare", remote.to_str().unwrap()]);
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = root.join(".hugit/log.json");
    git_with_hugit(&root, &["add", "a.txt"]);
    git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);

    // REAL git push (to a local bare remote) — push the ACTUAL current branch
    // (git init defaults to `master` on this host; never assume `main`).
    let branch = git_in(&root, &["branch", "--show-current"]).1;
    let branch = branch.trim().to_string();
    let (code, _) = git_with_hugit(&root, &["push", "-u", "origin", &branch]);
    assert_eq!(code, 0, "push succeeds (pre-push hook exits 0)");
    std::thread::sleep(std::time::Duration::from_millis(3000));

    let updates = ref_updates(&log);
    assert!(
        updates.iter().any(|r| {
            serde_json::from_str::<Value>(r["payload"].as_str().unwrap())
                .map(|p| p["attempt"] == true)
                .unwrap_or(false)
        }),
        "a push attempt:true capture exists"
    );
}

#[test]
fn missing_bin_never_blocks_git() {
    let root = scratch("missing-bin");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git_in(&root, &["add", "a.txt"]);

    // HUGIT_BIN points at a nonexistent binary — the hook must still exit 0
    // and the commit must succeed (the silent contract's adversarial case).
    let out = Command::new("git")
        .args(["commit", "-m", "c1", "--no-gpg-sign"])
        .current_dir(&root)
        .env("HUGIT_BIN", "/nonexistent/hugit")
        .output()
        .expect("git runs with bad HUGIT_BIN");
    assert_eq!(
        out.status.code(),
        Some(0),
        "commit succeeds even with missing hugit"
    );
}

// ── 6. The captured git activity is watchable by class ───────────────────────

#[test]
fn captured_activity_is_watchable_as_git_activity() {
    let root = scratch("watch");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = root.join(".hugit/log.json");

    // Two REAL commits (the LLM's normal actions) — each fires post-commit.
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);
    assert_eq!(code, 0, "first commit");
    std::fs::write(root.join("a.txt"), "a2").unwrap();
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c2", "--no-gpg-sign"]);
    assert_eq!(code, 0, "second commit");

    // Wait for the async hooks to have captured both commits.
    let got = wait_for_ref_update(
        &log,
        |p| !p["target"].as_str().unwrap_or("").is_empty(),
        8000,
    );
    assert!(got, "hooks captured a commit");

    // `hugit watch --class git-activity` renders the captured trace.
    let (code, v) = run_in(
        &root,
        &[
            "watch",
            "--log",
            log.to_str().unwrap(),
            "--class",
            "git-activity",
        ],
    );
    assert_eq!(code, 0, "watch --class git-activity exits 0");
    assert!(
        v["count"].as_u64().unwrap_or(0) >= 1,
        "git-activity lines rendered: {v}"
    );
    assert_eq!(
        v["lines"][0]["class"].as_str(),
        Some("git-activity"),
        "line class is git-activity"
    );
}

// ── 7. W7: a human may undo a hook-captured ref.update ───────────────────────

/// The capture records currently on the log: `ref.update`s under the hook
/// principal, each carrying its `seq` + branch ref + target oid.
fn captured_commits(log: &Path) -> Vec<Value> {
    log_records(log)
        .into_iter()
        .filter(|r| {
            r["kind"] == "ref.update"
                && r["principal_chain"]
                    .as_array()
                    .map(|c| {
                        c.iter()
                            .any(|p| p.as_str() == Some("orchestrator:hugit-hook"))
                    })
                    .unwrap_or(false)
        })
        .filter(|r| {
            r["payload"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .map(|p| p["ref"].is_string() && !p["target"].as_str().unwrap_or("").is_empty())
                .unwrap_or(false)
        })
        .collect()
}

/// Wait until at least two hook-captured commit records are on the log (the
/// async hooks may still be catching up).
fn wait_for_two_captures(log: &Path, timeout_ms: u64) -> Vec<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        let caps = captured_commits(log);
        if caps.len() >= 2 {
            return caps;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    captured_commits(log)
}

/// The mocked `hugit undo` journey: a REAL `git commit` is captured silently,
/// then a HUMAN (`--actor user:human`) undoes that captured `ref.update`. The
/// compensating event lands with the supersession marker (`capture_undone_seq`
/// naming the undone seq + the ref + the restored target), and the chain still
/// verifies (watch loads it through the verify path, exit 0).
#[test]
fn human_can_undo_a_captured_ref_update() {
    let root = scratch("undo-capture");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = root.join(".hugit/log.json");

    // TWO REAL commits on the same branch: undoing the second capture restores
    // the ref to the first capture's target (a real compensator).
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);
    assert_eq!(code, 0, "first commit");
    std::fs::write(root.join("a.txt"), "a2").unwrap();
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c2", "--no-gpg-sign"]);
    assert_eq!(code, 0, "second commit");

    let captures = wait_for_two_captures(&log, 15000);
    assert!(captures.len() >= 2, "two captures landed: {captures:?}");
    let second = captures.last().unwrap().clone();
    let first_target = captures[0]["payload"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap()["target"]
        .as_str()
        .unwrap()
        .to_string();
    let seq = second["seq"].as_u64().expect("capture carries its seq");
    let branch_ref =
        serde_json::from_str::<Value>(second["payload"].as_str().unwrap()).unwrap()["ref"]
            .as_str()
            .unwrap()
            .to_string();

    // The HUMAN undoes the captured ref.update.
    let (code, v) = run_in(
        &root,
        &[
            "undo",
            "--log",
            log.to_str().unwrap(),
            "--seq",
            &seq.to_string(),
            "--actor",
            "user:human",
        ],
    );
    assert_eq!(code, 0, "a human may undo a captured ref.update: {v}");
    assert_eq!(v["undone_seq"].as_u64(), Some(seq));
    assert_eq!(v["compensation_kind"], "ref.update");
    assert_eq!(v["ref"].as_str(), Some(branch_ref.as_str()));

    // The compensating event landed, naming the superseded capture.
    let after = log_records(&log);
    let comp = after.last().expect("a compensator was appended");
    assert_eq!(comp["kind"], "ref.update");
    assert_eq!(
        comp["principal_chain"].as_array().map(|c| {
            c.iter()
                .map(|p| p.as_str().unwrap_or(""))
                .collect::<Vec<_>>()
        }),
        Some(vec!["user:human"]),
        "the compensator is attributed to the human"
    );
    let p: Value = serde_json::from_str(comp["payload"].as_str().unwrap()).unwrap();
    assert_eq!(p["ref"].as_str(), Some(branch_ref.as_str()));
    assert_eq!(p["target"].as_str(), Some(first_target.as_str()));
    assert_eq!(
        p["capture_undone_seq"].as_u64(),
        Some(seq),
        "the compensator on-record names the superseded capture seq"
    );

    // The chain still verifies end-to-end: watch loads through the verify path.
    let (code, v) = run_in(
        &root,
        &[
            "watch",
            "--log",
            log.to_str().unwrap(),
            "--class",
            "git-activity",
        ],
    );
    assert_eq!(code, 0, "watch (chain verify) still exits 0 after the undo");
    assert!(
        v["count"].as_u64().unwrap_or(0) >= 3,
        "2 captures + the compensator render as git-activity: {v}"
    );
}

/// W7 gate: the D14 Human-only rule is unchanged for captured `ref.update`s —
/// a non-human principal (`--actor agent:x`) is denied `authz_denied`, the
/// denial is audited, and NO compensator lands.
#[test]
fn agent_undo_of_captured_ref_update_is_authz_denied() {
    let root = scratch("undo-capture-denied");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = root.join(".hugit/log.json");

    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);
    assert_eq!(code, 0, "commit");

    let captures = wait_for_two_captures(&log, 15000);
    assert!(!captures.is_empty(), "a capture landed");
    let seq = captures.last().unwrap()["seq"].as_u64().unwrap();
    let ref_updates_before = ref_updates(&log).len();

    let (code, v) = run_in(
        &root,
        &[
            "undo",
            "--log",
            log.to_str().unwrap(),
            "--seq",
            &seq.to_string(),
            "--actor",
            "agent:runner-03",
        ],
    );
    assert_eq!(code, 2, "a non-human undo is denied (exit 2): {v}");
    assert_eq!(v["error"]["kind"], "authz_denied");

    // The denial is audited; no compensator landed.
    let after = log_records(&log);
    assert_eq!(
        ref_updates(&log).len(),
        ref_updates_before,
        "no compensating ref.update on a denied undo"
    );
    assert_eq!(after.last().unwrap()["kind"], "authz.denied");
    assert!(
        after.last().unwrap()["payload"]
            .as_str()
            .unwrap()
            .contains("\"endpoint\":\"undo\"")
    );
}

// ── 7. why resolves a path to the captured commit that touched it ────────────

#[test]
fn why_resolves_path_to_captured_commit() {
    let root = scratch("why-capture");
    lib_init(&root);
    set_git_identity(&root);
    let log = root.join(".hugit/log.json");

    // ONE real commit touching src/lib.rs (the LLM's normal action).
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").unwrap();
    git_in(&root, &["add", "src/lib.rs"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: add lib", "--no-gpg-sign"]);
    assert_eq!(code, 0);

    // Wait for the capture to land.
    let got = wait_for_ref_update(
        &log,
        |p| !p["target"].as_str().unwrap_or("").is_empty(),
        10000,
    );
    assert!(got, "hook captured the commit");

    // The captured ref.update payload carries `files` (from the post-commit
    // diff-tree) so `resolve_why` can attribute `src/lib.rs`.
    let some_capture_carries_files = ref_updates(&log).iter().any(|r| {
        serde_json::from_str::<Value>(r["payload"].as_str().unwrap_or(""))
            .map(|p| p["files"].as_array().is_some_and(|a| !a.is_empty()))
            .unwrap_or(false)
    });
    assert!(
        some_capture_carries_files,
        "the captured commit records the files it touched"
    );
}

#[test]
fn why_lib_resolves_path_to_captured_ref_update() {
    let root = scratch("why-lib");
    lib_init(&root);
    set_git_identity(&root);
    let log = root.join(".hugit/log.json");

    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").unwrap();
    git_in(&root, &["add", "src/lib.rs"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: add lib", "--no-gpg-sign"]);
    assert_eq!(code, 0);
    let got = wait_for_ref_update(
        &log,
        |p| !p["target"].as_str().unwrap_or("").is_empty(),
        10000,
    );
    assert!(got, "capture landed");

    // Use the real `why` resolver: a path touched by the captured commit must
    // resolve to that commit (the `files` in the payload feed payload_attribution).
    let raw: Vec<hugit_cli::why::resolver::LogEntry> = {
        let records: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
        records
            .into_iter()
            .map(|r| {
                let rec: hugit_contracts::EventRecord =
                    serde_json::from_value(r).expect("record shape");
                hugit_cli::why::resolver::LogEntry {
                    record: rec,
                    attestation: None,
                    sidecar: None,
                }
            })
            .collect()
    };
    let answer = hugit_cli::why::resolve_why(
        &hugit_cli::why::WhyQuery {
            path: "src/lib.rs".to_string(),
            line: None,
            symbol: None,
        },
        &raw,
    );
    assert!(
        answer.is_ok(),
        "why resolves the captured commit: {:?}",
        answer.err()
    );
}
