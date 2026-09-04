//! MCP capture tool — checkout + merge captures proven e2e against REAL `jj`.
//!
//! `jj` fires no git hooks (`jj new` + `jj describe` + `jj git export` write
//! refs directly), so the agent layer records each event through the REAL MCP
//! capture tool (`hugit_mcp::tools::capture::run`) — the same seam the silent
//! git hooks use — into the canonical `.hugit/log.json`. This suite proves the
//! fix: the tool's `confirm_on_log` now confirms a checkout `ref.update` by its
//! `to` payload key (not only `target`).
//!
//! The journey: init a jj colocated repo → install the canonical log via lib
//! init → `jj describe` + `jj git export` a real change → CAPTURE the checkout
//! through the tool (confirmed `seq` + `event_hash`) → `jj squash` a third
//! change (a `git merge --squash` analog) → CAPTURE the merge through the tool
//! → assert the payloads landed → `hugit pr open --commit` proves the captured
//! merge commit is accepted as a PR member.
//!
//! Fail-closed: if the `jj` binary is absent, the live lane SKIPs loudly (the
//! hermetic `confirm_on_log` match tests in `hugit-mcp` remain the
//! deterministic gate).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

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

/// Run `hugit init` via the library (the binary verb is X5-reserved). Leaves
/// the existing jj `.git` untouched and adds the canonical `.hugit/` log.
fn lib_init(dir: &Path) {
    let code = run_init(InitArgs {
        dir: Some(dir.to_path_buf()),
    });
    assert_eq!(code, std::process::ExitCode::SUCCESS, "lib init exits 0");
}

/// Read the canonical log (a bare `[EventRecord, ...]` array).
fn log_records(log: &Path) -> Vec<Value> {
    let bytes = std::fs::read(log).expect("log readable");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| vec![])
}

/// The canonical log's most-recent `ref.update` whose payload satisfies `pred`.
fn last_ref_update(log: &Path, pred: impl Fn(&Value) -> bool) -> Option<Value> {
    log_records(log).iter().rev().find_map(|r| {
        if r.get("kind").and_then(Value::as_str) != Some("ref.update") {
            return None;
        }
        let payload: Value = r
            .get("payload")
            .and_then(Value::as_str)
            .and_then(|p| serde_json::from_str(p).ok())
            .unwrap_or(Value::Null);
        if pred(&payload) { Some(payload) } else { None }
    })
}

/// Drive the REAL MCP capture tool (the fn the MCP server calls) against the
/// REAL hugit binary under test → the tool's JSON result. `verify` is forced
/// true — an unconfirmed capture must be a tool error, never a claim.
fn mcp_capture(
    dir: &Path,
    kind: &str,
    oid: &str,
    from: Option<&str>,
    branch: Option<&str>,
) -> Value {
    let log = dir.join(".hugit/log.json");
    let mut args = json!({
        "kind": kind,
        "top_level": dir.to_str().unwrap(),
        "log": log.to_str().unwrap(),
        "oid": oid,
        "hugit_bin": env!("CARGO_BIN_EXE_hugit"),
        "verify": true,
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

    // CAPTURE the checkout via the REAL MCP tool (verify forced on). Pre-fix
    // this failed: a checkout payload carries `to`, not `target` — the confirm
    // read must match `to`.
    let result = mcp_capture(
        &dir,
        "checkout",
        &oid_feat,
        Some(&oid_super),
        Some(&cid_feat),
    );
    assert_eq!(
        result["status"],
        json!("captured"),
        "checkout capture is CONFIRMED: {result}"
    );
    let seq_checkout = result["seq"].as_u64().expect("checkout seq present");
    let hash_checkout = result["event_hash"]
        .as_str()
        .expect("checkout hash present");
    assert!(
        !hash_checkout.is_empty(),
        "checkout hash is a nonempty string"
    );
    let _ = seq_checkout;

    // Read the canonical log back: the most-recent `checkout:true` payload is
    // the confirmed record (to == oid_feat, from == oid_super, branch ==
    // cid_feat).
    let payload = last_ref_update(&dir.join(".hugit/log.json"), |p| {
        p.get("checkout").and_then(Value::as_bool) == Some(true)
    })
    .expect("a checkout:true ref.update exists");
    assert_eq!(payload["checkout"], json!(true));
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

    // CAPTURE the merge-ish squash via the SAME tool — the confirm matches the
    // `target` key (the merge payload shape). No `branch` for a merge.
    let result = mcp_capture(&dir, "merge", &oid_squashed, Some(&oid_c3), None);
    assert_eq!(
        result["status"],
        json!("captured"),
        "merge capture is CONFIRMED: {result}"
    );
    let _seq_merge = result["seq"].as_u64().expect("merge seq present");
    let hash_merge = result["event_hash"].as_str().expect("merge hash present");
    assert!(!hash_merge.is_empty(), "merge hash is a nonempty string");

    // The squash landed as `merged_from → target` (from == oid_c3, target ==
    // oid_squashed).
    let merged = last_ref_update(&dir.join(".hugit/log.json"), |p| {
        p.get("merged_from").and_then(Value::as_str) == Some(oid_c3.as_str())
            && p.get("target").and_then(Value::as_str) == Some(oid_squashed.as_str())
    })
    .expect("the merge ref.update exists");
    assert_eq!(merged["merged_from"].as_str(), Some(oid_c3.as_str()));
    assert_eq!(merged["target"].as_str(), Some(oid_squashed.as_str()));

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
