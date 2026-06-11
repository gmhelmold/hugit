//! WP-WB0 acceptance — the one error/exit law (legacy verbs converged) + the
//! `checks`/`queue` wedge-visibility stubs, against the REAL `hugit` binary.
//!
//! Proves:
//!   ① every legacy verb (`why`/`impact`/`tournament`/`export`) emits STABLE
//!      JSON on stdout — success AND error — under the one error law
//!      (`{"error":{"kind","message","fix", …}}`), exit `2` on a domain error;
//!   ② the input-error law: a missing `--log` FILE is an explicit
//!      `log_not_found` (never silently an empty world), a malformed/truncated
//!      `--log` file is `parse_log` — both exit `2`;
//!   ③ the `checks`/`queue` wedge verbs are wired and return the honest
//!      NOT-IMPLEMENTED envelope (`wp:"WB2"`), exit `2`.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wb0-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Parse stdout as the canonical error envelope, asserting the one-law shape:
/// nested under `error`, `fix` (never `suggested_fix`) is THE remediation key.
fn assert_canonical_error(stdout: &str, expect_kind: &str) -> Value {
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("error must be JSON on stdout, got {stdout:?}: {e}"));
    assert!(v.get("error").is_some(), "error must be nested: {stdout}");
    assert!(v.get("kind").is_none(), "error must NOT be flat: {stdout}");
    assert_eq!(v["error"]["kind"], expect_kind, "kind: {stdout}");
    assert!(v["error"]["message"].is_string(), "message: {stdout}");
    assert!(
        v["error"]["fix"].is_string(),
        "fix is THE remediation key: {stdout}"
    );
    assert!(
        v["error"].get("suggested_fix").is_none(),
        "never suggested_fix: {stdout}"
    );
    v
}

/// Assert the process exited with the structured user/domain error code (`2`).
fn assert_exit_two(out: &std::process::Output) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "structured user/domain error MUST exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── ① legacy verb SUCCESS is stable JSON on stdout ───────────────────────────

#[test]
fn tournament_success_is_stable_json() {
    let out = Command::new(hugit_bin())
        .args(["tournament", "-n", "2", "--intent", "intent-xyz"])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "tournament within cap exits 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("tournament success must be JSON: {stdout:?}: {e}"));
    assert_eq!(v["intent"], "intent-xyz");
    assert_eq!(v["candidates"], 2);
    assert_eq!(v["fanout"].as_array().unwrap().len(), 2);
}

#[test]
fn export_success_is_stable_json_with_paths_and_digest() {
    let dir = scratch("export-ok");
    let log = dir.join("corpus.json");
    std::fs::write(
        &log,
        serde_json::json!({
            "events": [{
                "kind": "ref.update",
                "principal_chain": ["ci"],
                "payload": serde_json::json!({"ref":"refs/heads/main","target":"deadbeef"})
                    .to_string(),
                "recorded_at": 1u64
            }]
        })
        .to_string(),
    )
    .unwrap();
    let out_dir = dir.join("artifact");
    let out = Command::new(hugit_bin())
        .args([
            "export",
            "--log",
            log.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("hugit runs");
    assert!(
        out.status.success(),
        "export exits 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("export success must be JSON: {stdout:?}: {e}"));
    assert!(v["exported"]["git_dir"].is_string());
    assert!(v["exported"]["envelope_json"].is_string());
    assert!(v["exported"]["schema_version"].is_string());
}

// ── ① legacy verb ERROR is the canonical envelope, exit 2 ────────────────────

#[test]
fn tournament_over_cap_is_canonical_error_exit_two() {
    let out = Command::new(hugit_bin())
        .args(["tournament", "-n", "9999", "--intent", "x"])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_canonical_error(&stdout, "policy_cap_exceeded");
}

#[test]
fn why_unresolved_is_canonical_error_exit_two() {
    let dir = scratch("why-unresolved");
    let log = dir.join("log.json");
    std::fs::write(
        &log,
        serde_json::json!([{
            "record": {
                "seq": 0, "prev_hash": "0".repeat(64), "this_hash": "a".repeat(64),
                "kind": "intent.landed", "principal_chain": ["a@b.c"],
                "payload": serde_json::json!({"path":"src/known.rs"}).to_string(),
                "recorded_at": 1u64
            }
        }])
        .to_string(),
    )
    .unwrap();
    let out = Command::new(hugit_bin())
        .args([
            "why",
            "--log",
            log.to_str().unwrap(),
            "--path",
            "src/absent.rs",
        ])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_canonical_error(&stdout, "unresolved");
}

// ── ② the input-error law: missing FILE vs malformed file ────────────────────

#[test]
fn missing_log_file_is_explicit_log_not_found_never_empty_world() {
    let dir = scratch("why-missing");
    let absent = dir.join("does-not-exist.json");
    let out = Command::new(hugit_bin())
        .args([
            "why",
            "--log",
            absent.to_str().unwrap(),
            "--path",
            "src/x.rs",
        ])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v = assert_canonical_error(&stdout, "log_not_found");
    assert!(
        v["error"]["path"]
            .as_str()
            .unwrap()
            .contains("does-not-exist"),
        "log_not_found names the path: {stdout}"
    );
}

#[test]
fn truncated_log_file_is_parse_log_error() {
    let dir = scratch("why-trunc");
    let log = dir.join("log.json");
    std::fs::write(&log, "[{\"record\": {\"seq\": 0, ").unwrap(); // truncated
    let out = Command::new(hugit_bin())
        .args(["why", "--log", log.to_str().unwrap(), "--path", "src/x.rs"])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_canonical_error(&stdout, "parse_log");
}

#[test]
fn export_missing_log_is_log_not_found() {
    let dir = scratch("export-missing");
    let absent = dir.join("no-corpus.json");
    let out = Command::new(hugit_bin())
        .args([
            "export",
            "--log",
            absent.to_str().unwrap(),
            "--out",
            dir.join("art").to_str().unwrap(),
        ])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    assert_canonical_error(&String::from_utf8_lossy(&out.stdout), "log_not_found");
}

// ── ③ the wedge stubs are wired + honest NOT-IMPLEMENTED ─────────────────────

#[test]
fn checks_show_is_not_implemented_stub() {
    let out = Command::new(hugit_bin())
        .args(["checks", "show", "--log", "/tmp/whatever.json"])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v = assert_canonical_error(&stdout, "not_implemented");
    assert_eq!(v["error"]["wp"], "WB2");
}

#[test]
fn checks_key_is_not_implemented_stub() {
    let out = Command::new(hugit_bin())
        .args(["checks", "key", "--log", "/tmp/whatever.json"])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    assert_eq!(
        assert_canonical_error(&String::from_utf8_lossy(&out.stdout), "not_implemented")["error"]["wp"],
        "WB2"
    );
}

#[test]
fn queue_show_is_not_implemented_stub() {
    let out = Command::new(hugit_bin())
        .args(["queue", "show", "--log", "/tmp/whatever.json"])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    let v = assert_canonical_error(&String::from_utf8_lossy(&out.stdout), "not_implemented");
    assert_eq!(v["error"]["wp"], "WB2");
}
