//! GitHub App authentication for the live merge lane.
//!
//! To drive the merge API the queue authenticates as the B1 GitHub App: mint a
//! short-lived RS256 App JWT from the App id + private key, call
//! `GET /app/installations` to find the installation covering the target repo,
//! then exchange it for an installation token (write-only secret model —
//! credentials NEVER logged).
//!
//! This module owns the *pure* parts: the JWT claim set (with GitHub's clock
//! constraints) and the installation-list selection. The RS256 signature and
//! the HTTPS calls are the impure seam, supplied by the live acceptance test
//! (the actual key material is read only there, from disk, via `std::fs`, and
//! never printed). Keeping signing/transport out of the production crate keeps
//! the library lean and auditable while still making the live call testable.

/// How far `iat` is backdated, in seconds, to absorb GitHub clock skew (≤60s).
const IAT_BACKDATE_SECS: i64 = 60;

/// Token lifetime in seconds: 9 minutes, inside GitHub's 10-minute ceiling.
const TOKEN_LIFETIME_SECS: i64 = 9 * 60;

/// GitHub's hard maximum for `exp - iat`, in seconds (10 minutes).
const GITHUB_MAX_JWT_WINDOW_SECS: i64 = 600;

/// The registered claims of a GitHub App JWT.
///
/// GitHub requires: `iat` backdated by ≤60s to tolerate clock skew, `exp` no
/// more than 10 minutes out, and `iss` = the App id. We mint a 9-minute token
/// (inside the 10-minute ceiling) backdated 60s.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppJwtClaims {
    /// Issued-at, unix seconds (backdated 60s for clock skew).
    pub iat: i64,
    /// Expiry, unix seconds (≤ 10 minutes after `iat`).
    pub exp: i64,
    /// Issuer — the GitHub App id (as a string per GitHub's docs).
    pub iss: String,
}

impl AppJwtClaims {
    /// Build the claim set for App id `app_id` given the current unix time.
    /// Backdates `iat` by [`IAT_BACKDATE_SECS`] and sets `exp`
    /// [`TOKEN_LIFETIME_SECS`] out — both inside GitHub's accepted window.
    pub fn mint(app_id: &str, now_unix: i64) -> Self {
        Self {
            iat: now_unix - IAT_BACKDATE_SECS,
            exp: now_unix + TOKEN_LIFETIME_SECS,
            iss: app_id.to_string(),
        }
    }

    /// Validate the claim window against GitHub's rules: `iat` not in the
    /// future relative to `now`, and `exp - iat` ≤ [`GITHUB_MAX_JWT_WINDOW_SECS`].
    pub fn is_within_github_window(&self, now_unix: i64) -> bool {
        self.iat <= now_unix
            && (self.exp - self.iat) <= GITHUB_MAX_JWT_WINDOW_SECS
            && self.exp > self.iat
    }
}

/// The App credentials, loaded from the on-disk secret store. The private key
/// PEM is held only long enough to sign; it is never serialised or logged.
pub struct AppCredentials {
    /// The GitHub App id.
    pub app_id: String,
    /// The RSA private key in PEM form. Treated as a write-only secret.
    pub private_key_pem: String,
}

impl std::fmt::Debug for AppCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the key material.
        f.debug_struct("AppCredentials")
            .field("app_id", &self.app_id)
            .field("private_key_pem", &"<redacted>")
            .finish()
    }
}

/// Errors minting or using the App JWT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtError {
    /// The private key could not be parsed / the signature could not be made.
    Signing(String),
    /// The HTTP call to GitHub failed.
    Transport(String),
    /// GitHub returned a non-success status.
    Status(u16, String),
}

/// The impure seam: signing the JWT and calling GitHub's App endpoints.
///
/// Implemented by the live acceptance test against real GitHub; the production
/// wiring (B1's installation-token client) implements the same shape. The trait
/// returns parsed installation ids so the *selection* logic stays pure here.
pub trait AppJwt {
    /// Sign `claims` with the App private key, returning the compact RS256 JWT.
    fn sign(&self, claims: &AppJwtClaims) -> Result<String, JwtError>;

    /// `GET /app/installations` with the App JWT, returning the parsed
    /// installations (their numeric ids and the account login they cover).
    fn list_installations(&self, jwt: &str) -> Result<Vec<Installation>, JwtError>;
}

/// One GitHub App installation, projected to what the selector needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    /// Numeric installation id (used to mint installation tokens).
    pub id: u64,
    /// The account login (org/user) the installation is on.
    pub account_login: String,
    /// Whether the installation is scoped to all repos or selected repos.
    pub repository_selection: String,
}

/// Pure selection: find the installation covering `owner` (the account the
/// target repo lives under). Returns `None` when the App is not installed on
/// that account — the caller then reports the live item PARTIAL / blocked on an
/// owner install-click, NEVER faking a merge.
pub fn select_installation<'a>(
    installations: &'a [Installation],
    owner: &str,
) -> Option<&'a Installation> {
    installations
        .iter()
        .find(|i| i.account_login.eq_ignore_ascii_case(owner))
}

/// Orchestrate the live lookup: mint claims, sign, list installations, select
/// the one for `owner`. Pure control-flow over the impure [`AppJwt`] seam.
pub struct InstallationLookup;

impl InstallationLookup {
    /// Resolve the installation id for `owner`, or `None` if not installed.
    pub fn resolve<J: AppJwt>(
        signer: &J,
        creds: &AppCredentials,
        owner: &str,
        now_unix: i64,
    ) -> Result<Option<Installation>, JwtError> {
        let claims = AppJwtClaims::mint(&creds.app_id, now_unix);
        let jwt = signer.sign(&claims)?;
        let installs = signer.list_installations(&jwt)?;
        Ok(select_installation(&installs, owner).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_are_within_github_window() {
        let now = 1_700_000_000;
        let c = AppJwtClaims::mint("12345", now);
        assert_eq!(c.iss, "12345");
        assert!(c.iat < now, "iat is backdated");
        assert!(c.is_within_github_window(now));
        assert!((c.exp - c.iat) <= GITHUB_MAX_JWT_WINDOW_SECS);
    }

    #[test]
    fn over_long_window_is_rejected() {
        let c = AppJwtClaims {
            iat: 0,
            exp: GITHUB_MAX_JWT_WINDOW_SECS + 100, // 100s over the ceiling
            iss: "1".to_string(),
        };
        assert!(!c.is_within_github_window(0));
    }

    #[test]
    fn select_installation_matches_owner_case_insensitively() {
        let installs = vec![
            Installation {
                id: 1,
                account_login: "other-org".to_string(),
                repository_selection: "all".to_string(),
            },
            Installation {
                id: 42,
                account_login: "HumanGR-Labs".to_string(),
                repository_selection: "selected".to_string(),
            },
        ];
        let found = select_installation(&installs, "humangr-labs").expect("found");
        assert_eq!(found.id, 42);
        assert!(select_installation(&installs, "nobody").is_none());
    }

    #[test]
    fn credentials_debug_redacts_key() {
        let creds = AppCredentials {
            app_id: "999".to_string(),
            private_key_pem: "-----BEGIN PRIVATE KEY-----secret".to_string(),
        };
        let dbg = format!("{creds:?}");
        assert!(dbg.contains("999"));
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("secret"));
    }
}
