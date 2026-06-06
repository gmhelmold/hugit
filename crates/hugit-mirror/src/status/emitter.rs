//! Status emitter — maps each `CheckResult` to a GitHub commit status (item ①).
//!
//! Each `CheckResult` is projected to a `ChecksWriteRequest` keyed on the
//! `memo_key` (used as the check context / name). The actual HTTP write is
//! delegated to `hugit_app::ChecksClient`.

use hugit_contracts::{CheckResult, ChecksWriteRequest};

/// A resolved status payload ready to be sent to the GitHub Checks API.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusPayload {
    /// The `ChecksWriteRequest` derived from a `CheckResult`.
    pub request: ChecksWriteRequest,
}

/// Errors from the status emitter.
#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    /// The Checks API client returned an error.
    #[error("checks API error: {0}")]
    Api(String),

    /// The CheckResult could not be projected (e.g. invalid fields).
    #[error("invalid check result: {0}")]
    InvalidResult(String),
}

/// Maps `CheckResult` values to GitHub commit statuses (item ①).
///
/// The check `memo_key` is used as the GitHub check-run name/context so that
/// each unique check surface appears as a distinct status in the GitHub UI.
pub struct StatusEmitter {
    /// Repository in `owner/repo` format.
    repo: String,
    /// Checks API client (local or production).
    client: hugit_app::ChecksClient,
}

impl StatusEmitter {
    /// Create a new emitter pointing at `repo`, using a local (in-process) client.
    pub fn new_local(repo: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            client: hugit_app::ChecksClient::new_local(),
        }
    }

    /// Create a new emitter pointing at `repo`, using the production client.
    pub fn new_production(repo: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            client: hugit_app::ChecksClient::new_production(),
        }
    }

    /// Project a `CheckResult` to a `StatusPayload`.
    ///
    /// - `exit == 0`  → status `"completed"`, conclusion `"success"`
    /// - `exit != 0`  → status `"completed"`, conclusion `"failure"`
    ///
    /// The `head_sha` is taken from `result.tree_hash` (the commit SHA of the
    /// workspace snapshot; the caller may supply an explicit SHA via `head_sha`
    /// override when available).
    pub fn project(
        &self,
        result: &CheckResult,
        head_sha: Option<&str>,
    ) -> Result<StatusPayload, EmitError> {
        if result.memo_key.is_empty() {
            return Err(EmitError::InvalidResult("memo_key is empty".to_string()));
        }

        let sha = head_sha.unwrap_or(&result.tree_hash);
        if sha.is_empty() {
            return Err(EmitError::InvalidResult(
                "head_sha / tree_hash is empty".to_string(),
            ));
        }

        let (status, conclusion) = if result.exit == 0 {
            ("completed".to_string(), Some("success".to_string()))
        } else {
            ("completed".to_string(), Some("failure".to_string()))
        };

        let summary = format!(
            "hugit check `{}`: exit={} duration={}ms",
            result.memo_key, result.exit, result.duration_ms
        );

        let request = ChecksWriteRequest {
            repo: self.repo.clone(),
            head_sha: sha.to_string(),
            check_name: result.memo_key.clone(),
            status,
            conclusion,
            summary,
            output_ref: result.stdout_ref.clone(),
        };

        Ok(StatusPayload { request })
    }

    /// Emit a `CheckResult` as a GitHub commit status (item ①).
    ///
    /// Projects the result then writes it via the Checks API client.
    /// Returns the written `StatusPayload` on success.
    pub fn emit(
        &self,
        result: &CheckResult,
        head_sha: Option<&str>,
        installation_token: Option<&str>,
    ) -> Result<StatusPayload, EmitError> {
        let payload = self.project(result, head_sha)?;
        self.client
            .write_check_run(&payload.request, installation_token)
            .map_err(|e| EmitError::Api(e.to_string()))?;
        Ok(payload)
    }
}
