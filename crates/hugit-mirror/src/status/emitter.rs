//! Status emitter — maps each `CheckResult` to a GitHub commit status (item ①).
//!
//! Each `CheckResult` is projected to a `ChecksWriteRequest` keyed on the
//! `memo_key` (used as the check context / name). The actual HTTP write is
//! delegated to `hugit_app::ChecksClient`.

use hugit_app::ChecksTransport;
use hugit_contracts::{CheckResult, ChecksWriteRequest};

use crate::outbound::auth::{AppAuth, AppAuthError};

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

    /// Emit through an injected [`ChecksTransport`] (the hermetically-testable
    /// seam). Projects the result then POSTs the check-run through `transport`.
    pub fn emit_with_transport<T: ChecksTransport>(
        &self,
        result: &CheckResult,
        head_sha: Option<&str>,
        installation_token: Option<&str>,
        transport: &T,
    ) -> Result<StatusPayload, EmitError> {
        let payload = self.project(result, head_sha)?;
        self.client
            .write_check_run_with_transport(&payload.request, installation_token, transport)
            .map_err(|e| EmitError::Api(e.to_string()))?;
        Ok(payload)
    }

    /// End-to-end: **mint an App installation token then POST the check-run** —
    /// the "landing rides ON GitHub" path (item ③). Honest by construction:
    ///
    /// - App credentials absent / installation revoked → [`LiveStatusOutcome::NotConfigured`]
    ///   (PARTIAL, never a fabricated post — this is the honest "App not installed
    ///   yet" state the owner's live install closes);
    /// - a mint transport / non-2xx / decode failure, or a Checks-API failure →
    ///   [`LiveStatusOutcome::Failed`] (fail-closed, secret-free reason);
    /// - a real minted token + a 2xx check-run → [`LiveStatusOutcome::Posted`].
    ///
    /// `auth` mints via [`AppAuth`]; `token_transport` and `checks_transport` are
    /// the two network seams (real in prod, fakes in tests). `now_ms` is passed to
    /// the mint for deterministic caching in tests.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_live<TT, CT>(
        &self,
        auth: &AppAuth,
        installation_id: &str,
        result: &CheckResult,
        head_sha: Option<&str>,
        token_transport: &TT,
        checks_transport: &CT,
        now_ms: u64,
    ) -> LiveStatusOutcome
    where
        TT: crate::outbound::auth::TokenTransport,
        CT: ChecksTransport,
    {
        let token = match auth.mint_with_transport(installation_id, token_transport, now_ms) {
            Ok(t) => t,
            // Absent creds or a revoked installation ⇒ the honest NotConfigured
            // door (never fake-green). The App simply is not installed / has been
            // uninstalled — surface that, don't invent a check.
            Err(AppAuthError::CredentialsUnavailable) => {
                return LiveStatusOutcome::NotConfigured {
                    reason: "github-app credentials unavailable (App not installed)".to_string(),
                };
            }
            Err(AppAuthError::InstallationRevoked { installation_id }) => {
                return LiveStatusOutcome::NotConfigured {
                    reason: format!("installation {installation_id} revoked"),
                };
            }
            Err(e) => {
                return LiveStatusOutcome::Failed {
                    reason: format!("token mint failed: {e}"),
                };
            }
        };
        match self.emit_with_transport(result, head_sha, Some(token.expose()), checks_transport) {
            Ok(payload) => LiveStatusOutcome::Posted {
                repo: self.repo.clone(),
                check_name: payload.request.check_name,
            },
            Err(e) => LiveStatusOutcome::Failed {
                reason: e.to_string(),
            },
        }
    }
}

/// The honest outcome of a live "surface the verdict on GitHub" attempt.
///
/// Mirrors [`crate::outbound::LiveLandingOutcome`]'s never-fake discipline: an
/// absent/uninstalled App is [`Self::NotConfigured`] (PARTIAL), a fault is
/// [`Self::Failed`] (fail-closed), and only a real 2xx check-run is
/// [`Self::Posted`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveStatusOutcome {
    /// The verdict was posted as a GitHub check-run.
    Posted {
        /// The repo it was posted to.
        repo: String,
        /// The check name that now appears on the PR.
        check_name: String,
    },
    /// The App is not installed / has been uninstalled → honest PARTIAL, never a
    /// fabricated post. Carries a secret-free reason.
    NotConfigured {
        /// Why the live post could not be attempted.
        reason: String,
    },
    /// A minted token was present but the post failed (transport / non-2xx /
    /// decode) → fail-closed, never marked posted. Carries a secret-free reason.
    Failed {
        /// Why the post failed.
        reason: String,
    },
}

impl LiveStatusOutcome {
    /// Whether the verdict was really posted to GitHub.
    pub fn is_posted(&self) -> bool {
        matches!(self, LiveStatusOutcome::Posted { .. })
    }

    /// Whether this is the honest "App not configured/installed" path.
    pub fn is_not_configured(&self) -> bool {
        matches!(self, LiveStatusOutcome::NotConfigured { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbound::auth::AppAuthError;
    use std::sync::Mutex;

    fn check_result(exit: i32) -> CheckResult {
        CheckResult {
            memo_key: "hugit/land".to_string(),
            tree_hash: "deadbeef".to_string(),
            def_digest: "d".to_string(),
            toolchain_digest: "t".to_string(),
            exit,
            artifacts: vec![],
            stdout_ref: "cas://out".to_string(),
            stderr_ref: "cas://err".to_string(),
            duration_ms: 1234,
            runner_ref: "runner-1".to_string(),
            produced_at: 1,
        }
    }

    /// A fake token transport that hands back a real-looking installation token.
    struct FakeToken(&'static str);
    impl crate::outbound::auth::TokenTransport for FakeToken {
        fn post_access_token(
            &self,
            _url: &str,
            _jwt: &str,
        ) -> Result<(u16, Vec<u8>), AppAuthError> {
            Ok((
                201,
                format!(
                    r#"{{"token":"{}","expires_at":"2999-01-01T00:00:00Z"}}"#,
                    self.0
                )
                .into_bytes(),
            ))
        }
    }

    /// A fake checks transport recording the last post.
    #[derive(Default)]
    struct FakeChecks {
        resp: Mutex<Option<(u16, Vec<u8>)>>,
        posted_token: Mutex<Option<String>>,
    }
    impl hugit_app::ChecksTransport for FakeChecks {
        fn post_json(
            &self,
            _url: &str,
            token: &str,
            _body: &[u8],
        ) -> Result<(u16, Vec<u8>), hugit_app::ChecksClientError> {
            *self.posted_token.lock().unwrap() = Some(token.to_string());
            Ok(self
                .resp
                .lock()
                .unwrap()
                .clone()
                .unwrap_or((201, br#"{"id":7,"html_url":"u"}"#.to_vec())))
        }
    }

    fn creds_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-emit-live-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("private-key.pem"), b"dummy").unwrap();
        std::fs::write(dir.join("app-id"), "123\n").unwrap();
        dir
    }

    #[test]
    fn emit_live_posts_the_verdict_with_the_minted_token() {
        // NB: emit_live goes through mint_with_transport, which uses the exchange
        // core WITHOUT signing (the JWT is only needed for the real transport), so
        // the dummy creds dir is sufficient — no real RSA key.
        let dir = creds_dir("posts");
        let auth = AppAuth::new(&dir);
        // Prime a cached token so the mint fast-path returns it without needing a
        // real RSA key (RS256 signing is covered in auth's own tests). This
        // exercises the real emit_live glue: mint → POST via the fake transport.
        auth.prime_installation_token("77", "ghs_minted");
        let emitter = StatusEmitter::new_production("acme/widgets");
        let checks = FakeChecks::default();
        let outcome = emitter.emit_live(
            &auth,
            "77",
            &check_result(0),
            Some("sha1"),
            &FakeToken("unused"),
            &checks,
            1_000,
        );
        assert!(outcome.is_posted(), "got {outcome:?}");
        // The check-run was posted with the freshly-minted token (never fabricated).
        assert_eq!(
            checks.posted_token.lock().unwrap().as_deref(),
            Some("ghs_minted")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn emit_live_is_not_configured_when_creds_absent() {
        // No creds on disk ⇒ mint returns CredentialsUnavailable ⇒ honest
        // NotConfigured, never a fabricated post.
        let auth = AppAuth::new("/nonexistent/github-app-dev");
        let emitter = StatusEmitter::new_production("acme/widgets");
        let checks = FakeChecks::default();
        let outcome = emitter.emit_live(
            &auth,
            "77",
            &check_result(0),
            Some("sha1"),
            &FakeToken("unused"),
            &checks,
            1_000,
        );
        assert!(outcome.is_not_configured(), "got {outcome:?}");
        assert!(!outcome.is_posted());
        assert!(checks.posted_token.lock().unwrap().is_none());
    }

    #[test]
    fn emit_live_is_not_configured_for_a_revoked_installation() {
        let dir = creds_dir("revoked");
        let led = hugit_app::RevocationLedger::open(std::env::temp_dir().join(format!(
            "hugit-emit-revled-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )))
        .unwrap();
        led.revoke("77").unwrap();
        let auth = AppAuth::new(&dir).with_revocation_ledger(led);
        let emitter = StatusEmitter::new_production("acme/widgets");
        let checks = FakeChecks::default();
        let outcome = emitter.emit_live(
            &auth,
            "77",
            &check_result(0),
            Some("sha1"),
            &FakeToken("unused"),
            &checks,
            1_000,
        );
        assert!(outcome.is_not_configured(), "got {outcome:?}");
        assert!(checks.posted_token.lock().unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn emit_with_transport_projects_failure_conclusion() {
        let emitter = StatusEmitter::new_local("acme/widgets");
        // exit != 0 ⇒ conclusion failure in the projected request.
        let payload = emitter
            .emit_with_transport(
                &check_result(1),
                Some("sha1"),
                Some("t"),
                &FakeChecks::default(),
            )
            .unwrap();
        assert_eq!(payload.request.conclusion.as_deref(), Some("failure"));
    }
}
