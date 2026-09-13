//! The five hugit MCP tools.
//!
//! Each tool is a pure-ish function `(serde_json::Value args) -> ToolOutcome`.
//! The MCP `tools/call` handler ([`crate::server`]) wraps the outcome into the
//! MCP content envelope. The five tools, honest scope each:
//!
//! - [`claim_disjointness`] — wraps the REAL union-engine disjointness primitive
//!   (`hugit_queue::core::AffectedSet::is_disjoint`). HONEST CAVEAT: the affected
//!   set is a path-approximation in v1 (file-path overlap, not the true memoized
//!   check-key blast radius B3 will supply) — stated in every result.
//! - [`land_status`] — shells `hugit queue show --log <path>` and parses the
//!   stdout JSON. No second source of truth.
//! - [`cost_attest`] — `GET /v1/repos/{repo}/insights` with a real Bearer token.
//!   STRUCTURALLY omits any hand-stamp override; per-PR cost is honest-null until
//!   the runner fabric supplies a provider-billed figure.
//! - [`liveness_probe`] — `/readyz` + a bounded Bearer probe with a git UA;
//!   disambiguates 404-denied / 404-missing / 401-gate / 403-bot. REFUSES heavy
//!   reads against the single-thread engine.
//! - [`capture`] — shells the real `hugit capture` seam (the SAME one the
//!   silent hooks use) so an LLM that did NOT go through a git hook path (e.g.
//!   `jj describe` + `jj git export`, which fire no post-commit) can still
//!   record its git activity on the canonical log. Reports dispatch only: no
//!   capture receipt or invocation id exists before WP3.

pub mod capture;
pub mod claim_disjointness;
pub mod cost_attest;
pub mod land_status;
pub mod liveness_probe;

use serde_json::Value;

/// The outcome of a tool invocation.
///
/// `Ok` carries a structured JSON value rendered into the MCP content envelope
/// as pretty JSON text. `Err` carries an `is_error: true` MCP tool error with a
/// human-readable message — distinct from a JSON-RPC protocol error (a tool
/// error is a normal, successful `tools/call` whose CONTENT reports a failure,
/// per the MCP spec, so the model can see and react to it).
pub enum ToolOutcome {
    Ok(Value),
    Err(String),
}

#[cfg(test)]
pub(crate) static TEST_CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) static TEST_HUGIT_LOG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl ToolOutcome {
    /// Convenience: a tool error from any displayable cause.
    pub fn err(msg: impl std::fmt::Display) -> Self {
        ToolOutcome::Err(msg.to_string())
    }
}

/// Read a required string field from the tool arguments object.
pub(crate) fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing or empty required string argument `{key}`"))
}

/// Read an optional string field.
pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Resolve a `land-status` log in the same order as porcelain defaults:
/// explicit argument, environment override, then Git runtime state.
///
/// Runtime and legacy resolution need a repository. The legacy `{log}` MCP
/// schema remains valid without one because its explicit path wins first.
pub(crate) struct ResolvedLog {
    pub(crate) path: String,
    pub(crate) source: &'static str,
    pub(crate) pass_log: bool,
}

pub(crate) fn resolve_land_status_log(args: &Value) -> Result<ResolvedLog, String> {
    if let Some(log) = opt_str(args, "log") {
        return Ok(ResolvedLog {
            path: absolute_log_path(log)?,
            source: "explicit",
            pass_log: true,
        });
    }
    if let Some(log) = std::env::var_os("HUGIT_LOG").filter(|value| !value.is_empty()) {
        return Ok(ResolvedLog {
            path: absolute_log_path(&log)?,
            source: "HUGIT_LOG",
            pass_log: true,
        });
    }
    let top_level = req_str(args, "top_level")?;
    if let Ok(runtime) = git_runtime_log(top_level) {
        // Do not pass an explicit runtime or legacy path here. `queue show`'s
        // default resolver performs the locked legacy-to-runtime migration
        // before reading; an explicit `--log` would bypass that safety gate.
        return Ok(ResolvedLog {
            path: runtime,
            source: "CLI default (Git runtime)",
            pass_log: false,
        });
    }
    let legacy = legacy_log(top_level);
    if std::path::Path::new(&legacy).exists() {
        return Ok(ResolvedLog {
            path: legacy,
            source: "legacy",
            pass_log: true,
        });
    }
    Ok(cli_default_log(top_level))
}

/// Resolve runtime state from an explicitly declared repository.
pub(crate) fn resolve_log(args: &Value, top_level: &str) -> Result<ResolvedLog, String> {
    if let Some(log) = opt_str(args, "log") {
        return Ok(ResolvedLog {
            path: absolute_log_path(log)?,
            source: "explicit",
            pass_log: true,
        });
    }
    if let Some(log) = std::env::var_os("HUGIT_LOG").filter(|value| !value.is_empty()) {
        return Ok(ResolvedLog {
            path: absolute_log_path(&log)?,
            source: "HUGIT_LOG",
            pass_log: true,
        });
    }
    match git_runtime_log(top_level) {
        Ok(runtime) => Ok(ResolvedLog {
            path: runtime,
            // Git runtime defaults must stay owned by the CLI. Passing its
            // canonical path explicitly would skip its legacy migration gate.
            source: "CLI default (Git runtime)",
            pass_log: false,
        }),
        // Match `resolve_log_for_repo`: a non-Git caller can still use an
        // existing repo-local legacy log. Without one, do not invent legacy
        // state: let the CLI select its own default.
        Err(_) => {
            let legacy = legacy_log(top_level);
            if std::path::Path::new(&legacy).exists() {
                Ok(ResolvedLog {
                    path: legacy,
                    source: "legacy",
                    pass_log: true,
                })
            } else {
                Ok(cli_default_log(top_level))
            }
        }
    }
}

/// Preserve relative override paths' historical MCP-process-CWD meaning before
/// child commands switch into a repository with `current_dir`.
fn absolute_log_path(path: impl AsRef<std::ffi::OsStr>) -> Result<String, String> {
    let path = std::path::Path::new(path.as_ref());
    if path.is_absolute() {
        return Ok(path.display().to_string());
    }
    let absolute = std::env::current_dir()
        .map_err(|e| format!("resolve MCP server current directory: {e}"))?
        .join(path);
    // Canonicalize existing relative paths without rejecting a log CLI will create.
    Ok(std::fs::canonicalize(&absolute)
        .unwrap_or(absolute)
        .display()
        .to_string())
}

fn legacy_log(top_level: &str) -> String {
    std::path::Path::new(top_level)
        .join(".hugit/log.json")
        .display()
        .to_string()
}

fn cli_default_log(top_level: &str) -> ResolvedLog {
    ResolvedLog {
        path: std::path::Path::new(top_level)
            .join(".git/hugit/event-log.json")
            .display()
            .to_string(),
        source: "CLI default",
        pass_log: false,
    }
}

fn git_runtime_log(top_level: &str) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(top_level)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .map_err(|e| format!("resolve Git common directory for `{top_level}`: {e}"))?;
    if !output.status.success() {
        return Err(format!("`{top_level}` is not a Git repository"));
    }
    let common = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if common.is_empty() {
        return Err(format!(
            "Git returned no common directory for `{top_level}`"
        ));
    }
    Ok(std::path::Path::new(&common)
        .join("hugit/event-log.json")
        .display()
        .to_string())
}
