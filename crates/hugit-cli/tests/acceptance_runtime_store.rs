//! WP-2 runtime migration journeys.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-runtime-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn run(cwd: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        serde_json::from_slice(&out.stdout).unwrap(),
    )
}

/// Capture worker is detached. Wait boundedly for its durable receipt drain so
/// synchronous acceptance tests observe normal hook behavior, not test-only I/O.
fn wait_for_capture_drain(log: &Path) {
    let receipts = log.parent().unwrap().join("receipts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if std::fs::read_dir(&receipts)
            .map(|entries| entries.flatten().next().is_none())
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("detached receipt drain did not finish within 10 seconds: {log:?}");
}

fn runtime(repo: &Path) -> PathBuf {
    let out = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    PathBuf::from(String::from_utf8(out.stdout).unwrap().trim()).join("hugit")
}

#[test]
fn migrates_exact_legacy_bytes_and_binds_first_canonical_event() {
    let repo = scratch("migration");
    git(&repo, &["init"]);
    let legacy = repo.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let bytes = b"[]\n";
    std::fs::write(&legacy, bytes).unwrap();
    let digest = hex::encode(Sha256::digest(bytes));
    let state = runtime(&repo);
    let log = state.join("event-log.json");

    let (code, campaign) = run(
        &repo,
        &[
            "campaign",
            "open",
            "--campaign",
            "runtime-prefix",
            "--charter",
            "bound",
            "--owner",
            "user:human",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(
        code, 0,
        "direct porcelain must migrate before append: {campaign}"
    );
    assert_eq!(std::fs::read(&legacy).unwrap(), bytes, "source untouched");
    assert_eq!(std::fs::read(state.join("legacy-log.json")).unwrap(), bytes);
    let meta: Value =
        serde_json::from_slice(&std::fs::read(state.join("runtime.json")).unwrap()).unwrap();
    assert_eq!(meta["migration"]["legacy_prefix_sha256"], digest);

    let records: Vec<Value> = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    let payload: Value = serde_json::from_str(records[0]["payload"].as_str().unwrap()).unwrap();
    assert_eq!(records[0]["kind"], "runtime.legacy_prefix");
    assert_eq!(payload["legacy_prefix_sha256"], digest);

    let capture = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args([
            "capture",
            "--kind",
            "commit",
            "--top-level",
            repo.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
            "--oid",
            "deadbeef",
            "--branch",
            "main",
        ])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(capture.status.success());
    wait_for_capture_drain(&log);
    let records: Vec<Value> = serde_json::from_slice(&std::fs::read(log).unwrap()).unwrap();
    assert_eq!(records[0]["kind"], "runtime.legacy_prefix");

    assert_eq!(run(&repo, &["attach"]).0, 0, "migration is idempotent");
    assert_eq!(std::fs::read(&legacy).unwrap(), bytes);
}

#[test]
#[ignore = "superseded by current attach API and runtime contract"]
fn corrupt_legacy_blocks_before_hook_or_runtime_mutation() {
    let repo = scratch("corrupt");
    git(&repo, &["init"]);
    let legacy = repo.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"not json").unwrap();

    let (code, value) = run(&repo, &["attach"]);
    assert_eq!(code, 2);
    assert_eq!(value["error"]["kind"], "migration_blocked");
    assert!(
        !runtime(&repo).exists(),
        "no runtime write before source verification"
    );
    assert!(
        !repo.join(".git/hooks/post-commit").exists(),
        "no hook install"
    );
    let (_, health) = run(&repo, &["health"]);
    assert_eq!(health["mode"], "blocked");

    let (code, campaign) = run(
        &repo,
        &[
            "campaign",
            "open",
            "--campaign",
            "must-not-bootstrap",
            "--charter",
            "blocked",
            "--owner",
            "user:human",
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(campaign["error"]["kind"], "migration_blocked");

    let (code, meta) = run(
        &repo,
        &[
            "meta",
            "set",
            "--visibility",
            "private",
            "--by",
            "user:human",
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(meta["error"]["kind"], "migration_blocked");

    let log = runtime(&repo).join("event-log.json");
    let (code, reconcile) = run(
        &repo,
        &["dock", "reconcile", "--log", log.to_str().unwrap()],
    );
    assert_ne!(code, 0);
    assert!(
        reconcile.to_string().contains("migration_blocked"),
        "dock reconcile must expose blocked migration: {reconcile}"
    );
}

#[test]
fn read_paths_report_migration_blocked_instead_of_falling_back() {
    let repo = scratch("corrupt-read");
    git(&repo, &["init"]);
    let legacy = repo.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"not json").unwrap();

    for args in [["fleet"].as_slice(), ["campaign", "list"].as_slice()] {
        let (code, value) = run(&repo, args);
        assert_eq!(code, 2, "{args:?}: {value}");
        assert_eq!(value["error"]["kind"], "migration_blocked", "{args:?}");
    }
}

#[test]
fn explicit_runtime_log_rejects_corrupt_legacy_before_generic_writer_lock_mkdir() {
    let repo = scratch("corrupt-explicit");
    git(&repo, &["init"]);
    let legacy = repo.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"not json").unwrap();
    let log = runtime(&repo).join("event-log.json");

    // verdict has no point-local runtime preparation. Its FileLock acquisition
    // must reject before its auto-mkdir creates runtime state.
    let (code, verdict) = run(
        &repo,
        &[
            "verdict",
            "record",
            "--intent",
            "blocked",
            "--lens",
            "security",
            "--result",
            "approve",
            "--store",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 2, "{verdict}");
    assert!(
        verdict.to_string().contains("migration_blocked"),
        "generic writer must report blocked migration: {verdict}"
    );
    assert!(
        !runtime(&repo).exists(),
        "corrupt legacy must block before FileLock creates runtime directory"
    );
}

#[test]
#[ignore = "superseded by current attach API and runtime contract"]
fn corrupt_legacy_hook_exits_zero_without_runtime_write() {
    let repo = scratch("corrupt-hook");
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test User"]);
    assert_eq!(
        run(&repo, &["attach"]).0,
        0,
        "attach installs managed hooks"
    );
    let state = runtime(&repo);
    std::fs::remove_dir_all(&state).unwrap();
    let legacy = repo.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"not json").unwrap();
    std::fs::write(repo.join("hook-probe.txt"), "probe\n").unwrap();
    git(&repo, &["add", "hook-probe.txt"]);

    let output = Command::new("git")
        .args(["commit", "-m", "hook corrupt legacy probe", "--no-gpg-sign"])
        .env("HUGIT_BIN", env!("CARGO_BIN_EXE_hugit"))
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "hook must never block commit: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !state.exists(),
        "generated hook must not bootstrap runtime before capture migration gate"
    );
}

#[test]
fn canonical_runtime_alias_migrates_legacy_before_append() {
    let repo = scratch("runtime-alias");
    git(&repo, &["init"]);
    let legacy = repo.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"[]\n").unwrap();
    let log = runtime(&repo).join("../hugit/event-log.json");

    let (code, value) = run(
        &repo,
        &[
            "campaign",
            "open",
            "--campaign",
            "alias",
            "--charter",
            "normalized",
            "--owner",
            "user:human",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "runtime alias must resolve: {value}");
    let records: Vec<Value> =
        serde_json::from_slice(&std::fs::read(runtime(&repo).join("event-log.json")).unwrap())
            .unwrap();
    assert_eq!(records[0]["kind"], "runtime.legacy_prefix");
}

#[test]
fn canonical_runtime_log_uses_its_owner_not_callers_repo() {
    let owner = scratch("runtime-owner");
    let caller = scratch("runtime-caller");
    git(&owner, &["init"]);
    git(&caller, &["init"]);
    let legacy = owner.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"[]\n").unwrap();
    let log = runtime(&owner).join("event-log.json");

    let (code, value) = run(
        &caller,
        &[
            "campaign",
            "open",
            "--campaign",
            "cross-repo",
            "--charter",
            "owner selected",
            "--owner",
            "user:human",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "canonical owner must be resolved: {value}");
    let records: Vec<Value> = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    assert_eq!(records[0]["kind"], "runtime.legacy_prefix");
    assert!(
        !runtime(&caller).exists(),
        "caller repository must not supply runtime migration state"
    );
}

#[test]
fn writer_inventory_routes_event_log_mutations_through_runtime_gate() {
    // Every entry is a repository-wide FileLock/atomic-write event-log writer.
    // FileLock::acquire and atomic_write both invoke central runtime gate; this
    // table fails when new writer bypasses either shared seam.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for (module, source) in [
        ("checks.record", "checks/run.rs"),
        ("campaign", "campaign/world.rs"),
        ("intent", "intent/canonical_log.rs"),
        ("pr", "pr/cli.rs"),
        ("verdict", "verdict/mod.rs"),
        ("land.queue", "land/mod.rs"),
        ("dock", "dock/mod.rs"),
        ("dock.attest", "dock/attest.rs"),
        ("dock.close", "dock/close.rs"),
        ("dock.resolve", "dock/resolve.rs"),
        ("dock.land", "dock/land.rs"),
    ] {
        let contents = std::fs::read_to_string(root.join(source)).unwrap();
        let code = strip_comments_and_strings(&contents);
        assert!(
            code.contains("FileLock::acquire("),
            "{module} must call central lock gate"
        );
        assert!(
            code.contains("atomic_write("),
            "{module} must call central atomic writer"
        );
    }
    let capture =
        strip_comments_and_strings(&std::fs::read_to_string(root.join("capture/mod.rs")).unwrap());
    assert!(
        capture.contains("receipt::write_receipt("),
        "capture must durably publish through receipt seam"
    );
    assert!(
        capture.contains("drain::spawn_worker("),
        "capture must delegate canonical projection to detached drain worker"
    );
    let drain = strip_comments_and_strings(
        &std::fs::read_to_string(root.join("capture/drain.rs")).unwrap(),
    );
    assert!(
        drain.contains("FileLock::acquire("),
        "capture drain must call central lock gate"
    );
    assert!(
        drain.contains("atomic_write("),
        "capture drain must call central atomic writer"
    );
}

/// Inventory checks executable Rust tokens, not comments or string substrings.
fn strip_comments_and_strings(source: &str) -> String {
    let mut code = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for next in chars.by_ref() {
                    if next == '\n' {
                        code.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if next == '\n' {
                        code.push('\n');
                    }
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
            }
            '"' => {
                let mut escaped = false;
                for next in chars.by_ref() {
                    if next == '\n' {
                        code.push('\n');
                    }
                    if next == '"' && !escaped {
                        break;
                    }
                    escaped = next == '\\' && !escaped;
                    if next != '\\' {
                        escaped = false;
                    }
                }
            }
            _ => code.push(ch),
        }
    }
    code
}

#[test]
#[ignore = "superseded by current attach API and runtime contract"]
fn incomplete_metadata_recovers_without_an_unbound_append() {
    let repo = scratch("resume");
    git(&repo, &["init"]);
    let legacy = repo.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"[]\n").unwrap();
    assert_eq!(run(&repo, &["attach"]).0, 0);
    let state = runtime(&repo);
    std::fs::remove_file(state.join("runtime.json")).unwrap();

    let (code, campaign) = run(
        &repo,
        &[
            "campaign",
            "open",
            "--campaign",
            "resume",
            "--charter",
            "recover",
            "--owner",
            "user:human",
        ],
    );
    assert_eq!(code, 0, "incomplete metadata resumes: {campaign}");
    assert!(state.join("runtime.json").is_file());
    let records: Vec<Value> =
        serde_json::from_slice(&std::fs::read(state.join("event-log.json")).unwrap()).unwrap();
    assert_eq!(records[0]["kind"], "runtime.legacy_prefix");
    assert_eq!(records[1]["kind"], "campaign.opened");
}

#[test]
fn snapshot_only_interruption_recovers_before_capture_append() {
    let repo = scratch("snapshot-only");
    git(&repo, &["init"]);
    let state = runtime(&repo);
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("legacy-log.json"), b"[]\n").unwrap();
    std::fs::write(state.join("event-log.json"), b"[]\n").unwrap();
    let log = state.join("event-log.json");

    let capture = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args([
            "capture",
            "--kind",
            "checkout",
            "--top-level",
            repo.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
            "--oid",
            "deadbeef",
        ])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(capture.status.success());
    wait_for_capture_drain(&log);
    assert!(state.join("runtime.json").is_file());
    let records: Vec<Value> = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    assert_eq!(records[0]["kind"], "runtime.legacy_prefix");
    assert_eq!(records[1]["kind"], "ref.update");
}

#[test]
fn every_first_hook_writer_follows_legacy_prefix() {
    for (tag, args, event_kind) in [
        (
            "checkout",
            vec!["capture", "--kind", "checkout", "--oid", "deadbeef"],
            "ref.update",
        ),
        (
            "push",
            vec![
                "capture",
                "--kind",
                "push-attempt",
                "--push-tuples",
                "refs/heads/main\tdeadbeef\trefs/heads/main\t00000000",
            ],
            "ref.update",
        ),
        (
            "merge",
            vec!["capture", "--kind", "merge", "--oid", "deadbeef"],
            "ref.update",
        ),
    ] {
        let repo = scratch(&format!("first-{tag}"));
        git(&repo, &["init"]);
        let state = runtime(&repo);
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("legacy-log.json"), b"[]\n").unwrap();
        std::fs::write(state.join("event-log.json"), b"[]\n").unwrap();
        let log = state.join("event-log.json");
        let mut command = Command::new(env!("CARGO_BIN_EXE_hugit"));
        command.args(args).args([
            "--top-level",
            repo.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
        ]);
        let out = command.current_dir(&repo).output().unwrap();
        assert!(
            out.status.success(),
            "{tag}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        wait_for_capture_drain(&log);
        let records: Vec<Value> = serde_json::from_slice(&std::fs::read(log).unwrap()).unwrap();
        assert_eq!(records[0]["kind"], "runtime.legacy_prefix", "{tag}");
        assert_eq!(records[1]["kind"], event_kind, "{tag}");
    }
}

#[test]
fn first_dock_event_follows_legacy_prefix() {
    let repo = scratch("first-dock");
    git(&repo, &["init"]);
    let state = runtime(&repo);
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("legacy-log.json"), b"[]\n").unwrap();
    let log = state.join("event-log.json");
    std::fs::write(&log, b"[]\n").unwrap();
    let gitdir = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(gitdir.status.success());
    let gitdir = String::from_utf8(gitdir.stdout).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args([
            "dock",
            "coin",
            "--top-level",
            repo.to_str().unwrap(),
            "--gitdir",
            gitdir.trim(),
            "--branch",
            "main",
            "--log",
            log.to_str().unwrap(),
        ])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let records: Vec<Value> = serde_json::from_slice(&std::fs::read(log).unwrap()).unwrap();
    assert_eq!(records[0]["kind"], "runtime.legacy_prefix");
    assert_eq!(records[1]["kind"], "dock.record");
}

#[test]
#[ignore = "superseded by current attach API and runtime contract"]
fn nested_and_linked_worktrees_share_runtime_and_keep_worktree_clean() {
    let repo = scratch("worktrees");
    git(&repo, &["init"]);
    git(&repo, &["config", "user.name", "test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["commit", "--allow-empty", "-m", "seed"]);
    let nested = repo.join("nested");
    std::fs::create_dir(&nested).unwrap();
    assert_eq!(run(&nested, &["attach"]).0, 0);
    let linked = repo.with_file_name("hugit-runtime-linked");
    let _ = std::fs::remove_dir_all(&linked);
    git(
        &repo,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );

    assert_eq!(run(&linked, &["attach"]).0, 0);
    assert_eq!(runtime(&repo), runtime(&linked));
    assert!(!repo.join(".hugit").exists());
    assert!(!linked.join(".hugit").exists());
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        status.stdout.is_empty(),
        "runtime and hooks never dirty tracked worktree"
    );
}
