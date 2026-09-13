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
//! landed"). This MCP surface has no capture receipt or invocation id, so it
//! can promise dispatch only: `{status:"dispatched"}`. Inspect canonical log
//! separately when landed activity matters.
use std::process::Command;

use serde_json::{Value, json};

use super::{ToolOutcome, opt_str, req_str, resolve_log};

/// The default hugit binary name (resolved on `$PATH`).
const DEFAULT_BIN: &str = "hugit";

/// Resolve the hugit binary: per-call `hugit_bin` arg › `$HUGIT_BIN` › `hugit`.
fn resolve_bin(args: &Value) -> String {
    if let Some(b) = opt_str(args, "hugit_bin") {
        return b.to_string();
    }
    std::env::var("HUGIT_BIN").unwrap_or_else(|_| DEFAULT_BIN.to_string())
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
///   "push_tuples": "<bounded typed push tuples>",
///   "hugit_bin": "<path>"
/// }
/// ```
///
/// `top_level` is REQUIRED. `log` is optional: omitted delegates default
/// selection to the CLI; an explicit path remains supported for legacy and
/// file-seam callers. `kind` is REQUIRED. MCP reports dispatch only; capture
/// landing has no invocation-correlated receipt before WP3.
pub fn run(args: &Value) -> ToolOutcome {
    let kind = match req_str(args, "kind") {
        Ok(k) => k,
        Err(e) => return ToolOutcome::err(e),
    };
    let top_level = match req_str(args, "top_level") {
        Ok(t) => t,
        Err(e) => return ToolOutcome::err(e),
    };
    let log = match resolve_log(args, top_level) {
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
        .current_dir(top_level);
    if log.pass_log {
        cmd.arg("--log").arg(&log.path);
    }
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
    if let Some(v) = opt_str(args, "push_tuples") {
        cmd.arg("--push-tuples").arg(v);
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

    // No receipt or invocation id exists before WP3. A shared-log event cannot
    // be attributed to this invocation, including an identical concurrent one.
    ToolOutcome::Ok(json!({
        "status": "dispatched",
        "kind": kind,
        "log": log.path,
        "log_source": log.source,
        "message": format!(
            "hugit capture {kind} dispatched (silent exit-0 contract). Inspect landed activity with \
             `hugit why --walk --log {} --path <file>` or `hugit watch --class git-activity`.",
            log.path
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    struct RestoreHugitLog(Option<std::ffi::OsString>);

    impl RestoreHugitLog {
        fn clear() -> Self {
            let old = std::env::var_os("HUGIT_LOG");
            unsafe { std::env::remove_var("HUGIT_LOG") };
            Self(old)
        }
    }

    impl Drop for RestoreHugitLog {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => unsafe { std::env::set_var("HUGIT_LOG", value) },
                None => unsafe { std::env::remove_var("HUGIT_LOG") },
            }
        }
    }

    struct RestoreCurrentDir(std::path::PathBuf);

    impl RestoreCurrentDir {
        fn set(path: &std::path::Path) -> Self {
            let old = std::env::current_dir().unwrap();
            std::env::set_current_dir(path).unwrap();
            Self(old)
        }
    }

    impl Drop for RestoreCurrentDir {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).unwrap();
        }
    }

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

    fn write_argv_fake_hugit(dir: &std::path::Path, argv: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("argv-fake-hugit.sh");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'", argv.display()).unwrap();
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

    fn write_posting_fake_hugit(
        dir: &std::path::Path,
        target: &std::path::Path,
    ) -> std::path::PathBuf {
        let path = dir.join("posting-fake-hugit.sh");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "#!/bin/sh\n(sleep 0.05; printf '%s\\n' '[{{\"kind\":\"ref.update\",\"payload\":\"{{\\\"ref\\\":\\\"refs/heads/main\\\",\\\"target\\\":\\\"deadbeef\\\",\\\"branch\\\":\\\"main\\\"}}\"}}]' > '{}') &\nexit 0",
            target.display()
        )
        .unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn missing_kind_is_a_tool_error() {
        assert!(matches!(run(&json!({})), ToolOutcome::Err(_)));
    }

    #[test]
    fn missing_top_level_is_a_tool_error() {
        assert!(matches!(
            run(&json!({"kind": "commit"})),
            ToolOutcome::Err(_)
        ));
    }

    #[test]
    fn nonexistent_binary_is_a_clear_tool_error() {
        let args = json!({
            "kind": "commit", "top_level": "/tmp", "log": "/tmp/log.json",
            "hugit_bin": "/nonexistent/definitely-not-a-binary-xyz"
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
            "kind": "commit", "top_level": dir.to_str().unwrap(), "log": "/tmp/log.json",
            "oid": "abcdef1234", "hugit_bin": bin.to_str().unwrap()
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
    fn cli_default_log_stays_honestly_dispatched() {
        let _env_lock = crate::tools::TEST_HUGIT_LOG_ENV_LOCK.lock().unwrap();
        let _restore_env = RestoreHugitLog::clear();
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let bin = write_fake_hugit(&dir, 0);
        let args = json!({
            "kind": "commit", "top_level": dir.to_str().unwrap(),
            "oid": "abcdef1234", "hugit_bin": bin.to_str().unwrap()
        });
        match run(&args) {
            ToolOutcome::Ok(v) => assert_eq!(v["status"], json!("dispatched")),
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
            "hugit_bin": bin.to_str().unwrap()
        });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn relative_explicit_log_keeps_server_cwd_when_capture_changes_directory() {
        let _cwd_lock = crate::tools::TEST_CWD_LOCK.lock().unwrap();
        let server = tmp_dir();
        let top_level = tmp_dir();
        std::fs::create_dir_all(&server).unwrap();
        std::fs::create_dir_all(&top_level).unwrap();
        let argv = server.join("argv.txt");
        let bin = write_argv_fake_hugit(&server, &argv);
        let _restore_cwd = RestoreCurrentDir::set(&server);
        let args = json!({
            "kind": "commit", "top_level": top_level, "log": "capture-log.json",
            "hugit_bin": bin
        });
        match run(&args) {
            ToolOutcome::Ok(_) => {}
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let argv = std::fs::read_to_string(&argv).unwrap();
        assert!(argv.lines().any(|arg| arg == "--log"));
        let expected = std::fs::canonicalize(&server)
            .unwrap()
            .join("capture-log.json");
        assert!(argv.lines().any(|arg| arg == expected.to_str().unwrap()));
        drop(_restore_cwd);
        let _ = std::fs::remove_dir_all(&server);
        let _ = std::fs::remove_dir_all(&top_level);
    }

    #[cfg(unix)]
    #[test]
    fn relative_environment_log_keeps_server_cwd_when_capture_changes_directory() {
        let _cwd_lock = crate::tools::TEST_CWD_LOCK.lock().unwrap();
        let _env_lock = crate::tools::TEST_HUGIT_LOG_ENV_LOCK.lock().unwrap();
        let _restore_env = RestoreHugitLog::clear();
        let server = tmp_dir();
        let top_level = tmp_dir();
        std::fs::create_dir_all(&server).unwrap();
        std::fs::create_dir_all(&top_level).unwrap();
        let argv = server.join("argv.txt");
        let bin = write_argv_fake_hugit(&server, &argv);
        let _restore_cwd = RestoreCurrentDir::set(&server);
        unsafe { std::env::set_var("HUGIT_LOG", "capture-env-log.json") };
        let args = json!({
            "kind": "commit", "top_level": top_level, "hugit_bin": bin
        });
        match run(&args) {
            ToolOutcome::Ok(_) => {}
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let argv = std::fs::read_to_string(&argv).unwrap();
        let expected = std::fs::canonicalize(&server)
            .unwrap()
            .join("capture-env-log.json");
        assert!(argv.lines().any(|arg| arg == expected.to_str().unwrap()));
        drop(_restore_cwd);
        let _ = std::fs::remove_dir_all(&server);
        let _ = std::fs::remove_dir_all(&top_level);
    }

    #[test]
    fn concurrent_identical_post_dispatch_event_stays_dispatched() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        let capture_bin = write_posting_fake_hugit(&dir, &log);
        let args = json!({
            "kind": "commit", "top_level": dir, "log": log,
            "oid": "deadbeef", "branch": "main", "hugit_bin": capture_bin,
        });
        match run(&args) {
            ToolOutcome::Ok(v) => assert_eq!(v["status"], json!("dispatched")),
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            log.is_file(),
            "fake capture posted matching event after dispatch"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_log_does_not_block_dispatch() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        std::fs::write(&log, b"not JSON").unwrap();
        let bin = write_fake_hugit(&dir, 0);
        let args = json!({
            "kind": "push-attempt", "top_level": dir, "log": log,
            "push_tuples": "refs/heads/main\taaaa\trefs/heads/main\t0000", "hugit_bin": bin,
        });
        match run(&args) {
            ToolOutcome::Ok(v) => assert_eq!(v["status"], json!("dispatched")),
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
