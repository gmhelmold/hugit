//! MCP capture tool — checkout + merge captures proven e2e against REAL `jj`.
//!
//! `jj` fires no git hooks (`jj new` + `jj describe` + `jj git export` write
//! refs directly), so the agent layer records each event through the REAL MCP
//! capture tool (`hugit_mcp::tools::capture::run`) — the same seam the silent
//! git hooks use — into common-dir runtime state. MCP reports dispatch only;
//! receipt completion is separately observed through drain-writer inventory.
//!
//! The journey: init a jj colocated repo → install the canonical log via lib
//! init → `jj describe` + `jj git export` a real change → CAPTURE checkout
//! through tool (honest `dispatched`, then separate record read) → `jj squash` a
//! third change (a `git merge --squash` analog) → CAPTURE merge through tool
//! → assert the payloads landed → `hugit pr open --commit` proves the captured
//! merge commit is accepted as a PR member.
//!
//! Fail-closed: if the `jj` binary is absent, the live lane SKIPs loudly (the
//! hugit-mcp dispatch tests remain deterministic gate).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use hugit_cli::init::{InitArgs, run as run_init};

/// Resolve the `jj` binary on PATH (fail-closed: None → skip).
fn jj_bin() -> Option<String> {
    let probe = Command::new("jj").arg("--version").output().ok()?;
    if probe.status.success() {
        Some("jj".to_string())
    } else {
        None
    }
}

/// Make a unique scratch dir (removed if it exists from a prior run).
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-jj-capture-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `jj <args>` in `cwd` → (exit, stdout). The colocated repo's git refs
/// only move after `jj git export`, so the flow calls export after every
/// describe.
fn jj(cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new("jj")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("jj runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn git_parents(cwd: &Path, oid: &str) -> Vec<String> {
    let out = Command::new("git")
        .args(["rev-list", "--parents", "-n", "1", oid])
        .current_dir(cwd)
        .output()
        .expect("git rev-list runs");
    assert!(out.status.success(), "git rev-list: {:?}", out.stderr);
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

/// Run `hugit init` via the library (the binary verb is X5-reserved). Leaves
/// the existing jj `.git` untouched and adds the canonical `.hugit/` log.
fn lib_init(dir: &Path) {
    let code = run_init(InitArgs {
        dir: Some(dir.to_path_buf()),
    });
    assert_eq!(code, std::process::ExitCode::SUCCESS, "lib init exits 0");
}

fn git_init(dir: &Path) {
    let out = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir)
        .output()
        .expect("git init runs");
    assert!(out.status.success(), "git init: {:?}", out.stderr);
}

/// Read the canonical log (a bare `[EventRecord, ...]` array).
fn log_records(log: &Path) -> Vec<Value> {
    let bytes = std::fs::read(log).expect("log readable");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| vec![])
}

fn runtime_log(repo: &Path) -> PathBuf {
    hugit_cli::runtime_store::for_repo(repo)
        .expect("Git runtime store resolves")
        .canonical_log()
}

/// Drain owns canonical log projection. A visible record alone is not completion:
/// wait until every published receipt has its durable completion marker and the
/// writer lock is gone before a next writer starts.
fn drain_is_idle(log: &Path) -> bool {
    let mut lock = log.as_os_str().to_os_string();
    lock.push(".lock");
    let root = log.parent().expect("runtime log parent");
    let receipts = root.join("receipts");
    let completed = root.join("completed-receipts");
    !PathBuf::from(lock).exists()
        && std::fs::read_dir(receipts)
            .map(|entries| {
                entries.flatten().all(|entry| {
                    let path = entry.path();
                    let Some(receipt_id) = path.file_stem().and_then(|name| name.to_str()) else {
                        return true;
                    };
                    completed.join(format!("{receipt_id}.done")).is_file()
                })
            })
            .unwrap_or(false)
}

/// The canonical log's most-recent completed `ref.update` whose payload
/// satisfies `pred`.
fn last_ref_update(log: &Path, pred: impl Fn(&Value) -> bool) -> Option<Value> {
    // A detached worker may exhaust three bounded 5s log-lock handoffs. Wait
    // only for exact event plus receipt-completion evidence, not wall-clock
    // hope; 20s covers that durable worker contract.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if let Some(payload) = log_records(log).iter().rev().find_map(|r| {
            if r.get("kind").and_then(Value::as_str) != Some("ref.update") {
                return None;
            }
            let payload: Value = r
                .get("payload")
                .and_then(Value::as_str)
                .and_then(|p| serde_json::from_str(p).ok())
                .unwrap_or(Value::Null);
            pred(&payload).then_some(payload)
        }) && drain_is_idle(log)
        {
            return Some(payload);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    None
}

/// Drive real MCP capture tool against real hugit binary. Default Git runtime
/// resolution returns dispatched: CLI owns migration and MCP cannot read back
/// an explicitly addressable same log.
fn mcp_capture(
    dir: &Path,
    kind: &str,
    oid: &str,
    from: Option<&str>,
    branch: Option<&str>,
) -> Value {
    let mut args = json!({
        "kind": kind,
        "top_level": dir.to_str().unwrap(),
        "oid": oid,
        "hugit_bin": env!("CARGO_BIN_EXE_hugit"),
    });
    if let Some(f) = from {
        args["from"] = json!(f);
    }
    if let Some(b) = branch {
        args["branch"] = json!(b);
    }
    match hugit_mcp::tools::capture::run(&args) {
        hugit_mcp::tools::ToolOutcome::Ok(v) => v,
        hugit_mcp::tools::ToolOutcome::Err(e) => panic!("MCP capture tool error: {e}"),
    }
}

/// Drive land-status through its real MCP tool seam against the hugit binary.
fn mcp_land_status(dir: &Path) -> Value {
    let args = json!({
        "top_level": dir.to_str().unwrap(),
        "hugit_bin": env!("CARGO_BIN_EXE_hugit"),
    });
    match hugit_mcp::tools::land_status::run(&args) {
        hugit_mcp::tools::ToolOutcome::Ok(v) => v,
        hugit_mcp::tools::ToolOutcome::Err(e) => panic!("MCP land-status tool error: {e}"),
    }
}

#[test]
fn mcp_capture_uses_runtime_and_delegates_legacy_migration_without_an_explicit_log() {
    let runtime_dir = scratch("runtime-default");
    git_init(&runtime_dir);
    lib_init(&runtime_dir);
    let runtime = runtime_log(&runtime_dir);
    assert!(runtime.is_file(), "init creates common-dir runtime log");
    let runtime_result = mcp_capture(&runtime_dir, "commit", "runtime-oid", None, None);
    assert_eq!(runtime_result["status"], json!("dispatched"));
    assert_eq!(
        runtime_result["log_source"],
        json!("CLI default (Git runtime)")
    );
    assert!(
        last_ref_update(&runtime, |p| p["target"] == "runtime-oid").is_some(),
        "CLI-default capture appends to common-dir runtime log"
    );

    let legacy_dir = scratch("legacy-fallback");
    git_init(&legacy_dir);
    let legacy = legacy_dir.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let legacy_bytes = b"[]\n";
    std::fs::write(&legacy, legacy_bytes).unwrap();
    let digest = hex::encode(Sha256::digest(legacy_bytes));
    let runtime = runtime_log(&legacy_dir);
    assert!(
        !runtime.exists(),
        "regression requires Git legacy state with no runtime log"
    );

    let legacy_result = mcp_capture(&legacy_dir, "commit", "legacy-oid", None, None);
    assert_eq!(legacy_result["status"], json!("dispatched"));
    assert_eq!(
        legacy_result["log_source"],
        json!("CLI default (Git runtime)")
    );
    assert_eq!(
        std::fs::read(&legacy).unwrap(),
        legacy_bytes,
        "source untouched"
    );

    let state = runtime.parent().unwrap();
    assert_eq!(
        std::fs::read(state.join("legacy-log.json")).unwrap(),
        legacy_bytes
    );
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(state.join("runtime.json")).unwrap()).unwrap();
    assert_eq!(metadata["migration"]["legacy_source"], json!(legacy));
    assert_eq!(metadata["migration"]["legacy_prefix_sha256"], json!(digest));

    let migrated = log_records(&runtime);
    assert_eq!(migrated[0]["kind"], json!("runtime.legacy_prefix"));
    let prefix: Value = serde_json::from_str(migrated[0]["payload"].as_str().unwrap()).unwrap();
    assert_eq!(prefix["legacy_prefix_sha256"], json!(digest));
    assert!(
        last_ref_update(&runtime, |p| p["target"] == "legacy-oid").is_some(),
        "valid legacy log migrates before MCP capture appends"
    );
}

#[test]
fn mcp_capture_uses_legacy_log_when_top_level_is_not_git() {
    let dir = scratch("non-git-legacy");
    let legacy = dir.join(".hugit/log.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"[]\n").unwrap();

    let result = mcp_capture(&dir, "commit", "legacy-non-git-oid", None, None);
    assert_eq!(result["status"], json!("dispatched"));
    assert!(
        last_ref_update(&legacy, |p| p["target"] == "legacy-non-git-oid").is_some(),
        "MCP capture falls back to legacy state when top_level is not Git"
    );
}

#[test]
fn mcp_capture_non_git_without_legacy_delegates_to_cli_default() {
    let dir = scratch("non-git-cli-default");
    let expected = dir.join(".git/hugit/event-log.json");

    let result = mcp_capture(&dir, "commit", "default-non-git-oid", None, None);
    assert_eq!(result["status"], json!("dispatched"));
    assert_eq!(result["log"], json!(expected.display().to_string()));
    assert_eq!(result["log_source"], json!("CLI default"));
    assert!(
        !expected.exists(),
        "MCP reports dispatch only; CLI default path does not create a log"
    );
    assert!(
        !dir.join(".hugit/log.json").exists(),
        "MCP never invents a legacy log"
    );
}

#[test]
fn mcp_land_status_non_git_without_legacy_delegates_to_cli_default() {
    let dir = scratch("land-non-git-cli-default");
    let expected = dir.join(".git/hugit/event-log.json");
    std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
    std::fs::write(&expected, b"[]\n").unwrap();

    let result = mcp_land_status(&dir);
    assert_eq!(result["log"], json!(expected.display().to_string()));
    assert_eq!(result["log_source"], json!("CLI default"));
    assert_eq!(
        result["source"],
        json!(format!("{} queue show", env!("CARGO_BIN_EXE_hugit")))
    );
    assert!(
        !dir.join(".hugit/log.json").exists(),
        "land-status never invents a legacy log"
    );
}

/// `jj log` template row for `rev`: the git `commit_id` (the oid, first 40).
fn jj_oid(cwd: &Path, rev: &str) -> String {
    let (code, out) = jj(
        cwd,
        &[
            "log",
            "--no-pager",
            "--no-graph",
            "-T",
            "commit_id ++ \"\\n\"",
            "-r",
            rev,
        ],
    );
    assert_eq!(code, 0, "jj log fails: {out}");
    out.lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(40)
        .collect()
}

/// `jj log` template row for `rev`: `change_id.short()` (the branch name, 12).
fn jj_change_id(cwd: &Path, rev: &str) -> String {
    let (code, out) = jj(
        cwd,
        &[
            "log",
            "--no-pager",
            "--no-graph",
            "-T",
            "change_id.short() ++ \"\\n\"",
            "-r",
            rev,
        ],
    );
    assert_eq!(code, 0, "jj log change_id fails: {out}");
    out.lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(12)
        .collect()
}

#[test]
fn mcp_capture_tool_records_jj_checkout_and_squash_merge_e2e() {
    let Some(_bin) = jj_bin() else {
        eprintln!(
            "SKIP: `jj` binary not found on PATH — the live jj lane is skipped; the hermetic match tests are the gate"
        );
        return;
    };

    let dir = scratch("e2e");
    let init = jj(&dir, &["git", "init"]);
    assert_eq!(init.0, 0, "jj git init: {}", init.1);
    lib_init(&dir);

    // super — the first committed change (the initial tip).
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    assert_eq!(jj(&dir, &["describe", "-m", "super"]).0, 0);
    assert_eq!(jj(&dir, &["git", "export"]).0, 0, "git export syncs refs");
    let oid_super = jj_oid(&dir, "@");

    // `jj new` = checkout a new empty change (jj's `git checkout -b` analog).
    assert_eq!(jj(&dir, &["new"]).0, 0);
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    assert_eq!(jj(&dir, &["describe", "-m", "feat"]).0, 0);
    assert_eq!(jj(&dir, &["git", "export"]).0, 0);
    let oid_feat = jj_oid(&dir, "@");
    let cid_feat = jj_change_id(&dir, "@");

    // CAPTURE checkout via real MCP tool. Git-default resolution delegates to
    // CLI, so silent capture stays honestly dispatched; record read below
    // proves the `to` payload landed.
    let result = mcp_capture(
        &dir,
        "checkout",
        &oid_feat,
        Some(&oid_super),
        Some(&cid_feat),
    );
    assert_eq!(
        result["status"],
        json!("dispatched"),
        "Git-default capture must report dispatch only: {result}"
    );

    // Read canonical log back: checkout is observed fact, not boolean claim.
    let payload = last_ref_update(&runtime_log(&dir), |p| {
        p["checkout"]["truth"] == "branch_checkout_observed"
    })
    .expect("a checkout:true ref.update exists");
    assert_eq!(payload["checkout"]["fact"], "nonzero_old_oid");
    assert_eq!(payload["checkout"]["truth"], "branch_checkout_observed");
    assert_eq!(payload["to"].as_str(), Some(oid_feat.as_str()));
    assert_eq!(payload["from"].as_str(), Some(oid_super.as_str()));
    assert_eq!(payload["branch"].as_str(), Some(cid_feat.as_str()));

    // A third change, then `jj squash` == the jj merge-ish rewrite (a `git
    // merge --squash` analog — absorbs the source change, new tip).
    assert_eq!(jj(&dir, &["new"]).0, 0);
    std::fs::write(dir.join("c.txt"), "c\n").unwrap();
    assert_eq!(jj(&dir, &["describe", "-m", "c3"]).0, 0);
    assert_eq!(jj(&dir, &["git", "export"]).0, 0);
    let oid_c3 = jj_oid(&dir, "@");
    assert_eq!(jj(&dir, &["squash", "-m", "feat+c3"]).0, 0);
    assert_eq!(jj(&dir, &["git", "export"]).0, 0);
    let oid_squashed = jj_oid(&dir, "@");
    assert_ne!(oid_c3, oid_squashed, "the squash produced a NEW tip commit");
    let expected_parents = git_parents(&dir, &oid_squashed);
    let expected_fast_forward = expected_parents.len() == 1
        && expected_parents
            .first()
            .is_some_and(|parent| parent == &oid_c3);
    let expected_merge_kind = if expected_parents.len() > 1 {
        "merge_commit"
    } else if expected_fast_forward {
        "fast_forward"
    } else {
        "unknown"
    };

    // CAPTURE merge-ish squash through same tool. No `branch` for merge.
    let result = mcp_capture(&dir, "merge", &oid_squashed, Some(&oid_c3), None);
    assert_eq!(
        result["status"],
        json!("dispatched"),
        "Git-default capture must report dispatch only: {result}"
    );

    // Merge capture derives topology from immutable Git object data. A jj squash
    // is not asserted as a Git merge parent edge.
    let merged = last_ref_update(&runtime_log(&dir), |p| {
        p.get("target").and_then(Value::as_str) == Some(oid_squashed.as_str())
    })
    .expect("the merge ref.update exists");
    assert_eq!(merged["target"].as_str(), Some(oid_squashed.as_str()));
    assert_eq!(
        merged["parents"],
        json!(vec!["[REDACTED]"; expected_parents.len()]),
        "upstream structural scrub keeps parent count but never leaks OIDs"
    );
    assert_eq!(merged["fast_forward"], json!(expected_fast_forward));
    assert_eq!(merged["merge_kind"], json!(expected_merge_kind));

    // The captured merge result is a REAL git commit → `pr open --commit`
    // accepts it as a PR member (the capture→PR loop closes for merge too).
    // The commit bundle is external (`--commit` oids only; the PR carries it
    // under `commit_ids`, never an intent).
    let out = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args([
            "pr",
            "open",
            "--pr",
            "PR-JJ",
            "--campaign",
            "camp-jj",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "jj-e2e",
            "--commit",
            &oid_squashed,
        ])
        .current_dir(&dir)
        .output()
        .expect("hugit pr open runs");
    assert!(
        out.status.success(),
        "pr open accepts the captured merge commit via the real CLI: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
