#![cfg(unix)]

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

/// Serializes the hook journeys. They run REAL `git commit`s whose async hooks
/// (background `hugit capture` children) compete for the log FileLock + system
/// resources; running them in PARALLEL makes the async capture flaky under
/// load (the bundle gate). One journey at a time — held for the whole test.
fn hook_serial() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn lock_hook_journey() -> std::sync::MutexGuard<'static, ()> {
    hook_serial()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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

fn runtime_log(repo: &Path) -> PathBuf {
    let out = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(repo)
        .output()
        .expect("resolve Git common directory");
    assert!(out.status.success());
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("hugit/event-log.json")
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
                && drain_is_idle(log)
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
#[ignore = "superseded by acceptance_attach for current attach API"]
fn attach_then_health_is_active_from_nested_directory() {
    let _serial = lock_hook_journey();
    let root = scratch("attach-health");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    let nested = root.join("nested");
    std::fs::create_dir(&nested).expect("create nested directory");

    let (code, attached) = run_in(&nested, &["attach"]);
    assert_eq!(code, 0, "attach succeeds: {attached}");
    assert_eq!(
        attached["initialized"], true,
        "attach creates canonical log"
    );

    let (code, health) = run_in(&nested, &["health"]);
    assert_eq!(code, 0, "health succeeds: {health}");
    assert_eq!(health["mode"], "active", "all hooks are managed: {health}");
    assert_eq!(
        health["log"]["state"], "valid",
        "log chain is valid: {health}"
    );
}

#[test]
fn detach_preserves_marker_text_inside_foreign_hook() {
    let _serial = lock_hook_journey();
    let root = scratch("detach-foreign-marker");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    assert_eq!(run_in(&root, &["attach"]).0, 0, "attach succeeds");
    let foreign = root.join(".git/hooks/post-commit");
    let bytes = b"#!/bin/sh\n# hugit-hook (managed by hugit init)\nprintf foreign\n";
    std::fs::write(&foreign, bytes).expect("write foreign marker hook");

    let (code, detached) = run_in(&root, &["detach"]);
    assert_eq!(code, 0, "detach succeeds: {detached}");
    assert!(
        detached["preserved"]
            .as_array()
            .is_some_and(|hooks| hooks.iter().any(|hook| hook == "post-commit")),
        "foreign marker hook is preserved: {detached}"
    );
    assert_eq!(std::fs::read(&foreign).expect("read foreign hook"), bytes);
}

#[cfg(unix)]
#[test]
fn health_marks_nonexecutable_managed_hook_partial() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = lock_hook_journey();
    let root = scratch("health-mode-bit");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    assert_eq!(run_in(&root, &["attach"]).0, 0, "attach succeeds");
    let hook = root.join(".git/hooks/post-commit");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644))
        .expect("remove executable bit");

    let (code, health) = run_in(&root, &["health"]);
    assert_eq!(code, 0, "health succeeds: {health}");
    assert_eq!(health["mode"], "partial");
    assert_eq!(health["hooks"]["post_commit"]["state"], "non_executable");
}

#[test]
#[ignore = "superseded by acceptance_attach for current attach API"]
fn attach_from_linked_worktree_uses_shared_log_root() {
    let _serial = lock_hook_journey();
    let root = scratch("attach-linked-worktree");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    set_git_identity(&root);
    assert_eq!(
        git_in(&root, &["commit", "--allow-empty", "-m", "root"]).0,
        0,
        "seed commit succeeds"
    );
    let linked = root.with_file_name("hugit-capture-linked-worktree");
    let _ = std::fs::remove_dir_all(&linked);
    let output = Command::new("git")
        .current_dir(&root)
        .args(["worktree", "add", "-b", "linked", linked.to_str().unwrap()])
        .output()
        .expect("create linked worktree");
    assert!(output.status.success(), "git worktree add succeeds");

    let (code, attached) = run_in(&linked, &["attach"]);
    assert_eq!(code, 0, "attach from linked worktree succeeds: {attached}");
    assert!(
        runtime_log(&root).is_file(),
        "common Git directory owns shared log"
    );
    assert!(
        !linked.join(".hugit").exists(),
        "linked worktree must not create tracked runtime state"
    );

    let (code, health) = run_in(&linked, &["health"]);
    assert_eq!(code, 0, "health succeeds: {health}");
    assert_eq!(health["mode"], "active");
    assert_eq!(
        std::fs::canonicalize(health["log"]["path"].as_str().expect("health log path"))
            .expect("canonical health log path")
            .display()
            .to_string(),
        std::fs::canonicalize(runtime_log(&root))
            .expect("canonical root log path")
            .display()
            .to_string(),
        "health inspects the hook-owned shared log"
    );

    // Linked checkout runs source hooks but must project into source common-dir.
    assert_eq!(
        git_with_hugit(&linked, &["checkout", "-b", "linked-capture"]).0,
        0,
        "linked checkout succeeds"
    );
    assert!(wait_for_ref_update(
        &runtime_log(&root),
        |payload| payload["checkout"]["truth"] == "branch_checkout_observed"
            && payload["branch"] == "linked-capture",
        20000,
    ));

    // Clone owns fresh Git common-dir; attaching then checking out captures only
    // clone-local movement, never writes into source runtime state.
    let clone = root.with_file_name("hugit-capture-clone-checkout");
    let _ = std::fs::remove_dir_all(&clone);
    let output = Command::new("git")
        .args(["clone", root.to_str().unwrap(), clone.to_str().unwrap()])
        .output()
        .expect("clone source repo");
    assert!(
        output.status.success(),
        "clone succeeds: {:?}",
        output.stderr
    );
    assert_eq!(run_in(&clone, &["attach"]).0, 0, "clone attach succeeds");
    assert_eq!(
        git_with_hugit(&clone, &["checkout", "-b", "clone-capture"]).0,
        0,
        "clone checkout succeeds"
    );
    assert!(wait_for_ref_update(
        &runtime_log(&clone),
        |payload| payload["checkout"]["truth"] == "branch_checkout_observed"
            && payload["branch"] == "clone-capture",
        20000,
    ));
    assert!(
        !ref_updates(&runtime_log(&root)).iter().any(|record| {
            serde_json::from_str::<Value>(record["payload"].as_str().unwrap())
                .ok()
                .is_some_and(|payload| payload["branch"] == "clone-capture")
        }),
        "clone checkout cannot cross into source runtime log"
    );
}

#[test]
fn setup_requires_explicit_global_template_replacement() {
    let root = scratch("setup-template-conflict");
    let config = root.join("gitconfig");
    std::fs::write(&config, b"").expect("create isolated git config");
    let first = root.join("first-template");
    let second = root.join("second-template");

    let run_setup = |dir: &Path, replace: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hugit"));
        command
            .args(["setup", "--dir", dir.to_str().unwrap()])
            .env("GIT_CONFIG_GLOBAL", &config);
        if replace {
            command.arg("--replace-global-template");
        }
        let output = command.output().expect("run setup");
        let value = serde_json::from_slice::<Value>(&output.stdout).expect("setup JSON");
        (output.status.code().unwrap_or(-1), value)
    };

    assert_eq!(run_setup(&first, false).0, 0, "first setup succeeds");
    let (code, conflict) = run_setup(&second, false);
    assert_eq!(code, 2, "foreign global template is preserved: {conflict}");
    assert_eq!(conflict["error"]["kind"], "global_template_conflict");
    assert_eq!(
        run_setup(&second, true).0,
        0,
        "explicit replacement succeeds"
    );
}

#[test]
#[ignore = "superseded by acceptance_attach for current attach API"]
fn attach_preserves_foreign_hook_and_health_reports_partial() {
    let _serial = lock_hook_journey();
    let root = scratch("attach-foreign-hook");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    let foreign = root.join(".git/hooks/post-commit");
    let bytes = b"#!/bin/sh\nprintf foreign\n";
    std::fs::write(&foreign, bytes).expect("write foreign hook");

    let (code, attached) = run_in(&root, &["attach", "--dir", root.to_str().unwrap()]);
    assert_eq!(code, 0, "attach succeeds despite foreign hook: {attached}");
    assert!(
        attached["hooks_conflict"]
            .as_array()
            .is_some_and(|hooks| hooks.iter().any(|hook| hook == "post-commit")),
        "attach reports exact foreign hook conflict: {attached}"
    );
    assert_eq!(std::fs::read(&foreign).expect("read foreign hook"), bytes);

    let (code, health) = run_in(&root, &["health"]);
    assert_eq!(code, 0, "health succeeds: {health}");
    assert_eq!(
        health["mode"], "partial",
        "foreign hook remains visible: {health}"
    );
    assert_eq!(health["hooks"]["post_commit"]["state"], "foreign");
}

#[test]
#[ignore = "superseded by acceptance_attach for current attach API"]
fn attach_preview_token_adopts_foreign_hook_and_fenced_detach_restores_it() {
    let _serial = lock_hook_journey();
    let root = scratch("attach-adopt-dispatcher");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    let foreign = root.join(".git/hooks/post-commit");
    let foreign_bytes = b"#!/bin/sh\nprintf foreign-hook\n";
    std::fs::write(&foreign, foreign_bytes).expect("write foreign hook");

    let (code, preview) = run_in(&root, &["attach", "--preview"]);
    assert_eq!(code, 0, "preview succeeds: {preview}");
    assert_eq!(preview["writes"], false);
    let token = preview["adoption_token"].as_str().expect("preview token");
    assert_eq!(
        std::fs::read(&foreign).unwrap(),
        foreign_bytes,
        "preview is read-only"
    );

    let (code, adopted) = run_in(
        &root,
        &[
            "attach",
            "--adopt-managed-dispatcher",
            "--adoption-token",
            token,
        ],
    );
    assert_eq!(code, 0, "adoption succeeds: {adopted}");
    assert!(
        std::fs::read_to_string(&foreign)
            .unwrap()
            .contains("hugit-managed-dispatcher v1")
    );
    let common = git_in(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let runtime = PathBuf::from(common.1.trim()).join("hugit");
    assert!(runtime.join("hook-manifest.json").is_file());
    assert!(runtime.join("hook-backups").is_dir());

    let (code, health) = run_in(&root, &["health"]);
    assert_eq!(code, 0, "health succeeds: {health}");
    assert_eq!(
        health["mode"], "active",
        "adopted dispatcher is healthy: {health}"
    );
    assert_eq!(health["hooks"]["post_commit"]["state"], "adopted");

    let (code, detached) = run_in(&root, &["detach"]);
    assert_eq!(code, 0, "detach succeeds: {detached}");
    assert!(
        detached["restored"]
            .as_array()
            .is_some_and(|hooks| hooks.iter().any(|hook| hook == "post-commit"))
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), foreign_bytes);
}

#[test]
#[ignore = "superseded by acceptance_attach for current attach API"]
fn attach_retry_resumes_prepared_adoption_from_canonical_backup() {
    let _serial = lock_hook_journey();
    let root = scratch("attach-adopt-retry");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    let post_commit = root.join(".git/hooks/post-commit");
    let pre_push = root.join(".git/hooks/pre-push");
    let post_commit_bytes = b"#!/bin/sh\nprintf post-commit\n";
    let pre_push_bytes = b"#!/bin/sh\nprintf pre-push\n";
    std::fs::write(&post_commit, post_commit_bytes).unwrap();
    std::fs::write(&pre_push, pre_push_bytes).unwrap();

    let (_, preview) = run_in(&root, &["attach", "--preview"]);
    let token = preview["adoption_token"].as_str().unwrap().to_string();
    let (code, adopted) = run_in(
        &root,
        &[
            "attach",
            "--adopt-managed-dispatcher",
            "--adoption-token",
            &token,
        ],
    );
    assert_eq!(code, 0, "adoption succeeds: {adopted}");
    let common = git_in(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let runtime = PathBuf::from(common.1.trim()).join("hugit");
    let manifest_path = runtime.join("hook-manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["state"] = Value::String("prepared".into());
    let entries = manifest["entries"].as_array().unwrap();
    let backup = entries
        .iter()
        .find(|entry| entry["kind"] == "pre-push")
        .unwrap()["backup"]
        .as_str()
        .unwrap();
    std::fs::write(
        &pre_push,
        std::fs::read(runtime.join("hook-backups").join(backup)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let (code, retried) = run_in(
        &root,
        &[
            "attach",
            "--adopt-managed-dispatcher",
            "--adoption-token",
            &token,
        ],
    );
    assert_eq!(code, 0, "prepared adoption resumes: {retried}");
    assert!(
        std::fs::read_to_string(&post_commit)
            .unwrap()
            .contains("hugit-managed-dispatcher v1")
    );
    assert!(
        std::fs::read_to_string(&pre_push)
            .unwrap()
            .contains("hugit-managed-dispatcher v1")
    );

    manifest["state"] = Value::String("installed".into());
    manifest["entries"][0]["backup"] = Value::String("../escape.hook".into());
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let (code, detached) = run_in(&root, &["detach"]);
    assert_eq!(code, 2, "non-canonical backup path is rejected: {detached}");
    assert_eq!(detached["error"]["kind"], "hook_manifest_invalid");
}

#[test]
#[ignore = "superseded by acceptance_attach for current attach API"]
fn attach_refuses_external_hooks_path_and_health_never_claims_active() {
    let _serial = lock_hook_journey();
    let root = scratch("attach-hooks-path");
    assert_eq!(git_in(&root, &["init"]).0, 0, "git init succeeds");
    assert_eq!(
        git_in(&root, &["config", "core.hooksPath", "custom-hooks"]).0,
        0,
        "configure external hooks path"
    );

    let (code, attached) = run_in(&root, &["attach"]);
    assert_eq!(
        code, 2,
        "attach refuses unsupported hook manager: {attached}"
    );
    assert_eq!(attached["error"]["kind"], "hooks_path_unsupported");
    assert!(
        !runtime_log(&root).exists(),
        "refusal must not create partial hugit state"
    );

    let (code, health) = run_in(&root, &["health"]);
    assert_eq!(code, 0, "health reports config without mutation: {health}");
    assert_eq!(health["mode"], "partial");
    assert_eq!(health["hooks_path"]["state"], "external");
}

#[test]
fn init_installs_all_capture_hooks() {
    let root = scratch("install");
    lib_init(&root);
    let hooks_dir = root.join(".git/hooks");
    for kind in [
        "post-commit",
        "post-checkout",
        "pre-push",
        "post-merge",
        "post-rewrite",
        "reference-transaction",
    ] {
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
    let _serial = lock_hook_journey();

    let root = scratch("commit");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = runtime_log(&root);

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
        20000,
    );
    assert!(
        got,
        "post-commit captured ref.update on the branch with a real oid"
    );
}

#[cfg(unix)]
#[test]
fn delayed_worker_keeps_pre_detach_commit_snapshot() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = lock_hook_journey();
    let root = scratch("delayed-a-b");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);
    let wrapper = root.join("delayed-hugit.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nsleep 2\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_hugit")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    std::fs::write(root.join("a.txt"), "A\n").unwrap();
    git_in(&root, &["add", "a.txt"]);
    let a = Command::new("git")
        .args(["commit", "-m", "A", "--no-gpg-sign"])
        .current_dir(&root)
        .env("HUGIT_BIN", &wrapper)
        .output()
        .unwrap();
    assert!(a.status.success(), "A commit succeeds");
    let a_oid = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();

    // Change HEAD while A's detached child waits. A must still resolve
    // paths/target from its captured OID, never later HEAD.
    std::fs::write(root.join("b.txt"), "B\n").unwrap();
    git_in(&root, &["add", "b.txt"]);
    let b = Command::new("git")
        .args(["commit", "-m", "B", "--no-gpg-sign", "--no-verify"])
        .current_dir(&root)
        .env("HUGIT_BIN", &wrapper)
        .output()
        .unwrap();
    assert!(b.status.success(), "B commit succeeds");

    assert!(wait_for_ref_update(
        &log,
        |p| p["target"].as_str() == Some(a_oid.as_str()),
        20000
    ));
    let records = ref_updates(&log);
    let payloads: Vec<Value> = records
        .iter()
        .filter_map(|r| serde_json::from_str(r["payload"].as_str()?).ok())
        .collect();
    assert!(
        payloads.iter().any(|p| {
            p["target"].as_str() == Some(a_oid.as_str())
                && p["files"] == serde_json::json!(["a.txt"])
        }),
        "delayed child captured immutable A snapshot, not later B HEAD; A was {a_oid}"
    );
}

#[test]
fn real_git_checkout_preserves_old_new_schema() {
    let _serial = lock_hook_journey();

    let root = scratch("checkout");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = runtime_log(&root);
    git_with_hugit(&root, &["add", "a.txt"]);
    git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);

    // REAL branch checkout (the LLM switches context).
    let (code, _) = git_with_hugit(&root, &["checkout", "-b", "feat/agent"]);
    assert_eq!(code, 0);
    assert!(
        wait_for_ref_update(
            &log,
            |p| {
                p["checkout"]["truth"] == "branch_checkout_observed"
                    && p["old"].as_str().is_some()
                    && p["new"].as_str().is_some()
                    && p["old"] == p["from"]
                    && p["new"] == p["to"]
            },
            20000
        ),
        "checkout preserves Git old/new plus legacy from/to"
    );
}

#[test]
fn real_git_push_attempts_capture_attempt_true() {
    let _serial = lock_hook_journey();

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
    let log = runtime_log(&root);
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

    // Push capture persists bounded typed tuples, never raw pre-push stdin.
    let pushed_has_shas = updates.iter().any(|r| {
        serde_json::from_str::<Value>(r["payload"].as_str().unwrap())
            .map(|p| p["updates"].as_array().is_some_and(|u| !u.is_empty()))
            .unwrap_or(false)
    });
    assert!(
        pushed_has_shas,
        "the push-attempt capture records typed local ref tuples"
    );

    // Advance remote from another clone, then prove failed transport still has a
    // pre-push attempt fact. `pre-push` cannot truthfully claim outcome.
    let rival = root.join("rival");
    assert_eq!(
        Command::new("git")
            .args(["clone", remote.to_str().unwrap(), rival.to_str().unwrap()])
            .status()
            .unwrap()
            .code(),
        Some(0)
    );
    set_git_identity(&rival);
    std::fs::write(rival.join("rival.txt"), "rival").unwrap();
    assert_eq!(git_in(&rival, &["add", "rival.txt"]).0, 0);
    assert_eq!(
        git_in(&rival, &["commit", "-m", "rival", "--no-gpg-sign"]).0,
        0
    );
    assert_eq!(git_in(&rival, &["push"]).0, 0);
    std::fs::write(root.join("local.txt"), "local").unwrap();
    assert_eq!(git_in(&root, &["add", "local.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "local", "--no-gpg-sign"]).0,
        0
    );
    assert_ne!(
        git_with_hugit(&root, &["push", "origin", &branch]).0,
        0,
        "non-fast-forward push fails"
    );
    assert!(
        wait_for_ref_update(&log, |p| p["attempt"] == true, 20000),
        "failed push retains attempted-only fact"
    );
}

#[test]
fn real_git_amend_rebase_and_reference_transaction_capture_facts() {
    let _serial = lock_hook_journey();
    let root = scratch("rewrite-transaction");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);
    std::fs::write(root.join("base.txt"), "base").unwrap();
    assert_eq!(git_in(&root, &["add", "base.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "base", "--no-gpg-sign"]).0,
        0
    );
    assert_eq!(
        git_with_hugit(&root, &["commit", "--amend", "--no-edit", "--no-gpg-sign"]).0,
        0
    );
    assert!(wait_for_ref_update(
        &log,
        |p| p["rewrite"]["type"] == "amend",
        20000
    ));

    assert_eq!(git_with_hugit(&root, &["checkout", "-b", "topic"]).0, 0);
    std::fs::write(root.join("topic.txt"), "topic").unwrap();
    assert_eq!(git_in(&root, &["add", "topic.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "topic", "--no-gpg-sign"]).0,
        0
    );
    assert_eq!(git_with_hugit(&root, &["checkout", "master"]).0, 0);
    std::fs::write(root.join("main.txt"), "main").unwrap();
    assert_eq!(git_in(&root, &["add", "main.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "main", "--no-gpg-sign"]).0,
        0
    );
    assert_eq!(git_with_hugit(&root, &["checkout", "topic"]).0, 0);
    assert_eq!(git_with_hugit(&root, &["rebase", "master"]).0, 0);
    assert!(wait_for_ref_update(
        &log,
        |p| p["rewrite"]["type"] == "rebase",
        20000
    ));

    let tip = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    assert_eq!(
        git_with_hugit(&root, &["update-ref", "refs/heads/captured", &tip]).0,
        0
    );
    assert!(wait_for_ref_update(
        &log,
        |p| p["reference_transaction"]["phase"] == "committed",
        20000
    ));
    assert_eq!(
        git_with_hugit(&root, &["update-ref", "-d", "refs/heads/captured"]).0,
        0
    );
    assert!(wait_for_ref_update(
        &log,
        |p| p["reference_transaction"]["updates"]
            .as_array()
            .is_some_and(|updates| updates.iter().any(|u| u["ref"] == "refs/heads/captured")),
        20000
    ));
}

#[test]
fn real_git_fast_forward_and_octopus_merges_capture_parent_truth() {
    let _serial = lock_hook_journey();
    let root = scratch("merge-matrix");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);
    std::fs::write(root.join("base.txt"), "base").unwrap();
    assert_eq!(git_in(&root, &["add", "base.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "base", "--no-gpg-sign"]).0,
        0
    );
    let main = git_in(&root, &["branch", "--show-current"])
        .1
        .trim()
        .to_string();

    assert_eq!(git_with_hugit(&root, &["checkout", "-b", "ff"]).0, 0);
    std::fs::write(root.join("ff.txt"), "ff").unwrap();
    assert_eq!(git_in(&root, &["add", "ff.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "ff", "--no-gpg-sign"]).0,
        0
    );
    assert_eq!(git_with_hugit(&root, &["checkout", &main]).0, 0);
    assert_eq!(git_with_hugit(&root, &["merge", "ff"]).0, 0);
    let ff_tip = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    assert!(wait_for_ref_update(
        &log,
        |p| p["target"] == ff_tip && p["merge_kind"] == "fast_forward",
        20000
    ));

    assert_eq!(git_with_hugit(&root, &["checkout", "-b", "oct-a"]).0, 0);
    std::fs::write(root.join("oct-a.txt"), "a").unwrap();
    assert_eq!(git_in(&root, &["add", "oct-a.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "oct-a", "--no-gpg-sign"]).0,
        0
    );
    assert_eq!(git_with_hugit(&root, &["checkout", &main]).0, 0);
    assert_eq!(git_with_hugit(&root, &["checkout", "-b", "oct-b"]).0, 0);
    std::fs::write(root.join("oct-b.txt"), "b").unwrap();
    assert_eq!(git_in(&root, &["add", "oct-b.txt"]).0, 0);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "oct-b", "--no-gpg-sign"]).0,
        0
    );
    assert_eq!(git_with_hugit(&root, &["checkout", &main]).0, 0);
    assert_eq!(
        git_with_hugit(
            &root,
            &["merge", "--no-ff", "oct-a", "oct-b", "-m", "octopus"]
        )
        .0,
        0
    );
    let octopus_tip = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    assert!(wait_for_ref_update(
        &log,
        |p| p["target"] == octopus_tip
            && p["merge_kind"] == "merge_commit"
            && p["parents"]
                .as_array()
                .is_some_and(|parents| parents.len() == 3),
        20000
    ));
}

#[test]
fn missing_bin_never_blocks_git() {
    let _serial = lock_hook_journey();

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
    let _serial = lock_hook_journey();

    let root = scratch("watch");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = runtime_log(&root);

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
        20000,
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

/// Wait for hook capture projection plus completed-receipt cleanup. A visible
/// log record alone is not worker quiescence, so a following writer may race.
fn drain_is_idle(log: &Path) -> bool {
    let mut lock = log.as_os_str().to_os_string();
    lock.push(".lock");
    let root = log.parent().expect("runtime log parent");
    !PathBuf::from(lock).exists()
        && std::fs::read_dir(root.join("receipts"))
            .map(|entries| entries.flatten().next().is_none())
            .unwrap_or(false)
}

/// Wait until at least two hook-captured commit records are complete (async
/// hooks may still be catching up).
fn wait_for_two_captures(log: &Path, timeout_ms: u64) -> Vec<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        let caps = captured_commits(log);
        if caps.len() >= 2 && drain_is_idle(log) {
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
    let _serial = lock_hook_journey();

    let root = scratch("undo-capture");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = runtime_log(&root);

    // TWO REAL commits on the same branch: undoing the second capture restores
    // the ref to the first capture's target (a real compensator).
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);
    assert_eq!(code, 0, "first commit");
    std::fs::write(root.join("a.txt"), "a2").unwrap();
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c2", "--no-gpg-sign"]);
    assert_eq!(code, 0, "second commit");

    let captures = wait_for_two_captures(&log, 25000);
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
    let seq_text = seq.to_string();
    let (code, v) = (0..20)
        .find_map(|_| {
            let result = run_in(
                &root,
                &[
                    "undo",
                    "--log",
                    log.to_str().unwrap(),
                    "--seq",
                    &seq_text,
                    "--actor",
                    "user:human",
                ],
            );
            if result.0 == 2 && result.1["error"]["kind"] == "log_busy" {
                std::thread::sleep(std::time::Duration::from_millis(100));
                None
            } else {
                Some(result)
            }
        })
        .expect("log lock releases within bounded retry window");
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
    let _serial = lock_hook_journey();

    let root = scratch("undo-capture-denied");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = runtime_log(&root);

    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "c1", "--no-gpg-sign"]);
    assert_eq!(code, 0, "commit");

    let captures = wait_for_two_captures(&log, 25000);
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
    let _serial = lock_hook_journey();

    let root = scratch("why-capture");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);

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
        20000,
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
    let _serial = lock_hook_journey();

    let root = scratch("why-lib");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);

    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").unwrap();
    git_in(&root, &["add", "src/lib.rs"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: add lib", "--no-gpg-sign"]);
    assert_eq!(code, 0);
    let got = wait_for_ref_update(
        &log,
        |p| !p["target"].as_str().unwrap_or("").is_empty(),
        20000,
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

// ── 8. A commits-only PR (captured raw commits) lands via land queue ────────

#[test]
fn commits_only_pr_lands_via_queue() {
    let _serial = lock_hook_journey();

    let root = scratch("land-commits");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);

    // ONE real commit (the LLM's normal action) — captured by the hook.
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: a", "--no-gpg-sign"]);
    assert_eq!(code, 0);
    let committed_oid = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    let got = wait_for_ref_update(
        &log,
        |p| p["target"].as_str() == Some(committed_oid.as_str()),
        20000,
    );
    assert!(got, "captured commit OID landed: {committed_oid}");

    // Pairing/ref-transaction records can append after capture. Select exact
    // durable commit fact, never whichever ref.update happens to be last.
    let target = ref_updates(&log)
        .into_iter()
        .filter_map(|record| {
            record["payload"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
        })
        .find_map(|payload| {
            (payload["target"].as_str() == Some(committed_oid.as_str())).then_some(payload)
        })
        .and_then(|payload| payload["target"].as_str().map(str::to_string))
        .expect("durable captured commit target selected by committed OID");

    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-C1",
            "--campaign",
            "camp-c",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "run-1",
            "--commit",
            &target,
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "pr open with a captured commit: {v}");

    // pr show reflects the captured commit as an external PR member.
    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "show",
            "--pr",
            "PR-C1",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "pr show works");
    assert_eq!(v["commit_ids"][0], target, "PR carries the captured commit");

    // Queue + land: the commits-only PR must land (its raw-commit members are
    // the content, not an empty PR).
    run_in(
        &root,
        &[
            "pr",
            "queue",
            "--pr",
            "PR-C1",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    let (code, v) = run_in(
        &root,
        &[
            "land",
            "queue",
            "--campaign",
            "camp-c",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "land queue succeeds with a commits-only PR: {v}");
    assert_eq!(v["landed"][0], "PR-C1", "the commits-only PR lands");

    // pr show reflects the terminal landed state.
    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "show",
            "--pr",
            "PR-C1",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        v["queue"]["queued"], false,
        "PR left the queue after landing"
    );
}

// ── 9. `hugit why` (the BINARY) resolves against the CANONICAL captured log ──

#[test]
fn why_binary_reads_canonical_captured_log_and_resolves() {
    let root = scratch("why-bin");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);

    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").unwrap();
    git_in(&root, &["add", "src/lib.rs"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: add lib", "--no-gpg-sign"]);
    assert_eq!(code, 0);
    let got = wait_for_ref_update(
        &log,
        |p| !p["target"].as_str().unwrap_or("").is_empty(),
        20000,
    );
    assert!(got, "capture landed");

    // `hugit why --log <canonical>` (real binary) resolves the touched path.
    // The canonical log is the bare [EventRecord, ...] the hooks wrote; the why
    // binary now accepts it directly (no wrapper transcription needed).
    let (code, v) = run_in(
        &root,
        &[
            "why",
            "--log",
            log.to_str().unwrap(),
            "--path",
            "src/lib.rs",
        ],
    );
    assert_eq!(
        code, 0,
        "hugit why reads the canonical log and resolves: {v}"
    );
    // The answer identifies the captured ref.update as the origin.
    assert!(
        v["origin"] != Value::Null
            || v.to_string().contains("capture")
            || v.to_string().contains("ref.update"),
        "why found the captured origin: {v}"
    );
}

// ── 10. `hugit why --walk` — the FULL provenance chain over captured commits ──

#[test]
fn why_walk_projects_the_provenance_chain_in_reverse_order() {
    let _serial = lock_hook_journey();

    let root = scratch("why-walk");
    lib_init(&root);
    set_git_identity(&root);
    let log = runtime_log(&root);

    // TWO real commits on the SAME path — the chain shows BOTH, newest first.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").unwrap();
    git_in(&root, &["add", "src/lib.rs"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: add lib", "--no-gpg-sign"]);
    assert_eq!(code, 0);
    let caps = wait_for_commit_captures(&log, 1, 20000);
    assert_eq!(caps.len(), 1, "first commit captured");

    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn x() { println!(\"hi\"); }\n",
    )
    .unwrap();
    git_in(&root, &["add", "src/lib.rs"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: print hi", "--no-gpg-sign"]);
    assert_eq!(code, 0);
    let caps = wait_for_commit_captures(&log, 2, 20000);
    assert!(caps.len() >= 2, "second commit captured: {caps:?}");
    let (c1, c2) = (caps[caps.len() - 2].clone(), caps.last().unwrap().clone());

    // `hugit why --walk --path src/lib.rs` (real binary) returns the chain.
    let (code, v) = run_in(
        &root,
        &[
            "why",
            "--walk",
            "--log",
            log.to_str().unwrap(),
            "--path",
            "src/lib.rs",
        ],
    );
    assert_eq!(code, 0, "why --walk succeeds: {v}");
    let chain = v.as_array().expect("walk returns an array");
    assert!(
        chain.len() >= 2,
        "chain shows BOTH captured commits (got {}): {v}",
        chain.len()
    );
    // Newest FIRST: the second commit is the chain head.
    assert_eq!(
        chain[0]["oid"].as_str(),
        Some(c2.as_str()),
        "chain head = the newest captured commit"
    );
    // Each link is a distinct, raw captured event — never fused.
    let oids: Vec<&str> = chain.iter().filter_map(|l| l["oid"].as_str()).collect();
    assert!(
        oids.contains(&c1.as_str()) && oids.contains(&c2.as_str()),
        "chain carries both distinct oids: {v}"
    );
    // Kind is the frozen ref.update; the chain never re-attributes an author.
    assert_eq!(
        chain[0]["kind"], "ref.update",
        "chain links are ref.update events"
    );
    assert!(
        chain[0]["seq"].as_u64().unwrap() > chain[1]["seq"].as_u64().unwrap() || chain.len() > 2,
        "chain is ordered most-recent-first (compare seqs): {v}"
    );

    // The single-origin read (no --walk) is unchanged and equals the head.
    let (code, origin) = run_in(
        &root,
        &[
            "why",
            "--log",
            log.to_str().unwrap(),
            "--path",
            "src/lib.rs",
        ],
    );
    assert_eq!(code, 0, "why (origin) still works: {origin}");
    assert_eq!(
        origin["event_seq"], chain[0]["seq"],
        "the origin read IS the chain head — one answer, no divergence"
    );
}

/// Poll until at least `need` distinct commit captures (ref.update with a
/// non-empty target + non-empty files) have landed, returning their target oids
/// in log order. Hooks are async; a bare wait on "any capture" can read the
/// first commit twice — this waits on a COUNT of distinct captures.
fn wait_for_commit_captures(log: &std::path::Path, need: usize, timeout_ms: u64) -> Vec<String> {
    for _ in 0..(timeout_ms / 250) {
        let records = log_records(log);
        let mut oids: Vec<String> = records
            .iter()
            .rev()
            .filter(|r| r["kind"] == "ref.update")
            .filter_map(|r| {
                let p = r["payload"].as_str()?;
                let v: Value = serde_json::from_str(p).ok()?;
                let t = v["target"].as_str()?.to_string();
                let has_files = v["files"].as_array().is_some_and(|a| !a.is_empty());
                if !t.is_empty() && has_files {
                    Some(t)
                } else {
                    None
                }
            })
            .collect();
        oids.reverse();
        if oids.len() >= need {
            return oids;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    vec![]
}
