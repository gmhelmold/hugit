#![cfg(unix)]

//! W2 — `hugit pr open --commit <oid>` bundles CAPTURED raw commits (the bundle
//! from silent-hook captures) as EXTERNAL PR members (2026-09-03).
//!
//! The owner direction: the LLM commits via git; the silent hooks capture the
//! `ref.update`; `hugit pr open` must then bundle that commit oid into a PR —
//! as a raw-commit member (`commit_ids`), NEVER forged into an intent. This
//! suite proves it END-TO-END through the REAL binary + the REAL log seam:
//!
//! 1. A log seeded with a real `intent new --log` (`intent.landed`) + a real
//!    `capture commit --oid aaaa1111` (`ref.update {"ref","target","branch"}`).
//! 2. `hugit pr open --intent <i1> --commit aaaa1111` succeeds and the
//!    `pr.opened` payload carries BOTH the intent id and the commit id.
//! 3. A nonexistent commit oid → structured `commit_not_found` (NOT
//!    `missing_intents` — the honest distinction).
//! 4. `--commit-ref <ref>` resolves to the ref's latest captured target.
//! 5. The no-fake-intent invariant holds: `intent.landed` set is unchanged, no
//!    intent is ever forged from a raw commit.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-pr-commit-journey-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` in `cwd` — returns (exit code, parsed stdout JSON).
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

fn log_records(log: &Path) -> Vec<Value> {
    let bytes = std::fs::read(log).expect("log readable");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| vec![])
}

fn wait_for_capture(log: &Path, oid: &str) {
    for _ in 0..100 {
        let captured = log_records(log).iter().any(|record| {
            record["kind"] == "ref.update"
                && serde_json::from_str::<Value>(record["payload"].as_str().unwrap_or(""))
                    .ok()
                    .and_then(|payload| payload["target"].as_str().map(|target| target == oid))
                    == Some(true)
        });
        if captured && worker_is_idle(log) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("capture worker did not project {oid}");
}

fn worker_is_idle(log: &Path) -> bool {
    let mut lock = log.as_os_str().to_os_string();
    lock.push(".lock");
    let root = log.parent().unwrap();
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

fn wait_for_worker(log: &Path) {
    for _ in 0..100 {
        if worker_is_idle(log)
            && log_records(log)
                .iter()
                .any(|record| record["kind"] == "ref.update")
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("capture worker did not become idle");
}

/// Seed the log with the two raw vocabularies W2 needs: an `intent.landed`
/// (via the real `intent new --log` verb) and a captured `ref.update`
/// (via the real `capture commit` verb — the silent-hook seam).
fn seed_log(root: &Path, log: &Path, oid: &str, branch: &str) {
    std::fs::write(log, b"[]\n").expect("empty log file");

    let (code, v) = run_in(
        root,
        &[
            "intent",
            "new",
            "--charter",
            "w2 journey charter",
            "--campaign",
            "camp",
            "--id",
            "i1",
            "--store",
            root.join("intents.json").to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "intent new exits 0: {v}");

    let (code, _) = run_in(
        root,
        &[
            "capture",
            "--kind",
            "commit",
            "--top-level",
            root.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
            "--oid",
            oid,
            "--branch",
            branch,
        ],
    );
    assert_eq!(code, 0, "capture commit exits 0");
    wait_for_capture(log, oid);
}

#[test]
fn pr_open_bundles_captured_commit_as_external_member() {
    let root = scratch("open-ok");
    let log = root.join("log.json");
    seed_log(&root, &log, "aaaa1111", "main");

    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-1",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--intent",
            "i1",
            "--commit",
            "aaaa1111",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "open with captured commit exits 0: {v}");
    assert_eq!(v["intent_ids"], json!(["i1"]), "intent id present");
    assert_eq!(v["commit_ids"], json!(["aaaa1111"]), "commit id present");
    assert_eq!(v["commit_count"], json!(1));

    // The `pr.opened` PAYLOAD on the log carries both — intent_ids AND
    // commit_ids, side by side, never merged.
    let opened = log_records(&log)
        .into_iter()
        .find(|r| r["kind"] == "pr.opened")
        .expect("pr.opened on the log");
    let payload: Value = serde_json::from_str(opened["payload"].as_str().unwrap()).unwrap();
    assert_eq!(payload["intent_ids"], json!(["i1"]));
    assert_eq!(payload["commit_ids"], json!(["aaaa1111"]));

    // No-fake-intent invariant: exactly ONE intent.landed (the seeded one) —
    // the raw commit was never forged into an intent.
    let landed = log_records(&log)
        .into_iter()
        .filter(|r| r["kind"] == "intent.landed")
        .count();
    assert_eq!(landed, 1, "no intent.landed forged for the raw commit");
}

#[test]
fn pr_open_uncaptured_commit_is_commit_not_found() {
    let root = scratch("open-missing");
    let log = root.join("log.json");
    seed_log(&root, &log, "aaaa1111", "main");

    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-BAD",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--intent",
            "i1",
            "--commit",
            "bbbb2222",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 2, "uncaptured commit exits 2: {v}");
    assert_eq!(
        v["error"]["kind"], "commit_not_found",
        "honest kind, NOT missing_intents: {v}"
    );
    assert_eq!(v["error"]["missing_commits"], json!(["bbbb2222"]));
    // Refused — nothing appended.
    assert_eq!(
        log_records(&log).len(),
        2,
        "only the seeded intent.landed + ref.update remain"
    );
}

#[test]
fn pr_open_commit_ref_resolves_to_captured_target() {
    let root = scratch("open-ref");
    let log = root.join("log.json");
    seed_log(&root, &log, "aaaa1111", "main");

    // `--commit-ref refs/heads/main` resolves to the captured target aaaa1111.
    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-2",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--intent",
            "i1",
            "--commit-ref",
            "refs/heads/main",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "--commit-ref resolves: {v}");
    assert_eq!(
        v["commit_ids"],
        json!(["aaaa1111"]),
        "the ref's captured target became the commit member"
    );

    // An uncaptured ref is commit_not_found.
    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-BADREF",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--intent",
            "i1",
            "--commit-ref",
            "refs/heads/never",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 2, "uncaptured ref exits 2: {v}");
    assert_eq!(
        v["error"]["kind"], "commit_not_found",
        "an uncaptured ref is commit_not_found: {v}"
    );
    assert_eq!(
        v["error"]["missing_commits"],
        json!(["refs/heads/never"]),
        "the unresolved ref token is named"
    );
}

#[test]
fn pr_open_commits_only_never_forges_intent() {
    let root = scratch("open-commits-only");
    let log = root.join("log.json");
    seed_log(&root, &log, "aaaa1111", "main");

    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-9",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--commit",
            "aaaa1111",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "commits-only PR opens: {v}");
    assert_eq!(v["intent_ids"], json!([]), "no intent forged");
    assert_eq!(v["commit_ids"], json!(["aaaa1111"]));

    let opened = log_records(&log)
        .into_iter()
        .find(|r| r["kind"] == "pr.opened")
        .expect("pr.opened on the log");
    let payload: Value = serde_json::from_str(opened["payload"].as_str().unwrap()).unwrap();
    assert_eq!(payload["intent_ids"], json!([]), "payload has no intent");
    assert_eq!(payload["commit_ids"], json!(["aaaa1111"]));

    let landed = log_records(&log)
        .into_iter()
        .filter(|r| r["kind"] == "intent.landed")
        .count();
    assert_eq!(landed, 1, "intent.landed set unchanged — nothing forged");
}

/// A captured push-attempt (`{attempt:true, updates:[...]}`) is a captured-
/// commit proof: `pr open --commit <pushed-sha>` must accept the local OID the
/// pre-push hook recorded, exactly like a post-commit `target`.
///
/// This closes the gap: a commit that a raw `git push` carries (the LLM's
/// normal action) is provable even before/without any `pr open --commit` run.
#[test]
fn pr_open_accepts_a_pushed_commit_as_captured_proof() {
    let root = scratch("open-pushed");
    let log = root.join("log.json");
    std::fs::write(&log, b"[]\n").expect("empty log file");

    // The pre-push hook records bounded typed LOCAL OIDs. Simulate its exact
    // call shape; no raw refspec or remote URL reaches capture.
    let (code, _) = run_in(
        &root,
        &[
            "capture",
            "--kind",
            "push-attempt",
            "--top-level",
            root.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
            "--push-tuples",
            "refs/heads/main\tdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef\trefs/heads/main\t0000000000000000000000000000000000000000",
        ],
    );
    assert_eq!(code, 0, "push-attempt captures");
    wait_for_worker(&log);

    // The pushed sha is NOW a captured-commit proof.
    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-PUSHED",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--commit",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(
        code, 0,
        "pr open accepts the pushed sha as captured proof: {v}"
    );
    assert_eq!(
        v["commit_ids"],
        json!(["deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"])
    );

    // A sha never seen (neither target nor shas) is still commit_not_found.
    let (code, v) = run_in(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-PUSHED2",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--commit",
            "cafebabecafebabecafebabecafebabecafebabe",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 2, "an unseen sha refuses: {v}");
    assert_eq!(
        v["error"]["kind"], "commit_not_found",
        "unseen sha is commit_not_found: {v}"
    );
}
