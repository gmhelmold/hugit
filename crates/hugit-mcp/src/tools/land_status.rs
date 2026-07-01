//! `land-status` — the current landing-queue state.
//!
//! Shells the REAL porcelain verb `hugit queue show --log <path>` and parses its
//! stdout JSON. No second source of truth: the same projection over the same
//! canonical event log every hugit porcelain verb shares (entries in queue
//! order, batch composition by campaign, per-batch union verdict + failing pair).
//!
//! The hugit binary is invoked by name `hugit` (resolved on `$PATH`) by default,
//! overridable via the `HUGIT_BIN` env var or the per-call `hugit_bin` argument
//! — so a user can host this MCP server alongside any hugit build.
//!
//! ## Honest semantics passed through verbatim
//!
//! `hugit queue show` already encodes the honesty law: a batch verdict is `null`
//! ("no `verdict.recorded` event covers this batch yet" — never a faked
//! pass/fail). We do not re-interpret it; we surface the parsed JSON plus the
//! invocation provenance so the caller can trust the chain.

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

/// Args: `{ "log": "<path>", "campaign"?: "<key>", "hugit_bin"?: "<path>" }`.
///
/// `log` is REQUIRED and explicit — the MCP server runs in an arbitrary cwd, so
/// we never silently fall back to `.hugit/log.json` (which would project the
/// wrong repo's queue). The caller states the log path it means.
pub fn run(args: &Value) -> ToolOutcome {
    let log = match req_str(args, "log") {
        Ok(l) => l,
        Err(e) => return ToolOutcome::err(e),
    };
    let bin = resolve_bin(args);

    let mut cmd = Command::new(&bin);
    cmd.arg("queue").arg("show").arg("--log").arg(log);
    if let Some(campaign) = opt_str(args, "campaign") {
        cmd.arg("--campaign").arg(campaign);
    }

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            return ToolOutcome::err(format!(
                "failed to invoke `{bin} queue show` (is the hugit binary on PATH, or set \
                 HUGIT_BIN / the `hugit_bin` argument?): {e}"
            ));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // `hugit queue show` prints stable JSON on stdout for BOTH success and its
    // one-error envelope (`{"error":{…}}`, exit 2). Parse stdout first; only if
    // it is not JSON do we treat it as an opaque invocation failure.
    let parsed: Value = match serde_json::from_str(stdout.trim()) {
        Ok(v) => v,
        Err(e) => {
            return ToolOutcome::err(format!(
                "`{bin} queue show` produced non-JSON stdout (exit {:?}): {e}; stderr: {}",
                output.status.code(),
                stderr.trim()
            ));
        }
    };

    // A porcelain `{"error":{…}}` envelope (e.g. log_not_found / parse_log) is a
    // real, structured failure — surface it as a tool error so the model reacts,
    // carrying the canonical error code/message through verbatim.
    if let Some(err) = parsed.get("error") {
        return ToolOutcome::err(format!(
            "hugit queue show reported an error: {}",
            serde_json::to_string(err).unwrap_or_else(|_| err.to_string())
        ));
    }

    let exit_code = output.status.code();
    ToolOutcome::Ok(json!({
        "queue": parsed,
        "source": format!("{bin} queue show --log {log}"),
        "exit_code": exit_code,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A tiny fake `hugit` that echoes a fixed JSON queue projection and exits 0.
    /// Written into a temp dir and invoked via the `hugit_bin` override — proves
    /// the shell + parse path end to end without the real binary.
    fn write_fake_hugit(dir: &std::path::Path, body: &str, exit: i32) -> std::path::PathBuf {
        let path = dir.join("fake-hugit.sh");
        let mut f = std::fs::File::create(&path).unwrap();
        // The script ignores its args and prints the canned body, exits `exit`.
        writeln!(f, "#!/bin/sh\ncat <<'EOF'\n{body}\nEOF\nexit {exit}").unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn tmp_dir() -> std::path::PathBuf {
        // A per-call ATOMIC counter guarantees a unique dir even when two parallel
        // test threads read the same coarse clock nanos (the prior flake: a nanos
        // collision → two tests shared a dir → a write-then-exec race on the same
        // fake-hugit.sh). process::id() + counter is collision-free within a run.
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "hugit-mcp-land-{}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn missing_log_arg_is_a_tool_error() {
        assert!(matches!(run(&json!({})), ToolOutcome::Err(_)));
    }

    #[test]
    fn nonexistent_binary_is_a_clear_tool_error() {
        let args = json!({
            "log": "/tmp/x.json",
            "hugit_bin": "/nonexistent/definitely-not-a-binary-xyz"
        });
        match run(&args) {
            ToolOutcome::Err(e) => assert!(e.contains("failed to invoke")),
            ToolOutcome::Ok(_) => panic!("expected invoke error"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn parses_a_queue_show_json_projection() {
        let dir = tmp_dir();
        let body = r#"{"campaign":null,"entries":[{"pr_id":"PR-1","position":0}],"batches":[]}"#;
        let bin = write_fake_hugit(&dir, body, 0);
        let args = json!({ "log": "/tmp/log.json", "hugit_bin": bin.to_str().unwrap() });
        match run(&args) {
            ToolOutcome::Ok(v) => {
                assert_eq!(v["queue"]["entries"][0]["pr_id"], json!("PR-1"));
                assert_eq!(v["exit_code"], json!(0));
                assert!(v["source"].as_str().unwrap().contains("queue show"));
            }
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn surfaces_a_porcelain_error_envelope_as_a_tool_error() {
        let dir = tmp_dir();
        let body = r#"{"error":{"code":"log_not_found","message":"no such log"}}"#;
        let bin = write_fake_hugit(&dir, body, 2);
        let args = json!({ "log": "/tmp/missing.json", "hugit_bin": bin.to_str().unwrap() });
        match run(&args) {
            ToolOutcome::Err(e) => assert!(e.contains("log_not_found")),
            ToolOutcome::Ok(_) => panic!("expected the error envelope to surface as a tool error"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn non_json_stdout_is_a_tool_error() {
        let dir = tmp_dir();
        let bin = write_fake_hugit(&dir, "not json at all", 0);
        let args = json!({ "log": "/tmp/log.json", "hugit_bin": bin.to_str().unwrap() });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
