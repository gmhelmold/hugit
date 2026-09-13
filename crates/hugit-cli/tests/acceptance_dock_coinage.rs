//! WP-DOCK-1 — hook-born dock coinage (post-checkout auto-dock).
//!
//! Proves, hermetically and end-to-end, that the post-checkout hook coins the
//! dock (gitdir+branch hash) at worktree checkout time:
//!
//! - **A1** — coin writes the marker + a `dock.record` (kind `dock.record`,
//!   charter derived from the branch, `charter_derived:true`, `state:open`).
//! - **S1** — idempotent: marker present ⇒ re-coin is a no-op (one record).
//! - **A3** — marker carries `created_ts` + `pid` (the reborn key).
//! - **R2** — env-vs-cwd divergence: `HUGIT_DOCK_ID` with a different gitdir
//!   ⇒ `parent_id` recorded + a warning in the hooks log (never silent, never
//!   adopted, never fail).
//! - **C1** — exit 0 ALWAYS: even a failing coin writes the error to the hooks
//!   log and exits 0 (never blocks git).
//! - **R1 e2e** — `git clone` (no hooks in the clone) coins NOTHING (the
//!   self-heal path is WP-DOCK-2; this WP proves the honest absence).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use hugit_cli::dock::{DOCK_MARKER, DOCK_RECORD_KIND};
use hugit_cli::init::{InitArgs, run as run_init};

/// Make a unique scratch dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-dock1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit init` via the library (installs hooks + empty log).
fn lib_init(dir: &Path) {
    let code = run_init(InitArgs {
        dir: Some(dir.to_path_buf()),
    });
    assert_eq!(code, std::process::ExitCode::SUCCESS, "lib init exits 0");
}

/// Run the real hugit binary (silent: returns code — stdout may be the hook log).
fn run_hugit(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hugit"));
    cmd.args(args).current_dir(cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("hugit runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Run git with HUGIT_BIN pointing at THIS build's binary (so the hook resolves).
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
    let bytes = std::fs::read(log).unwrap_or_else(|_| b"[]".to_vec());
    serde_json::from_slice(&bytes).unwrap_or_else(|_| vec![json!([])])
}

fn dock_records(log: &Path) -> Vec<Value> {
    log_records(log)
        .into_iter()
        .filter(|r| r.get("kind").and_then(|k| k.as_str()) == Some(DOCK_RECORD_KIND))
        .collect()
}

/// The dock record's JSON payload — the log stores `payload` as a JSON STRING.
fn payload_of(rec: &Value) -> Value {
    serde_json::from_str(rec.get("payload").and_then(|v| v.as_str()).unwrap_or("{}"))
        .unwrap_or(Value::Null)
}

fn write_event_log(path: &Path, events: &[(&str, Value)]) {
    let mut log = hugit_refstore::EventLog::new();
    for (kind, payload) in events {
        log.append_for_test(*kind, vec![], payload.to_string(), 0);
    }
    std::fs::write(path, serde_json::to_vec_pretty(log.records()).unwrap()).unwrap();
}

/// Poll for a condition on the (async) hooks — hooks detach by design.
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

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(input.as_bytes());
    h.iter().map(|b| format!("{b:02x}")).collect()
}

// ── Hermetic: coinage, idempotency, charter, A3 marker, R2, C1 ──────────────

#[test]
fn hermetic_coin_coins_record_and_marker_idempotently() {
    let dir = scratch("hermetic-coin");
    let gitdir = dir.join(".git/worktrees/rate-limit");
    std::fs::create_dir_all(&gitdir).unwrap();
    let log = dir.join(".hugit/log.json");
    std::fs::create_dir_all(dir.join(".hugit")).unwrap();
    std::fs::write(&log, "[]").unwrap();
    let hook_log = dir.join(".hugit/hooks.log");

    // A1 + S1: first coin writes marker + record; second coin is a no-op.
    for _ in 0..2 {
        let (code, _) = run_hugit(
            &dir,
            &[
                "dock",
                "coin",
                "--top-level",
                dir.to_str().unwrap(),
                "--gitdir",
                gitdir.to_str().unwrap(),
                "--branch",
                "feat/rate-limit",
                "--log",
                log.to_str().unwrap(),
                "--hook-log",
                hook_log.to_str().unwrap(),
            ],
            &[],
        );
        assert_eq!(code, 0, "coin exits 0");
    }

    let recs = dock_records(&log);
    assert_eq!(recs.len(), 1, "S1 — one dock, no duplicate");

    let rec = &recs[0];
    let payload = payload_of(rec);
    assert_eq!(payload.get("branch").unwrap(), "feat/rate-limit");
    assert_eq!(payload.get("charter").unwrap(), "add rate limit");
    assert_eq!(payload.get("charter_derived").unwrap(), true);
    assert_eq!(payload.get("state").unwrap(), "open");
    assert_eq!(payload.get("origin").unwrap(), "worktree");
    assert!(payload.get("parent_id").unwrap().is_null());

    let expected_id = sha256_hex(&format!("{}\u{0}feat/rate-limit", gitdir.to_str().unwrap()));
    assert_eq!(
        payload.get("dock_id").and_then(|v| v.as_str()).unwrap(),
        expected_id,
        "dock_id = sha256(gitdir NUL branch)"
    );

    // A3 — marker carries created_ts + pid.
    let marker_path = gitdir.join(DOCK_MARKER);
    assert!(marker_path.exists(), "marker exists");
    let marker: Value = serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
    assert!(marker.get("created_ts").is_some(), "marker has created_ts");
    assert!(marker.get("pid").is_some(), "marker has pid");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hermetic_r2_env_diff_records_parent_and_warns() {
    let dir = scratch("hermetic-r2");
    let gitdir_a = dir.join(".git/worktrees/a");
    let gitdir_b = dir.join(".git/worktrees/b");
    std::fs::create_dir_all(&gitdir_a).unwrap();
    std::fs::create_dir_all(&gitdir_b).unwrap();
    let log = dir.join(".hugit/log.json");
    std::fs::create_dir_all(dir.join(".hugit")).unwrap();
    std::fs::write(&log, "[]").unwrap();
    let hook_log = dir.join(".hugit/hooks.log");

    // Coin A first.
    run_hugit(
        &dir,
        &[
            "dock",
            "coin",
            "--top-level",
            dir.to_str().unwrap(),
            "--gitdir",
            gitdir_a.to_str().unwrap(),
            "--branch",
            "feat/a",
            "--log",
            log.to_str().unwrap(),
            "--hook-log",
            hook_log.to_str().unwrap(),
        ],
        &[],
    );
    let id_a = payload_of(&dock_records(&log)[0])
        .get("dock_id")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // Coin B with HUGIT_DOCK_ID = A (different gitdir) → parent_id + warn.
    run_hugit(
        &dir,
        &[
            "dock",
            "coin",
            "--top-level",
            dir.to_str().unwrap(),
            "--gitdir",
            gitdir_b.to_str().unwrap(),
            "--branch",
            "feat/b",
            "--log",
            log.to_str().unwrap(),
            "--hook-log",
            hook_log.to_str().unwrap(),
        ],
        &[("HUGIT_DOCK_ID", id_a.as_str())],
    );

    let recs = dock_records(&log);
    assert_eq!(recs.len(), 2, "two docks (A, B)");
    let b = payload_of(&recs[1]);
    assert_eq!(b.get("branch").unwrap(), "feat/b");
    assert_eq!(
        b.get("parent_id").unwrap(),
        id_a.as_str(),
        "R2 — parent recorded"
    );

    let hook_txt = std::fs::read_to_string(&hook_log).unwrap();
    assert!(
        hook_txt.contains("differs from HUGIT_DOCK_ID"),
        "R2 — warning visible in hooks log: {hook_txt}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hermetic_c1_failure_is_silent_exit_zero_writes_hook_log() {
    let dir = scratch("hermetic-c1");
    let gitdir = dir.join("is-a-file");
    std::fs::write(&gitdir, "x").unwrap(); // gitdir is a FILE → marker join fails
    let log = dir.join(".hugit/log.json");
    std::fs::create_dir_all(dir.join(".hugit")).unwrap();
    std::fs::write(&log, "[]").unwrap();
    let hook_log = dir.join(".hugit/hooks.log");

    let (code, _) = run_hugit(
        &dir,
        &[
            "dock",
            "coin",
            "--top-level",
            dir.to_str().unwrap(),
            "--gitdir",
            gitdir.to_str().unwrap(),
            "--branch",
            "feat/x",
            "--log",
            log.to_str().unwrap(),
            "--hook-log",
            hook_log.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "C1 — exit 0 ALWAYS");
    assert!(
        std::fs::read_to_string(&hook_log)
            .unwrap()
            .contains("write marker"),
        "the error is logged (never a fail)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dock_views_redact_secret_shaped_record_fields() {
    let dir = scratch("view-redaction");
    let log = dir.join("log.json");
    let secret = "SECRET:dock-view";
    write_event_log(
        &log,
        &[(
            DOCK_RECORD_KIND,
            json!({
                "dock_id": "dock-safe-id",
                "gitdir": format!("/tmp/{secret}"),
                "branch": "feat/safe",
                "charter": secret,
                "charter_derived": true,
                "state": "open",
                "origin": "worktree",
                "created_ts": 1,
                "pid": 1,
            }),
        )],
    );

    for args in [
        vec!["dock", "ls", "--log", log.to_str().unwrap()],
        vec![
            "dock",
            "show",
            "dock-safe-id",
            "--log",
            log.to_str().unwrap(),
        ],
    ] {
        let (code, output) = run_hugit(&dir, &args, &[]);
        assert_eq!(code, 0, "dock view exits 0: {output}");
        assert!(
            !output.contains(secret),
            "dock view must not leak secret: {output}"
        );
        assert!(
            output.contains("[REDACTED]"),
            "dock view signals redaction: {output}"
        );
    }
}

// ── e2e real git: worktree add coins; re-checkout is idempotent ────────────

#[test]
fn e2e_worktree_add_coins_and_switch_is_idempotent() {
    let dir = scratch("e2e");
    let (rc, _) = git_in(&dir, &["init", "-q", "-b", "main"]);
    assert_eq!(rc, 0);
    git_in(&dir, &["config", "user.email", "t@t"]);
    git_in(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git_with_hugit(&dir, &["add", "a.txt"]);
    let (rc, _) = git_with_hugit(&dir, &["commit", "-q", "-m", "init"]);
    assert_eq!(rc, 0, "seed commit lands");

    lib_init(&dir); // installs hooks after the commit (avoids an early capture race)

    let wt = dir.join("wt-ratelimit");
    let (rc, out) = git_with_hugit(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/rate-limit",
            wt.to_str().unwrap(),
        ],
    );
    assert_eq!(rc, 0, "worktree add: {out}");

    let log = hugit_cli::runtime_store::for_repo(&dir)
        .expect("runtime store resolves")
        .canonical_log();
    let admin_dir = dir.join(".git/worktrees/wt-ratelimit");
    let marker_ok = wait_until(30000, || admin_dir.join(DOCK_MARKER).exists());
    if !marker_ok {
        let hl = dir.join(".hugit/hooks.log");
        panic!(
            "e2e marker appears (async hook) — hooks.log:\n{}",
            std::fs::read_to_string(&hl).unwrap_or_else(|_| "<no hooks.log>".into())
        );
    }

    let rec = wait_until(30000, || {
        dock_records(&log).into_iter().any(|r| {
            payload_of(&r).get("branch").and_then(|b| b.as_str()) == Some("feat/rate-limit")
        })
    });
    assert!(rec, "e2e dock.record lands with the worktree branch");

    let recs = dock_records(&log);
    let rl = recs
        .iter()
        .find(|r| payload_of(r).get("branch").and_then(|b| b.as_str()) == Some("feat/rate-limit"))
        .expect("dock for the worktree");
    let p = payload_of(rl);
    assert_eq!(p.get("origin").unwrap(), "worktree");
    assert_eq!(p.get("charter").unwrap(), "add rate limit");

    // S1 idempotent across a re-checkout in the SAME worktree (branch switch;
    // NOT main — that branch is already checked out in the main worktree).
    let (rc, _) = git_with_hugit(&wt, &["switch", "-q", "-c", "feat/other"]);
    assert_eq!(rc, 0, "worktree re-checkout lands");
    std::thread::sleep(Duration::from_millis(500)); // allow the async hook to fire
    let rate_limit_count = recs
        .iter()
        .filter(|r| payload_of(r).get("branch").and_then(|b| b.as_str()) == Some("feat/rate-limit"))
        .count();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        dock_records(&log).len(),
        rate_limit_count,
        "one dock for the worktree branch — switch did not duplicate outside it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── e2e R1: clone with NO hooks coins nothing (honest absence; self-heal is DOCK-2) ──

#[test]
fn e2e_clone_without_hooks_coins_nothing() {
    let src = scratch("e2e-src");
    git_in(&src, &["init", "-q", "-b", "main"]);
    git_in(&src, &["config", "user.email", "t@t"]);
    git_in(&src, &["config", "user.name", "t"]);
    std::fs::write(src.join("f.txt"), "f").unwrap();
    git_in(&src, &["add", "-A"]);
    git_in(&src, &["commit", "-q", "-m", "init"]);
    lib_init(&src); // hooks in the SOURCE
    std::fs::write(src.join("g.txt"), "g").unwrap();
    git_in(&src, &["add", "g.txt"]);
    git_in(&src, &["commit", "-q", "-m", "more"]);
    std::thread::sleep(Duration::from_millis(500)); // let source's own hooks settle

    let dst = scratch("e2e-dst");
    let (rc, _) = git_with_hugit(
        dst.parent().unwrap(),
        &["clone", "-q", src.to_str().unwrap(), dst.to_str().unwrap()],
    );
    assert_eq!(rc, 0, "clone lands");

    // The clone has NO hooks (git does not transfer them) → the post-checkout
    // hook of the clone is absent → NO dock.record and NO marker (honest R1).
    let log = dst.join(".hugit/log.json");
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !log.exists(),
        "clone has no hugit log (no init) — nothing coined"
    );
    assert!(
        !dst.join(".git/hugit-dock").exists(),
        "no marker in the clone's gitdir — R1 honest absence"
    );

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}
