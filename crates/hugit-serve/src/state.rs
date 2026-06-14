//! Server state + repo→log resolution + the verified-load chokepoint.
//!
//! The `{repo}` slug arrives from the URL, so it is validated as a single safe
//! path segment (no traversal) BEFORE touching the filesystem — a hostile
//! `..%2F..%2Fetc%2Fpasswd` can never escape the log dir. Loading routes through
//! the engine's single verified loader (`hugit_cli::checks::load_event_log` →
//! `rehydrate_and_verify` → `verify_chain`, PS-13) — a tampered chain fails
//! CLOSED as 503, never projected.

use std::path::PathBuf;

use hugit_refstore::EventLog;

use crate::error::EngineErr;

/// Immutable server configuration.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Directory holding one canonical event log per repo: `<log_dir>/<repo>.json`.
    pub log_dir: PathBuf,
    /// The Wave-1 dev Bearer token (the P2-Clerk stub). Fail-closed: required.
    pub dev_token: String,
}

impl AppState {
    /// Build from env: `HUGIT_SERVE_LOG_DIR` + `HUGIT_ENGINE_DEV_TOKEN` (both
    /// required — fail-closed: no token ⇒ refuse to start).
    pub fn from_env() -> Result<Self, String> {
        let log_dir = std::env::var("HUGIT_SERVE_LOG_DIR")
            .map_err(|_| "HUGIT_SERVE_LOG_DIR is not set".to_string())?;
        let dev_token = std::env::var("HUGIT_ENGINE_DEV_TOKEN").map_err(|_| {
            "HUGIT_ENGINE_DEV_TOKEN is not set (fail-closed: refusing to start without an auth token)"
                .to_string()
        })?;
        if dev_token.trim().is_empty() {
            return Err("HUGIT_ENGINE_DEV_TOKEN is empty (fail-closed)".to_string());
        }
        Ok(Self {
            log_dir: PathBuf::from(log_dir),
            dev_token,
        })
    }

    /// Explicit constructor (tests).
    #[must_use]
    pub fn new(log_dir: PathBuf, dev_token: String) -> Self {
        Self { log_dir, dev_token }
    }

    /// The canonical event-log path for `repo` (caller MUST have validated the
    /// slug via [`is_safe_repo_slug`]).
    fn log_path(&self, repo: &str) -> PathBuf {
        self.log_dir.join(format!("{repo}.json"))
    }

    /// Load + chain-verify a repo's event log. A non-existent log (or an unsafe
    /// slug) → 404 (no existence leak). Any parse / tamper / I/O fault → 503
    /// ENGINE_UNAVAILABLE (fail-honest — never a fake-empty VM).
    pub fn load_verified(&self, repo: &str) -> Result<EventLog, EngineErr> {
        if !is_safe_repo_slug(repo) {
            return Err(EngineErr::not_found());
        }
        match hugit_cli::checks::load_event_log(&self.log_path(repo)) {
            Ok(log) => Ok(log),
            Err(e) if e.kind() == "log_not_found" => Err(EngineErr::not_found()),
            Err(e) => Err(EngineErr::unavailable(format!(
                "engine log read/verify failed ({})",
                e.kind()
            ))),
        }
    }
}

/// A repo slug is a single safe path segment: non-empty, ≤100 chars, ASCII
/// alnum + `-_.`, never `.`/`..`/containing `..` or a path separator. Blocks URL
/// path-traversal into arbitrary files.
#[must_use]
pub fn is_safe_repo_slug(repo: &str) -> bool {
    !repo.is_empty()
        && repo.len() <= 100
        && repo != "."
        && repo != ".."
        && !repo.contains("..")
        && !repo.contains('/')
        && !repo.contains('\\')
        && repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_slugs_accept_normal_repos() {
        for ok in ["hugit", "my-repo", "repo_1", "a.b", "HuGR"] {
            assert!(is_safe_repo_slug(ok), "{ok} should be safe");
        }
    }

    #[test]
    fn unsafe_slugs_block_traversal() {
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "a/b",
            "a\\b",
            "a..b",
            "../../etc/passwd",
        ] {
            assert!(!is_safe_repo_slug(bad), "{bad} must be rejected");
        }
    }
}
