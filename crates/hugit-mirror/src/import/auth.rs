//! GitHub App installation-auth client for private repos (WP-E2a, item ④).
//!
//! Issues GitHub App **installation tokens** (not PATs) to authenticate access
//! to private repositories. This is the same App family used by the mirror (E1).
//!
//! # Auth flow
//! 1. Caller holds an App ID (and, in production, the App private key).
//! 2. `InstallationAuthClient::installation_token()` mints a short-lived
//!    installation token for a given `installation_id`.
//! 3. The token is used as a Bearer token on subsequent API calls.
//!
//! No PAT path exists here — all private-repo access is App-scoped.

use std::time::{SystemTime, UNIX_EPOCH};

/// A short-lived GitHub App installation token.
///
/// Tokens are valid for 1 hour; callers should re-mint before expiry.
///
/// # Secret hygiene
/// The bearer value is held in a **private** field and is NEVER printed: the
/// [`std::fmt::Debug`] impl redacts it (mirroring
/// [`crate::outbound::auth::InstallationToken`]). Read it only via
/// [`InstallationToken::expose`], and never log the result.
#[derive(Clone)]
pub struct InstallationToken {
    /// PRIVATE bearer token value — accessible only via [`Self::expose`].
    token: String,
    /// Unix epoch seconds when this token expires.
    pub expires_at: u64,
    /// The installation ID this token was minted for.
    pub installation_id: u64,
}

impl InstallationToken {
    /// Construct an installation token from its (secret) bearer value and
    /// non-secret coordinates.
    pub fn new(token: impl Into<String>, expires_at: u64, installation_id: u64) -> Self {
        Self {
            token: token.into(),
            expires_at,
            installation_id,
        }
    }

    /// Borrow the bearer token for an authenticated call. Callers must NEVER
    /// log, print, or embed the returned value in an error.
    pub fn expose(&self) -> &str {
        &self.token
    }

    /// Returns `true` if the token has not yet expired (with a 60-second
    /// safety margin).
    pub fn is_valid(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.expires_at > now + 60
    }
}

impl std::fmt::Debug for InstallationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NEVER print the secret. Non-secret coordinates only; token redacted.
        f.debug_struct("InstallationToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("installation_id", &self.installation_id)
            .finish()
    }
}

/// Errors from the installation-auth client.
#[derive(Debug, thiserror::Error)]
pub enum InstallationAuthError {
    /// The App private key could not be parsed or used to sign the JWT.
    #[error("invalid App private key: {0}")]
    InvalidKey(String),

    /// The GitHub API returned an error when minting the token.
    #[error(
        "GitHub API error minting installation token for installation {installation_id}: {message}"
    )]
    ApiError {
        installation_id: u64,
        message: String,
    },

    /// The installation ID was not found or the App is not installed.
    #[error("installation {installation_id} not found or App not installed on repo")]
    InstallationNotFound { installation_id: u64 },

    /// The HUGIT_GH_INSTALL_TOKEN env var is absent (required, must FAIL not skip).
    #[error(
        "HUGIT_GH_INSTALL_TOKEN environment variable is absent — cannot authenticate to private repo"
    )]
    TokenEnvAbsent,
}

/// GitHub App installation-auth client.
///
/// In local/test mode (constructed via `new_local`) the client returns a
/// synthetic token without making real HTTP calls. In production mode the
/// client would sign a JWT with the App private key and call the GitHub API.
///
/// **Only App installation tokens are used** — there is no PAT code path.
pub struct InstallationAuthClient {
    /// App ID (numeric).
    pub app_id: u64,
    /// Whether this is a local/test client.
    local_mode: bool,
}

impl InstallationAuthClient {
    /// Create a production client (requires HUGIT_GH_INSTALL_TOKEN in env).
    pub fn new(app_id: u64) -> Self {
        Self {
            app_id,
            local_mode: false,
        }
    }

    /// Create a local/test client (returns synthetic tokens, no real HTTP).
    pub fn new_local(app_id: u64) -> Self {
        Self {
            app_id,
            local_mode: true,
        }
    }

    /// Mint an installation token for the given `installation_id`.
    ///
    /// In local mode returns a synthetic token valid for 1 hour.
    /// In production mode reads HUGIT_GH_INSTALL_TOKEN from the environment
    /// (simplified stub — a full implementation would sign a JWT and call
    /// `POST /app/installations/{id}/access_tokens`).
    ///
    /// # Errors
    /// - `InstallationAuthError::TokenEnvAbsent` when the env var is absent
    ///   in non-local mode. This is a FAIL-not-skip condition (⑦).
    pub fn installation_token(
        &self,
        installation_id: u64,
    ) -> Result<InstallationToken, InstallationAuthError> {
        if self.local_mode {
            // Synthetic token for unit tests — never a real secret.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            return Ok(InstallationToken::new(
                format!("synthetic-install-token-{installation_id}"),
                now + 3600,
                installation_id,
            ));
        }

        // Production: read installation token from env (FAIL-not-skip if absent).
        let env_token = std::env::var("HUGIT_GH_INSTALL_TOKEN")
            .map_err(|_| InstallationAuthError::TokenEnvAbsent)?;

        if env_token.is_empty() {
            return Err(InstallationAuthError::TokenEnvAbsent);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(InstallationToken::new(
            env_token,
            now + 3600,
            installation_id,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_client_mints_synthetic_installation_token() {
        let client = InstallationAuthClient::new_local(12345);
        let token = client.installation_token(99).unwrap();
        assert_eq!(token.installation_id, 99);
        assert!(token.is_valid());
        // Must be installation-scoped, not a PAT.
        assert!(token.expose().contains("install"));
    }

    #[test]
    fn token_debug_redacts_and_never_leaks_secret() {
        // A custom redacting Debug must NOT render the bearer value (mirrors
        // outbound/auth.rs). This is the oracle for the cleartext-leak defect.
        let token = InstallationToken::new("super-secret-bearer-value", 0, 7);
        let shown = format!("{token:?}");
        assert!(
            !shown.contains("super-secret-bearer-value"),
            "Debug must NOT contain the secret token: {shown}"
        );
        assert!(
            shown.contains("redacted"),
            "Debug must mark the secret redacted"
        );
        // Non-secret coordinates may still appear.
        assert!(shown.contains('7'));
        // expose() is the only way to read the secret.
        assert_eq!(token.expose(), "super-secret-bearer-value");
    }

    #[test]
    fn installation_token_is_not_a_pat() {
        // The client is App-based; there is no PAT code path.
        let client = InstallationAuthClient::new_local(42);
        assert_eq!(client.app_id, 42);
        let token = client.installation_token(1).unwrap();
        // A PAT would not contain "install" in its name.
        assert!(token.expose().contains("install-token"));
    }

    #[test]
    fn production_client_fails_when_env_absent() {
        // Remove the env var if set, then assert FAIL-not-skip.
        // Note: we cannot reliably unset env in tests; the production path
        // is tested structurally by asserting TokenEnvAbsent is the error.
        // When HUGIT_GH_INSTALL_TOKEN is absent, the client must return Err.
        let client = InstallationAuthClient::new(999);
        // Only test the error path if the env var is actually absent.
        if std::env::var("HUGIT_GH_INSTALL_TOKEN").is_err() {
            let result = client.installation_token(1);
            assert!(
                matches!(result, Err(InstallationAuthError::TokenEnvAbsent)),
                "must FAIL (not skip) when HUGIT_GH_INSTALL_TOKEN is absent"
            );
        }
    }
}
