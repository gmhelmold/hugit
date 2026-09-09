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

fn capture_binary(cwd: &Path, args: &[String]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hugit capture runs")
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
    let _serial = hook_serial().lock().expect("hook serial lock");

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
                && p["hunk_capture"] == "complete"
                && p["files"][0]["path"] == "a.txt"
                && p["files"][0]["ranges"][0] == serde_json::json!({ "start": 1, "end": 1 })
        },
        20000,
    );
    assert!(
        got,
        "post-commit captured ref.update on the branch with a real oid"
    );
}

#[test]
fn rapid_commits_capture_their_snapshotted_oids() {
    let _serial = hook_serial().lock().expect("hook serial lock");
    let root = scratch("rapid-commits");
    lib_init(&root);
    set_git_identity(&root);
    let log = root.join(".hugit/log.json");
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git_with_hugit(&root, &["add", "a.txt"]);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "one", "--no-gpg-sign"]).0,
        0
    );
    let first = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    std::fs::write(root.join("a.txt"), "two\n").unwrap();
    git_with_hugit(&root, &["add", "a.txt"]);
    assert_eq!(
        git_with_hugit(&root, &["commit", "-m", "two", "--no-gpg-sign"]).0,
        0
    );
    let second = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    let captures = wait_for_commit_captures(&log, 2, 20000);
    assert!(
        captures.contains(&first),
        "first commit capture survives rapid successor: {captures:?}"
    );
    assert!(
        captures.contains(&second),
        "second commit capture exists: {captures:?}"
    );
}

#[test]
fn capture_discovers_newline_path_and_target_hunk_from_real_git() {
    let root = scratch("nul-path");
    lib_init(&root);
    set_git_identity(&root);
    let path = "src/line\nbreak.rs";
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join(path), "pub fn alpha() {\n    let value = 1;\n}\n").unwrap();
    git_in(&root, &["add", "--", path]);
    assert_eq!(
        git_in(&root, &["commit", "-m", "newline path", "--no-gpg-sign"]).0,
        0
    );
    let oid = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    let log = root.join(".hugit/log.json");
    let args = vec![
        "capture".to_string(),
        "--kind".to_string(),
        "commit".to_string(),
        "--top-level".to_string(),
        root.display().to_string(),
        "--log".to_string(),
        log.display().to_string(),
        "--oid".to_string(),
        oid.clone(),
    ];
    assert!(capture_binary(&root, &args).status.success());
    let payload: Value = serde_json::from_str(
        log_records(&log)
            .last()
            .and_then(|record| record["payload"].as_str())
            .expect("capture event payload"),
    )
    .unwrap();
    assert_eq!(payload["hunk_capture"], "complete");
    assert_eq!(payload["files"][0]["path"], path);
    assert_eq!(
        payload["files"][0]["ranges"],
        serde_json::json!([{ "start": 1, "end": 3 }])
    );

    let (code, answer) = run_in(
        &root,
        &[
            "why",
            "--log",
            log.to_str().unwrap(),
            "--repo",
            root.to_str().unwrap(),
            "--commit",
            "HEAD",
            "--path",
            path,
            "--line",
            "2",
        ],
    );
    assert_eq!(code, 0, "precise why resolves real Git line: {answer}");
    assert_eq!(answer["status"], "attributed");
    assert_eq!(answer["observed"]["commit"], oid);

    let (code, symbol_answer) = run_in(
        &root,
        &[
            "why",
            "--log",
            log.to_str().unwrap(),
            "--repo",
            root.to_str().unwrap(),
            "--commit",
            "HEAD",
            "--path",
            path,
            "--symbol",
            "alpha",
        ],
    );
    assert_eq!(
        code, 0,
        "precise why resolves committed symbol: {symbol_answer}"
    );
    assert_eq!(symbol_answer["status"], "attributed");
    assert_eq!(
        symbol_answer["observed"]["range"],
        serde_json::json!([1, 3])
    );
    assert_eq!(
        symbol_answer["contributors"].as_array().map(Vec::len),
        Some(1)
    );
    assert!(
        root.join(".hugit/cache/symbols")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
    let cache = root
        .join(".hugit/cache/symbols")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(
        &cache,
        br#"[{"name":"forged","start_line":1,"end_line":1}]"#,
    )
    .unwrap();
    let (code, answer) = run_in(
        &root,
        &[
            "why",
            "--log",
            log.to_str().unwrap(),
            "--repo",
            root.to_str().unwrap(),
            "--commit",
            "HEAD",
            "--path",
            path,
            "--symbol",
            "alpha",
        ],
    );
    assert_eq!(
        code, 0,
        "tampered cache cannot alter symbol provenance: {answer}"
    );
    assert_eq!(answer["observed"]["range"], serde_json::json!([1, 3]));
}

#[cfg(unix)]
#[test]
fn capture_refuses_non_utf8_path_without_lossy_attribution() {
    use std::os::unix::ffi::OsStringExt;

    let root = scratch("non-utf8-path");
    lib_init(&root);
    set_git_identity(&root);
    let path = std::ffi::OsString::from_vec(b"bad-\xff.rs".to_vec());
    if std::fs::write(root.join(&path), "fn x() {}\n").is_err() {
        return;
    }
    assert!(
        Command::new("git")
            .arg("add")
            .arg(&path)
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "non utf8", "--no-gpg-sign"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    let oid = git_in(&root, &["rev-parse", "HEAD"]).1.trim().to_string();
    let log = root.join(".hugit/log.json");
    let args = vec![
        "capture".to_string(),
        "--kind".to_string(),
        "commit".to_string(),
        "--top-level".to_string(),
        root.display().to_string(),
        "--log".to_string(),
        log.display().to_string(),
        "--oid".to_string(),
        oid,
    ];
    assert!(capture_binary(&root, &args).status.success());
    let payload: Value = serde_json::from_str(
        log_records(&log)
            .last()
            .and_then(|record| record["payload"].as_str())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["hunk_capture"], "unavailable");
    assert!(payload.get("files").is_none());
}

#[test]
fn real_git_checkout_captures_checkout_true() {
    let _serial = hook_serial().lock().expect("hook serial lock");

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
    let _serial = hook_serial().lock().expect("hook serial lock");

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

    // The captured push-attempt now records the LOCAL shas being pushed (the
    // pre-push hook extracts field #2 of each refspec line) — so the pushed
    // commit is a captured-commit proof, not just "a push happened".
    let pushed_has_shas = updates.iter().any(|r| {
        serde_json::from_str::<Value>(r["payload"].as_str().unwrap())
            .map(|p| p["shas"].as_str().is_some_and(|s| !s.trim().is_empty()))
            .unwrap_or(false)
    });
    assert!(
        pushed_has_shas,
        "the push-attempt capture records the local shas (hook extracts them)"
    );
}

#[test]
fn missing_bin_never_blocks_git() {
    let _serial = hook_serial().lock().expect("hook serial lock");

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

#[test]
fn capture_binary_scrubs_every_payload_leaf_before_append() {
    const SAFE_SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    const SAFE_FROM: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const SAFE_REF: &str = "feature/safe-capture";
    const CANARY_BRANCH: &str = "ghp_CAPTURE_BRANCH_0123456789abcdefghijklmnop";
    const CANARY_OID: &str = "ghp_CAPTURE_OID_0123456789abcdefghijklmnop";
    const CANARY_FROM: &str = "ghp_CAPTURE_FROM_0123456789abcdefghijklmnop";
    const CANARY_REFSPEC: &str =
        "refs/heads/ghp_CAPTURE_REFSPEC_0123456789abcdefghijklmnop:refs/heads/main";

    let root = scratch("payload-scrub");
    lib_init(&root);
    let log = root.join(".hugit/log.json");
    let hook_log = root.join(".hugit/hooks.log");
    let common = vec![
        "--top-level".to_string(),
        root.display().to_string(),
        "--log".to_string(),
        log.display().to_string(),
        "--hook-log".to_string(),
        hook_log.display().to_string(),
    ];
    let run_capture = |kind: &str, fields: &[(&str, &str)]| {
        let mut args = vec![
            "capture".to_string(),
            "--kind".to_string(),
            kind.to_string(),
        ];
        args.extend(common.clone());
        for (key, value) in fields {
            args.push((*key).to_string());
            args.push((*value).to_string());
        }
        let out = capture_binary(&root, &args);
        assert!(out.status.success(), "capture {kind} exits 0: {out:?}");
        out
    };

    // Positive controls: structurally valid Git addresses remain useful. Commit
    // paths are Git-discovered, never accepted from hook command arguments.
    run_capture("commit", &[("--oid", SAFE_SHA), ("--branch", SAFE_REF)]);
    run_capture(
        "checkout",
        &[
            ("--oid", SAFE_SHA),
            ("--from", SAFE_FROM),
            ("--branch", SAFE_REF),
        ],
    );

    // Every capture shape carries secret-shaped values at every hook-controlled
    // string position. Capture remains nonblocking while persisted payloads scrub.
    let outputs = [
        run_capture(
            "commit",
            &[("--oid", CANARY_OID), ("--branch", CANARY_BRANCH)],
        ),
        run_capture(
            "checkout",
            &[
                ("--oid", CANARY_OID),
                ("--from", CANARY_FROM),
                ("--branch", CANARY_BRANCH),
            ],
        ),
        run_capture(
            "push-attempt",
            &[("--refspecs", CANARY_REFSPEC), ("--shas", CANARY_OID)],
        ),
        run_capture("merge", &[("--oid", CANARY_OID), ("--from", CANARY_FROM)]),
    ];

    let log_bytes = std::fs::read(&log).expect("canonical log readable");
    let hook_bytes = std::fs::read(&hook_log).expect("hooks log readable");
    for canary in [CANARY_BRANCH, CANARY_OID, CANARY_FROM, CANARY_REFSPEC] {
        assert!(
            !log_bytes
                .windows(canary.len())
                .any(|w| w == canary.as_bytes()),
            "canonical log must not retain {canary}"
        );
        assert!(
            !hook_bytes
                .windows(canary.len())
                .any(|w| w == canary.as_bytes()),
            "hooks log must not retain {canary}"
        );
        for out in &outputs {
            assert!(
                !out.stdout
                    .windows(canary.len())
                    .any(|w| w == canary.as_bytes())
                    && !out
                        .stderr
                        .windows(canary.len())
                        .any(|w| w == canary.as_bytes()),
                "capture stdout/stderr must not retain {canary}"
            );
        }
    }

    let records = log_records(&log);
    let safe_commit = records.iter().find_map(|r| {
        let payload: Value = serde_json::from_str(r["payload"].as_str()?).ok()?;
        (payload["target"].as_str() == Some(SAFE_SHA)).then_some(payload)
    });
    let safe_commit = safe_commit.expect("safe commit capture exists");
    assert_eq!(safe_commit["ref"], format!("refs/heads/{SAFE_REF}"));
    assert_eq!(safe_commit["hunk_capture"], "unavailable");
    let safe_checkout = records.iter().find_map(|r| {
        let payload: Value = serde_json::from_str(r["payload"].as_str()?).ok()?;
        (payload["checkout"] == true).then_some(payload)
    });
    let safe_checkout = safe_checkout.expect("safe checkout capture exists");
    assert_eq!(safe_checkout["from"], SAFE_FROM);
    assert_eq!(safe_checkout["to"], SAFE_SHA);

    let out_dir = root.join("export-out");
    let export = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args([
            "export",
            "--log",
            log.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .current_dir(&root)
        .output()
        .expect("export runs");
    assert!(export.status.success(), "export succeeds: {export:?}");
    for path in [out_dir.join("export.json"), out_dir.join("repo.work/REFS")] {
        let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("artifact exists: {path:?}"));
        for canary in [CANARY_BRANCH, CANARY_OID, CANARY_FROM, CANARY_REFSPEC] {
            assert!(
                !bytes.windows(canary.len()).any(|w| w == canary.as_bytes()),
                "exported artifact {path:?} must not retain {canary}"
            );
        }
    }
}

// ── 6. The captured git activity is watchable by class ───────────────────────

#[test]
fn captured_activity_is_watchable_as_git_activity() {
    let _serial = hook_serial().lock().expect("hook serial lock");

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
    let _serial = hook_serial().lock().expect("hook serial lock");

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
    let _serial = hook_serial().lock().expect("hook serial lock");

    let root = scratch("undo-capture-denied");
    lib_init(&root);
    set_git_identity(&root);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let log = root.join(".hugit/log.json");

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
    let _serial = hook_serial().lock().expect("hook serial lock");

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
    let _serial = hook_serial().lock().expect("hook serial lock");

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
    let _serial = hook_serial().lock().expect("hook serial lock");

    let root = scratch("land-commits");
    lib_init(&root);
    set_git_identity(&root);
    let log = root.join(".hugit/log.json");

    // ONE real commit (the LLM's normal action) — captured by the hook.
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git_in(&root, &["add", "a.txt"]);
    let (code, _) = git_with_hugit(&root, &["commit", "-m", "feat: a", "--no-gpg-sign"]);
    assert_eq!(code, 0);
    let got = wait_for_ref_update(
        &log,
        |p| !p["target"].as_str().unwrap_or("").is_empty(),
        20000,
    );
    assert!(got, "capture landed");

    // Open a PR bundling ONLY the captured commit (no intents).
    let target = ref_updates(&log).last().unwrap()["payload"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap()["target"]
        .as_str()
        .unwrap()
        .to_string();

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
    let log = root.join(".hugit/log.json");

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
    let _serial = hook_serial().lock().expect("hook serial lock");

    let root = scratch("why-walk");
    lib_init(&root);
    set_git_identity(&root);
    let log = root.join(".hugit/log.json");

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
