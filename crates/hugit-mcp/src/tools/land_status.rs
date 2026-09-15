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

use super::{ToolOutcome, opt_str, resolve_land_status_log};

/// The default hugit binary name (resolved on `$PATH`).
const DEFAULT_BIN: &str = "hugit";

/// Resolve the hugit binary: per-call `hugit_bin` arg › `$HUGIT_BIN` › `hugit`.
fn resolve_bin(args: &Value) -> String {
    if let Some(b) = opt_str(args, "hugit_bin") {
        return b.to_string();
    }
    std::env::var("HUGIT_BIN").unwrap_or_else(|_| DEFAULT_BIN.to_string())
}

/// Args: `{ "top_level"?: "<repo>", "log"?: "<path>", "campaign"?: "<key>",
/// "hugit_bin"?: "<path>" }`.
///
/// `log` is legacy-compatible and needs no `top_level`. Otherwise resolution is
/// explicit `log` > `$HUGIT_LOG` > Git common-dir runtime. Git repositories
/// delegate no `--log` to CLI, so its default resolver safely migrates legacy
/// state before reading. Existing non-Git legacy state remains readable.
pub fn run(args: &Value) -> ToolOutcome {
    let resolved = match resolve_land_status_log(args) {
        Ok(l) => l,
        Err(e) => return ToolOutcome::err(e),
    };
    let top_level = opt_str(args, "top_level");
    let bin = resolve_bin(args);

    let mut cmd = Command::new(&bin);
    cmd.arg("queue").arg("show");
    if resolved.pass_log {
        cmd.arg("--log").arg(&resolved.path);
    }
    if let Some(top_level) = top_level {
        cmd.current_dir(top_level);
    }
    if let Some(campaign) = opt_str(args, "campaign") {
        cmd.arg("--campaign").arg(campaign);
    }

    let mut etxtbsy_retries = 0;
    let output = loop {
        match cmd.output() {
            Ok(output) => break output,
            Err(error) if error.raw_os_error() == Some(26) => {
                etxtbsy_retries += 1;
                if etxtbsy_retries == 10 {
                    return ToolOutcome::err(format!(
                        "failed to invoke `{bin} queue show` after {etxtbsy_retries} retries: {error}"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                return ToolOutcome::err(format!(
                    "failed to invoke `{bin} queue show` (is the hugit binary on PATH, or set \
                     HUGIT_BIN / the `hugit_bin` argument?): {e}"
                ));
            }
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
        "source": if resolved.pass_log {
            format!("{bin} queue show --log {}", resolved.path)
        } else {
            format!("{bin} queue show")
        },
        "log_source": resolved.source,
        "log": resolved.path,
        "exit_code": exit_code,
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::tools::git_runtime_log;
    use std::io::Write;
    struct RestoreHugitLog(Option<std::ffi::OsString>);

    impl RestoreHugitLog {
        fn clear() -> Self {
            // Environment is process-global. Tests taking this guard restore it
            // before another resolver test can observe it.
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

    /// Fake porcelain that records argv before returning a valid projection.
    fn write_argv_fake_hugit(dir: &std::path::Path, argv: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("argv-fake-hugit.sh");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{{\"campaign\":null,\"entries\":[],\"batches\":[]}}'",
            argv.display()
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
    fn missing_log_source_is_a_tool_error() {
        assert!(matches!(run(&json!({})), ToolOutcome::Err(_)));
    }

    #[test]
    fn explicit_log_keeps_legacy_schema_without_top_level() {
        let resolved = resolve_land_status_log(&json!({ "log": "/tmp/legacy.json" })).unwrap();
        assert_eq!(resolved.path, "/tmp/legacy.json");
        assert_eq!(resolved.source, "explicit");
    }

    #[test]
    fn environment_log_beats_runtime_without_top_level() {
        let _lock = crate::tools::TEST_HUGIT_LOG_ENV_LOCK.lock().unwrap();
        let _restore = RestoreHugitLog::clear();
        unsafe { std::env::set_var("HUGIT_LOG", "/tmp/environment.json") };

        let resolved = resolve_land_status_log(&json!({})).unwrap();
        assert_eq!(resolved.path, "/tmp/environment.json");
        assert_eq!(resolved.source, "HUGIT_LOG");
    }

    #[test]
    fn git_repo_delegates_legacy_migration_to_cli_default() {
        let _lock = crate::tools::TEST_HUGIT_LOG_ENV_LOCK.lock().unwrap();
        let _restore = RestoreHugitLog::clear();
        let dir = tmp_dir();
        let output = Command::new("git")
            .arg("init")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(output.status.success());
        let legacy = dir.join(".hugit/log.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(legacy, b"[\"legacy\"]\n").unwrap();

        let resolved =
            resolve_land_status_log(&json!({ "top_level": dir.to_str().unwrap() })).unwrap();
        assert_eq!(
            resolved.path,
            git_runtime_log(dir.to_str().unwrap()).unwrap()
        );
        assert_eq!(resolved.source, "CLI default (Git runtime)");
        assert!(!resolved.pass_log);

        let argv = dir.join("argv.txt");
        let bin = write_argv_fake_hugit(&dir, &argv);
        match run(&json!({ "top_level": dir.to_str().unwrap(), "hugit_bin": bin })) {
            ToolOutcome::Ok(v) => assert_eq!(v["log_source"], json!("CLI default (Git runtime)")),
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let argv = std::fs::read_to_string(&argv).unwrap();
        assert!(argv.lines().any(|arg| arg == "queue"));
        assert!(argv.lines().any(|arg| arg == "show"));
        assert!(
            !argv.lines().any(|arg| arg == "--log"),
            "Git legacy state must reach CLI default migration, not explicit legacy --log: {argv}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_repo_without_logs_delegates_runtime_bootstrap_to_cli_default() {
        let _lock = crate::tools::TEST_HUGIT_LOG_ENV_LOCK.lock().unwrap();
        let _restore = RestoreHugitLog::clear();
        let dir = tmp_dir();
        let output = Command::new("git")
            .arg("init")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(output.status.success());
        let legacy = dir.join(".hugit/log.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"[]\n").unwrap();

        let resolved =
            resolve_land_status_log(&json!({ "top_level": dir.to_str().unwrap() })).unwrap();
        assert_eq!(
            resolved.path,
            git_runtime_log(dir.to_str().unwrap()).unwrap()
        );
        assert_eq!(resolved.source, "CLI default (Git runtime)");
        assert!(!resolved.pass_log);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_git_top_level_uses_legacy_log() {
        let _lock = crate::tools::TEST_HUGIT_LOG_ENV_LOCK.lock().unwrap();
        let _restore = RestoreHugitLog::clear();
        let dir = tmp_dir();
        let legacy = dir.join(".hugit/log.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"[]\n").unwrap();

        let resolved =
            resolve_land_status_log(&json!({ "top_level": dir.to_str().unwrap() })).unwrap();
        assert_eq!(resolved.path, legacy.display().to_string());
        assert_eq!(resolved.source, "legacy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nonexistent_binary_is_a_clear_tool_error() {
        let args = json!({
            "top_level": "/tmp/repo",
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
                assert_eq!(v["log_source"], json!("explicit"));
                assert_eq!(v["log"], json!("/tmp/log.json"));
                assert!(v["source"].as_str().unwrap().contains("queue show"));
            }
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn relative_explicit_log_keeps_server_cwd_when_land_status_changes_directory() {
        let _cwd_lock = crate::tools::TEST_CWD_LOCK.lock().unwrap();
        let server = tmp_dir();
        let top_level = tmp_dir();
        std::fs::create_dir_all(&top_level).unwrap();
        let argv = server.join("argv.txt");
        let bin = write_argv_fake_hugit(&server, &argv);
        let _restore_cwd = RestoreCurrentDir::set(&server);
        match run(&json!({
            "top_level": top_level, "log": "land-status-log.json", "hugit_bin": bin
        })) {
            ToolOutcome::Ok(_) => {}
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let argv = std::fs::read_to_string(&argv).unwrap();
        let expected = std::fs::canonicalize(&server)
            .unwrap()
            .join("land-status-log.json");
        assert!(argv.lines().any(|arg| arg == expected.to_str().unwrap()));
        drop(_restore_cwd);
        let _ = std::fs::remove_dir_all(&server);
        let _ = std::fs::remove_dir_all(&top_level);
    }

    #[cfg(unix)]
    #[test]
    fn relative_environment_log_keeps_server_cwd_when_land_status_changes_directory() {
        let _cwd_lock = crate::tools::TEST_CWD_LOCK.lock().unwrap();
        let _env_lock = crate::tools::TEST_HUGIT_LOG_ENV_LOCK.lock().unwrap();
        let _restore_env = RestoreHugitLog::clear();
        let server = tmp_dir();
        let top_level = tmp_dir();
        std::fs::create_dir_all(&top_level).unwrap();
        let argv = server.join("argv.txt");
        let bin = write_argv_fake_hugit(&server, &argv);
        let _restore_cwd = RestoreCurrentDir::set(&server);
        unsafe { std::env::set_var("HUGIT_LOG", "land-status-env-log.json") };
        match run(&json!({ "top_level": top_level, "hugit_bin": bin })) {
            ToolOutcome::Ok(_) => {}
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
        let argv = std::fs::read_to_string(&argv).unwrap();
        let expected = std::fs::canonicalize(&server)
            .unwrap()
            .join("land-status-env-log.json");
        assert!(argv.lines().any(|arg| arg == expected.to_str().unwrap()));
        drop(_restore_cwd);
        let _ = std::fs::remove_dir_all(&server);
        let _ = std::fs::remove_dir_all(&top_level);
    }

    #[cfg(unix)]
    #[test]
    fn surfaces_a_porcelain_error_envelope_as_a_tool_error() {
        let dir = tmp_dir();
        let body = r#"{"error":{"code":"log_not_found","message":"no such log"}}"#;
        let bin = write_fake_hugit(&dir, body, 2);
        let args = json!({ "top_level": dir.to_str().unwrap(), "log": "/tmp/missing.json", "hugit_bin": bin.to_str().unwrap() });
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
        let args = json!({ "top_level": dir.to_str().unwrap(), "log": "/tmp/log.json", "hugit_bin": bin.to_str().unwrap() });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
