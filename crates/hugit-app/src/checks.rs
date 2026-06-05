//! Checks-API write-back client (WP-B1 item ③).
//!
//! Writes a check-run back to GitHub's Checks API on a real PR using the
//! `ChecksWriteRequest` / `ChecksWriteResponse` contract types (frozen, WP-00).
//!
//! The live GitHub API call is infrastructure-gated (requires a valid GitHub
//! App installation token and a real repository). The client interface is
//! fully local-testable via the in-process implementation.

use hugit_contracts::{ChecksWriteRequest, ChecksWriteResponse};

/// Errors from the Checks API client.
#[derive(Debug, thiserror::Error)]
pub enum ChecksClientError {
    /// The request payload could not be serialised.
    #[error("serialise checks request: {0}")]
    Serialise(String),

    /// The API call failed (HTTP or transport error).
    #[error("GitHub Checks API error: {0}")]
    Api(String),

    /// Installation token is absent or revoked — cannot write checks.
    #[error("installation token absent or revoked for installation {installation_id}")]
    TokenRevoked { installation_id: String },
}

/// Checks API client.
///
/// The production implementation issues authenticated HTTP calls to
/// `https://api.github.com/repos/{owner}/{repo}/check-runs` using a
/// per-installation token minted by the App.
///
/// The in-process implementation returns a synthetic response for unit tests.
pub struct ChecksClient {
    /// GitHub API base URL (overridable for tests).
    api_base: String,
    /// In-process mode: bypass real HTTP.
    local_mode: bool,
    /// Running counter for synthetic check_run_id.
    next_id: std::sync::atomic::AtomicU64,
}

impl ChecksClient {
    /// Create a production client against the real GitHub API.
    pub fn new_production() -> Self {
        Self {
            api_base: "https://api.github.com".to_string(),
            local_mode: false,
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Create a local/test client (no real HTTP, returns synthetic responses).
    pub fn new_local() -> Self {
        Self {
            api_base: "https://api.github.com".to_string(),
            local_mode: true,
            next_id: std::sync::atomic::AtomicU64::new(100_001),
        }
    }

    /// Write a check-run to GitHub (item ③).
    ///
    /// In local mode, returns a synthetic `ChecksWriteResponse`.
    /// In production mode, would issue an authenticated HTTPS POST to the
    /// Checks API (infrastructure-gated).
    pub fn write_check_run(
        &self,
        request: &ChecksWriteRequest,
        _installation_token: Option<&str>,
    ) -> Result<ChecksWriteResponse, ChecksClientError> {
        // Validate the request is well-formed.
        if request.repo.is_empty() {
            return Err(ChecksClientError::Api("repo is empty".to_string()));
        }
        if request.head_sha.is_empty() {
            return Err(ChecksClientError::Api("head_sha is empty".to_string()));
        }

        if self.local_mode {
            let id = self
                .next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(ChecksWriteResponse {
                check_run_id: id,
                html_url: format!("{}/repos/{}/check-runs/{}", self.api_base, request.repo, id),
            });
        }

        // Production: would issue HTTP call here (infrastructure-gated).
        Err(ChecksClientError::Api(
            "live GitHub API call requires CF Worker binding (infrastructure-gated)".to_string(),
        ))
    }
}
