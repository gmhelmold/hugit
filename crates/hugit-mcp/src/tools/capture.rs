//! `capture` — record a git event (commit / checkout / push-attempt / merge).
//!
//! Shells the REAL porcelain verb `hugit capture --kind <k> ...` — the SAME
//! seam the silent git hooks use. The MCP server hosts this as a tool so an LLM
//! agent that did NOT go through a raw git hook path can still record its git
//! activity on the canonical event log (for example: `jj describe` + `jj git
//! export` write refs directly and fire NO post-commit hook — the agent calls
//! this tool instead). This is the materialization of "hugit as the agent layer"
//! (scope decision 2026-09-03, doc #3): the LLM calls the tool IN PLACE OF the
//! raw git action when git's own hooks cannot observe it.
//!
//! ## Honesty contract
//!
//! `hugit capture` is silent by design: it exits 0 ALWAYS (a hook must never
//! block git), and a real failure writes to the hooks log, not stdout. A tool
//! that merely reported the exit code would LIE to the model (exit 0 ≠ "it
//! landed"). So the tool's default shape is `{status:"dispatched"}` + a pointer
//! to verify; with `"verify": true` (or by default for non-hook callers it
//! remains honest) the tool READS the recursive same log to confirm the exact
//! `ref.update` landed and returns its `seq` + `event_hash` — a proof, from the
//! same canonical source, not a second source of truth.
//!
//! `verify` (`true` by default when the caller supplies a `target`/`shas` to
//! match against) turns the dispatch into a confirmed capture. Without it, the
//! tool can only promise the event was dispatched — never that it landed.
use std::process::Command;

use serde_json::{Value, json};

use super::{ToolOutcome, opt_str, req_str};

/// The default hugit binary name (resolved on `$PATH`).
const DEFAULT_BIN: &str = "hugit";

/// Resolve the hugit binary: per-call `hugit_bin` arg › `$HUGIT_BIN` › `hugit`.
fn resolve_bin(args: &Value) -> String {
    if let Some(b) = opt_str(args, "hugit_bin") {
        return b.to_string();
    }
    std::env::var("HUGIT_BIN").unwrap_or_else(|_| DEFAULT_BIN.to_string())
}

/// Read a repeatable `files` array from the tool arguments.
fn opt_files(args: &Value) -> Vec<String> {
    args.get("files")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Args:
/// ```json
/// {
///   "kind": "commit|checkout|push-attempt|merge",
///   "top_level": "<repo top-level dir>",
///   "log": "<canonical log path>",
///   "hook_log": "<optional .hugit/hooks.log>",
///   "oid": "<commit target / checkout to>",
///   "branch": "<branch name>",
///   "from": "<checkout/merge from>",
///   "recorded_at": "<unix seconds>",
///   "refspecs": "<push stdin refspecs>",
///   "shas": "<local shas being pushed>",
///   "files": ["<touched file>", ...],
///   "verify": true,
///   "hugit_bin": "<path>"
/// }
/// ```
///
/// `top_level` + `log` are REQUIRED. `kind` is REQUIRED. `verify` defaults to
/// `true` (the tool must not promise an unconfirmed capture).
pub fn run(args: &Value) -> ToolOutcome {
    let kind = match req_str(args, "kind") {
        Ok(k) => k,
        Err(e) => return ToolOutcome::err(e),
    };
    let top_level = match req_str(args, "top_level") {
        Ok(t) => t,
        Err(e) => return ToolOutcome::err(e),
    };
    let log = match req_str(args, "log") {
        Ok(l) => l,
        Err(e) => return ToolOutcome::err(e),
    };
    let bin = resolve_bin(args);

    let mut cmd = Command::new(&bin);
    cmd.arg("capture")
        .arg("--kind")
        .arg(kind)
        .arg("--top-level")
        .arg(top_level)
        .arg("--log")
        .arg(log);
    if let Some(h) = opt_str(args, "hook_log") {
        cmd.arg("--hook-log").arg(h);
    }
    if let Some(v) = opt_str(args, "oid") {
        cmd.arg("--oid").arg(v);
    }
    if let Some(v) = opt_str(args, "branch") {
        cmd.arg("--branch").arg(v);
    }
    if let Some(v) = opt_str(args, "from") {
        cmd.arg("--from").arg(v);
    }
    if let Some(v) = opt_str(args, "recorded_at") {
        cmd.arg("--recorded-at").arg(v);
    }
    if let Some(v) = opt_str(args, "refspecs") {
        cmd.arg("--refspecs").arg(v);
    }
    if let Some(v) = opt_str(args, "shas") {
        cmd.arg("--shas").arg(v);
    }
    for f in opt_files(args) {
        cmd.arg("--files").arg(f);
    }

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            return ToolOutcome::err(format!(
                "failed to invoke `{bin} capture` (is the hugit binary on PATH, or set HUGIT_BIN \
                 / the `hugit_bin` argument?): {e}"
            ));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code();

    // `capture` is silent-by-contract: exit 0 ALWAYS (hook discipline), even on
    // a real failure which is written to the hooks log instead. So a non-zero
    // exit IS an invocation-level failure (binary missing / unusable).
    if exit_code != Some(0) {
        return ToolOutcome::Err(format!(
            "`{bin} capture` exited {exit_code:?} (non-zero): stderr: {}",
            stderr.trim()
        ));
    }
    let _ = stdout;

    // Default honest response: dispatched, NOT yet confirmed.
    let mut result = json!({
        "status": "dispatched",
        "kind": kind,
        "message": format!(
            "hugit capture {kind} dispatched (silent exit-0 contract). Confirm it landed with \
             `hugit why --walk --log {log} --path <file>` or `hugit watch --class git-activity`."
        ),
    });

    // With verify: read the SAME canonical log and confirm the ref.update
    // matching `target` or `shas` actually landed — return its seq + hash.
    // `verify` defaults to true (honest: never promise an unconfirmed capture
    // unless the caller explicitly opts out of the confirm read).
    let verify = args.get("verify").and_then(Value::as_bool).unwrap_or(true);
    if !verify {
        // verify=false → skip the confirm read; report the dispatch honestly.
        return ToolOutcome::Ok(result);
    }

    let target = opt_str(args, "oid").map(String::from);
    let shas = opt_str(args, "shas").map(String::from);
    match confirm_on_log(log, target.as_deref(), shas.as_deref()) {
        Ok(Some(proof)) => {
            result["status"] = json!("captured");
            result["seq"] = json!(proof.seq);
            result["event_hash"] = json!(proof.event_hash);
            result["message"] = json!(format!(
                "capture confirmed on the canonical log: ref.update seq {} hash {}",
                proof.seq, proof.event_hash
            ));
            ToolOutcome::Ok(result)
        }
        Ok(None) => ToolOutcome::Err(format!(
            "capture dispatched but NOT confirmed on {} (no matching ref.update found). \
             The silent hook may have deduped or the write raced; re-run with a fresh oid.",
            log
        )),
        Err(e) => ToolOutcome::Err(format!("could not verify capture on the log: {e}")),
    }
}

/// A confirmed capture's proof: the `seq` + `event_hash` of the landed record.
struct CaptureProof {
    seq: u64,
    event_hash: String,
}

/// Read `log` (a canonical `[EventRecord, ...]` array) and find the most-recent
/// `ref.update` whose payload `target` == `target` OR whose `shas` string
/// contains `shas`. Returns `None` when no record matches (not-yet-landed, or a
/// dedupe silently skipped a repeat).
fn confirm_on_log(
    log: &str,
    target: Option<&str>,
    shas: Option<&str>,
) -> Result<Option<CaptureProof>, String> {
    let bytes = std::fs::read(log).map_err(|e| format!("read {}: {e}", log))?;
    let records: Vec<Value> = serde_json::from_slice(&bytes)
        .map_err(|e| format!("parse {} (must be a canonical [EventRecord,...]): {e}", log))?;

    for r in records.iter().rev() {
        if r.get("kind").and_then(Value::as_str) != Some("ref.update") {
            continue;
        }
        let payload: Value = r
            .get("payload")
            .and_then(Value::as_str)
            .and_then(|p| serde_json::from_str(p).ok())
            .unwrap_or(Value::Null);
        let matches = match target {
            Some(t) => payload.get("target").and_then(Value::as_str) == Some(t),
            None => false,
        } || match shas {
            Some(s) => payload
                .get("shas")
                .and_then(Value::as_str)
                .map(|sh| sh.split_whitespace().any(|sha| sha == s))
                .unwrap_or(false),
            None => false,
        };
        if !matches {
            continue;
        }
        let seq = r.get("seq").and_then(Value::as_u64);
        let hash = r.get("this_hash").and_then(Value::as_str).map(String::from);
        if let (Some(seq), Some(hash)) = (seq, hash) {
            return Ok(Some(CaptureProof {
                seq,
                event_hash: hash,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A fake `hugit` that prints nothing and exits 0 (the real capture's
    /// silent contract) — proves the shell path end to end.
    fn write_fake_hugit(dir: &std::path::Path, exit: i32) -> std::path::PathBuf {
        let path = dir.join("fake-hugit.sh");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh\nexit {exit}").unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn tmp_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "hugit-mcp-capture-{}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn missing_kind_is_a_tool_error() {
        assert!(matches!(run(&json!({})), ToolOutcome::Err(_)));
    }

    #[test]
    fn missing_top_level_or_log_is_a_tool_error() {
        assert!(matches!(
            run(&json!({"kind": "commit"})),
            ToolOutcome::Err(_)
        ));
    }

    #[test]
    fn nonexistent_binary_is_a_clear_tool_error() {
        let args = json!({
            "kind": "commit", "top_level": "/tmp", "log": "/tmp/log.json",
            "hugit_bin": "/nonexistent/definitely-not-a-binary-xyz", "verify": false
        });
        match run(&args) {
            ToolOutcome::Err(e) => assert!(e.contains("failed to invoke")),
            ToolOutcome::Ok(_) => panic!("expected invoke error"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn dispatches_to_the_binary_and_reports_dispatched() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let bin = write_fake_hugit(&dir, 0);
        let args = json!({
            "kind": "commit", "top_level": "/tmp/repo", "log": "/tmp/log.json",
            "oid": "abcdef1234", "hugit_bin": bin.to_str().unwrap(), "verify": false
        });
        match run(&args) {
            ToolOutcome::Ok(v) => {
                assert_eq!(v["status"], json!("dispatched"));
                assert_eq!(v["kind"], json!("commit"));
            }
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_exit_is_a_tool_error() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let bin = write_fake_hugit(&dir, 127);
        let args = json!({
            "kind": "commit", "top_level": "/tmp/repo", "log": "/tmp/log.json",
            "hugit_bin": bin.to_str().unwrap(), "verify": false
        });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_reads_the_same_log_for_a_matching_target() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        std::fs::write(
            &log,
            r#"[{"seq":0,"prev_hash":"0","this_hash":"abc","kind":"ref.update","principal_chain":["orchestrator:hugit-hook"],"payload":"{\"target\":\"deadbeef\",\"files\":[\"a.rs\"]}","recorded_at":1}]"#,
        )
        .unwrap();
        let proof = confirm_on_log(log.to_str().unwrap(), Some("deadbeef"), None)
            .unwrap()
            .expect("target found");
        assert_eq!(proof.seq, 0);
        assert_eq!(proof.event_hash, "abc");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_reads_the_same_log_for_shas() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        std::fs::write(
            &log,
            r#"[{"seq":0,"prev_hash":"0","this_hash":"def","kind":"ref.update","principal_chain":["orchestrator:hugit-hook"],"payload":"{\"attempt\":true,\"shas\":\"aaaa bbbb\"}","recorded_at":1}]"#,
        )
        .unwrap();
        let proof = confirm_on_log(log.to_str().unwrap(), None, Some("bbbb"))
            .unwrap()
            .expect("sha found");
        assert_eq!(proof.seq, 0);
        assert_eq!(proof.event_hash, "def");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_returns_none_when_no_record_matches() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        std::fs::write(&log, b"[]\n").unwrap();
        assert!(
            confirm_on_log(log.to_str().unwrap(), Some("nope"), None)
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
