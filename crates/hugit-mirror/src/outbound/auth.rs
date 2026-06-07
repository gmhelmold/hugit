//! GitHub App authentication for the outbound push channel (WP-E1a item ①).
//!
//! Auth is a **GitHub App installation token** — never a PAT, never OAuth. A
//! per-push installation token is minted from the App's installation; rotation
//! is handled by the App layer. This module models that surface for the writer
//! and provides the live-credential discovery the live lane uses.
//!
//! # Secret hygiene
//!
//! The App private key / installation credentials live ONLY on disk under
//! `~/.hugit/secrets/github-app-dev/`. They are read via [`std::fs`] and are
//! NEVER printed, logged, or embedded in any error message. [`AppAuth`] holds
//! only non-secret coordinates (installation id, app id length-checks) plus an
//! opaque token handle; the secret bytes never leave their read scope.

use std::path::{Path, PathBuf};

/// An opaque installation token handle.
///
/// The inner string is the minted installation token. It is deliberately NOT
/// `Debug`-printed in full and never rendered into errors — `Debug` redacts.
#[derive(Clone)]
pub struct InstallationToken(String);

impl InstallationToken {
    /// Wrap a minted token.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// Borrow the token for an authenticated call. Callers must never log it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether a (non-empty) token is held.
    pub fn is_present(&self) -> bool {
        !self.0.is_empty()
    }
}

impl std::fmt::Debug for InstallationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NEVER print the secret. Redact to a fixed marker.
        f.write_str("InstallationToken(<redacted>)")
    }
}

/// Errors from GitHub App auth discovery / token minting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppAuthError {
    /// The App credential directory is not present / not readable. The live
    /// lane treats this as "installation unavailable" → PARTIAL, never fake.
    #[error("github-app credentials unavailable")]
    CredentialsUnavailable,

    /// The configured installation does not cover the requested repository.
    /// The live lane treats this as PARTIAL — never a faked success.
    #[error("installation does not cover repo {repo}")]
    RepoNotCovered {
        /// The repo (owner/name) the installation lacks.
        repo: String,
    },
}

/// GitHub App auth client — mints per-push installation tokens.
///
/// Holds only non-secret coordinates. The private key is read from disk on
/// demand inside [`AppAuth::mint_installation_token`] and dropped immediately;
/// it is never retained in the struct nor surfaced.
#[derive(Debug, Clone)]
pub struct AppAuth {
    secret_dir: PathBuf,
}

impl AppAuth {
    /// Construct from the App credential directory (default:
    /// `~/.hugit/secrets/github-app-dev`).
    pub fn new(secret_dir: impl Into<PathBuf>) -> Self {
        Self {
            secret_dir: secret_dir.into(),
        }
    }

    /// The default dev credential directory under the user's home.
    pub fn default_dev_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|home| {
            Path::new(&home)
                .join(".hugit")
                .join("secrets")
                .join("github-app-dev")
        })
    }

    /// Whether the App credential material is present on disk (private key +
    /// app-id). Used by the live lane to decide live-vs-PARTIAL; reads metadata
    /// only, never the key bytes into anything observable.
    pub fn credentials_present(&self) -> bool {
        self.secret_dir.join("private-key.pem").is_file()
            && self.secret_dir.join("app-id").is_file()
    }

    /// Mint a per-push installation token (item ①: App token, not PAT).
    ///
    /// Reads the App private key + app-id from disk via [`std::fs`] (NEVER
    /// printed), then would exchange them for an installation token against the
    /// GitHub App API. Token exchange requires network + a covering
    /// installation; when those are unavailable the live lane reports PARTIAL.
    ///
    /// This v0 reads the credential material to prove the fail-CLOSED secret
    /// path, but does not perform the live HTTPS JWT→token exchange (no
    /// network in the gate lane), so it returns `CredentialsUnavailable` when
    /// material is absent and otherwise an `app_token`-shaped handle scoped to
    /// the local proof. Live exchange is wired for the dogfood soak.
    pub fn mint_installation_token(&self) -> Result<InstallationToken, AppAuthError> {
        if !self.credentials_present() {
            return Err(AppAuthError::CredentialsUnavailable);
        }
        // Read the private key + app-id via std::fs to exercise the real secret
        // path. The bytes are used only to gate presence and are dropped here;
        // they are never returned, logged, or formatted into any value.
        let key = std::fs::read(self.secret_dir.join("private-key.pem"))
            .map_err(|_| AppAuthError::CredentialsUnavailable)?;
        let app_id = std::fs::read(self.secret_dir.join("app-id"))
            .map_err(|_| AppAuthError::CredentialsUnavailable)?;
        if key.is_empty() || app_id.is_empty() {
            return Err(AppAuthError::CredentialsUnavailable);
        }
        // A non-secret, opaque local handle. The live installation-token
        // exchange (JWT sign → POST /app/installations/{id}/access_tokens)
        // happens in the soak lane; never fabricate a usable token here.
        Ok(InstallationToken::new(format!(
            "app-installation-token:local-proof:{}",
            app_id.len()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_redacts_in_debug_and_never_leaks() {
        let t = InstallationToken::new("super-secret-value");
        let shown = format!("{t:?}");
        assert!(!shown.contains("super-secret-value"));
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn missing_credentials_is_unavailable_not_panic() {
        let auth = AppAuth::new(PathBuf::from("/nonexistent/github-app-dev"));
        assert!(!auth.credentials_present());
        assert_eq!(
            auth.mint_installation_token().unwrap_err(),
            AppAuthError::CredentialsUnavailable
        );
    }
}
