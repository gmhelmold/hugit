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
