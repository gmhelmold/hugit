//! GitHub App authentication for the outbound push channel (WP-E1a item ① / WP-2).
//!
//! Auth is a **GitHub App installation token** — never a PAT, never OAuth. A
//! per-push installation token is minted from the App's installation: sign a
//! short-lived App JWT (RS256) from the on-disk private key + app-id, then
//! exchange it at `POST /app/installations/{id}/access_tokens` for the scoped
//! installation token. Rotation is handled by refreshing the cached token before
//! it expires.
//!
//! # Secret hygiene
//!
//! The App private key / installation credentials live ONLY on disk under
//! `~/.hugit/secrets/github-app-dev/`. They are read via [`std::fs`] and are
//! NEVER printed, logged, or embedded in any error message. [`AppAuth`] holds
//! only non-secret coordinates (installation id, app id length-checks) plus an
//! opaque, redacting token cache; the secret bytes never leave their read scope,
//! and the minted token only ever leaves via [`InstallationToken::expose`].
//!
//! # Live-vs-fail-closed
//!
//! The live HTTPS exchange runs ONLY behind [`UreqTokenTransport`] (the single
//! network seam, exactly like `lease_client.rs`). Everything around it — JWT
//! signing, cache/refresh, response decode, error mapping — is proven
//! hermetically against a [`TokenTransport`] fake with ZERO network. On ANY error
//! (missing material, transport, non-2xx, undecodable body) minting FAILS CLOSED
//! and NEVER fabricates a usable token.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

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
    /// The App credential directory / material is not present / not readable. The
    /// live lane treats this as "installation unavailable" → PARTIAL, never fake.
    #[error("github-app credentials unavailable")]
    CredentialsUnavailable,

    /// The configured installation does not cover the requested repository.
    /// The live lane treats this as PARTIAL — never a faked success.
    #[error("installation does not cover repo {repo}")]
    RepoNotCovered {
        /// The repo (owner/name) the installation lacks.
        repo: String,
    },

    /// The App JWT could not be signed (a malformed/non-RSA private key). Carries
    /// a short, SECRET-FREE reason (never the key bytes). Fail-closed → PARTIAL.
    #[error("github-app JWT signing failed: {0}")]
    JwtSigning(String),

    /// The live installation-token exchange failed: a transport/TLS/I-O fault, a
    /// non-2xx HTTP status, or an undecodable success body. Carries the HTTP status
    /// when the server answered (never a credential / token value). Fail-closed →
    /// the live lane reports PARTIAL, never a fabricated token.
    #[error("github-app installation-token exchange failed (status {status:?})")]
    ExchangeFailed {
        /// The HTTP status the server returned, when it answered; `None` for a
        /// transport-level fault (no response).
        status: Option<u16>,
    },

    /// The installation has been **durably revoked** (an `installation.deleted`
    /// webhook was processed; the on-disk revocation ledger records it). Minting
    /// a token for a revoked installation is refused fail-closed — the mint is
    /// NEVER attempted, so a revoked App can never surface a check-run/push again
    /// even across a restart. Carries the (non-secret) installation id.
    #[error("installation {installation_id} is revoked; token mint refused")]
    InstallationRevoked {
        /// The revoked installation id.
        installation_id: String,
    },
}

/// GitHub App auth client — mints per-push installation tokens.
///
/// Holds only non-secret coordinates plus a redacting token cache. The private
/// key is read from disk on demand inside [`AppAuth::mint_installation_token`]
/// and dropped immediately; it is never retained in the struct nor surfaced.
#[derive(Debug, Clone)]
pub struct AppAuth {
    secret_dir: PathBuf,
    /// Redacting installation-token cache (shared across clones so a `Clone`d
    /// `AppAuth` reuses the same live token). Interior-mutable so `mint` can be
    /// called through a shared `&self`.
    cache: Arc<Mutex<Option<CachedToken>>>,
    /// Durable revocation ledger (WP-B1 item ⑤). When present, a mint for a
    /// revoked installation is refused fail-closed BEFORE any disk/sign/network
    /// work. Shared with the webhook processor so an `installation.deleted`
    /// immediately (and durably) disarms future mints.
    revocations: Option<hugit_app::RevocationLedger>,
}

/// A cached installation token + the epoch-ms at which it should be considered
/// stale (the GitHub `expires_at`, minus a refresh skew). The `installation_id`
/// is part of the key so a cache built for one installation is never handed to a
/// different one.
#[derive(Clone)]
struct CachedToken {
    installation_id: String,
    token: InstallationToken,
    /// Epoch-ms after which the token must be re-minted (expiry − refresh skew).
    refresh_after_ms: u64,
}

impl std::fmt::Debug for CachedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedToken")
            .field("installation_id", &self.installation_id)
            .field("token", &self.token) // redacts
            .field("refresh_after_ms", &self.refresh_after_ms)
            .finish()
    }
}

/// Refresh the token this many milliseconds BEFORE its stated expiry so an
/// in-flight push never races the boundary (GitHub tokens live ~1h).
const REFRESH_SKEW_MS: u64 = 5 * 60 * 1000;

/// Fallback token lifetime when the server's `expires_at` cannot be parsed: a
/// conservative 55 min from mint (GitHub's real TTL is 60 min).
const FALLBACK_TTL_MS: u64 = 55 * 60 * 1000;

impl AppAuth {
    /// Construct from the App credential directory (default:
    /// `~/.hugit/secrets/github-app-dev`).
    pub fn new(secret_dir: impl Into<PathBuf>) -> Self {
        Self {
            secret_dir: secret_dir.into(),
            cache: Arc::new(Mutex::new(None)),
            revocations: None,
        }
    }

    /// Attach a durable revocation ledger so a mint for a revoked installation is
    /// refused fail-closed (WP-B1 item ⑤). Share the SAME ledger the webhook
    /// processor uses so an `installation.deleted` disarms mints immediately and
    /// durably (survives a restart).
    pub fn with_revocation_ledger(mut self, ledger: hugit_app::RevocationLedger) -> Self {
        self.revocations = Some(ledger);
        self
    }

    /// Resolve the App credential directory.
    ///
    /// Resolution order (mirrors the `cas.rs` PAT-file pattern):
    /// 1. `HUGIT_GITHUB_APP_SECRET_DIR` env var — explicit override; useful for
    ///    non-default layouts (staging, CI credential injection, etc.).
    /// 2. `~/.hugit/secrets/github-app-dev` — the conventional dev default,
    ///    assembled from `$HOME` (never a hardcoded literal path).
    ///
    /// Returns `None` only when no override is set AND `$HOME` is absent.
    pub fn resolve_secret_dir() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("HUGIT_GITHUB_APP_SECRET_DIR") {
            let p = p.trim().to_string();
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
        Self::default_dev_dir()
    }

    /// The default dev credential directory under the user's home
    /// (`~/.hugit/secrets/github-app-dev`). Prefer [`AppAuth::resolve_secret_dir`]
    /// in production code so that `HUGIT_GITHUB_APP_SECRET_DIR` is honoured.
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

    /// Resolve the installation id: the `installation-id` file in the secret dir
    /// (preferred; trailing whitespace trimmed) or the `HUGIT_GITHUB_INSTALLATION_ID`
    /// env var. Returns `None` (→ `CredentialsUnavailable` at the call site) when
    /// neither is present/non-empty. The id is a NON-secret coordinate.
    fn resolve_installation_id(&self) -> Option<String> {
        if let Ok(s) = std::fs::read_to_string(self.secret_dir.join("installation-id")) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        std::env::var("HUGIT_GITHUB_INSTALLATION_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Mint a per-push installation token (item ①: App token, not PAT).
    ///
    /// Reads the App private key + app-id from disk via [`std::fs`] (NEVER
    /// printed), signs a short-lived App JWT (RS256), and exchanges it at
    /// `POST /app/installations/{id}/access_tokens` over the real
    /// [`UreqTokenTransport`]. The token is cached with its expiry and refreshed
    /// before it goes stale (see [`AppAuth::mint_with_transport`]).
    ///
    /// FAIL-CLOSED at every step: missing material → [`AppAuthError::CredentialsUnavailable`];
    /// a signing/transport/non-2xx/decode failure surfaces the typed error and
    /// NEVER a fabricated token. The offline gate lane has no network + no real
    /// creds, so it short-circuits on `CredentialsUnavailable` before any socket.
    pub fn mint_installation_token(&self) -> Result<InstallationToken, AppAuthError> {
        // Presence gate FIRST — no network is ever attempted without material.
        if !self.credentials_present() {
            return Err(AppAuthError::CredentialsUnavailable);
        }
        let installation_id = self
            .resolve_installation_id()
            .ok_or(AppAuthError::CredentialsUnavailable)?;
        self.mint_with_transport(&installation_id, &UreqTokenTransport, now_ms())
    }

    /// The transport-injectable core of [`AppAuth::mint_installation_token`] — the
    /// unit-testable seam (a fake [`TokenTransport`] proves the whole path with
    /// ZERO network). Cache fast-path, then reads the on-disk material, signs the
    /// App JWT, and delegates the exchange + caching to
    /// [`AppAuth::exchange_and_cache`] (deterministic against the injected `now_ms`).
    ///
    /// Cache hit: a cached token for the SAME installation whose `refresh_after_ms`
    /// is still in the future is returned WITHOUT reading disk, signing, or a
    /// network call. Otherwise a fresh token is minted, cached, and returned.
    pub fn mint_with_transport<T: TokenTransport>(
        &self,
        installation_id: &str,
        transport: &T,
        now_ms: u64,
    ) -> Result<InstallationToken, AppAuthError> {
        // ── Revocation gate (WP-B1 ⑤): a revoked installation NEVER mints. ──
        // Checked FIRST — before the cache, disk, signing, or any network — so a
        // revoked App cannot even serve a still-cached token. Durable: the ledger
        // reads its on-disk tombstones, so this holds across a restart.
        if let Some(ledger) = &self.revocations
            && ledger.is_revoked(installation_id)
        {
            return Err(AppAuthError::InstallationRevoked {
                installation_id: installation_id.to_string(),
            });
        }

        // ── Cache fast-path: reuse a still-fresh token (no disk / sign / net). ──
        if let Some(hit) = self.cached_if_fresh(installation_id, now_ms) {
            return Ok(hit);
        }

        // ── Read the secret material (NEVER logged / returned / formatted). ──
        if !self.credentials_present() {
            return Err(AppAuthError::CredentialsUnavailable);
        }
        let key = std::fs::read(self.secret_dir.join("private-key.pem"))
            .map_err(|_| AppAuthError::CredentialsUnavailable)?;
        let app_id_raw = std::fs::read_to_string(self.secret_dir.join("app-id"))
            .map_err(|_| AppAuthError::CredentialsUnavailable)?;
        let app_id = app_id_raw.trim();
        if key.is_empty() || app_id.is_empty() {
            return Err(AppAuthError::CredentialsUnavailable);
        }

        // ── Sign the short-lived App JWT (RS256). ──
        let now_secs = now_ms / 1000;
        let jwt = sign_app_jwt(&key, app_id, now_secs)?;

        self.exchange_and_cache(installation_id, &jwt, transport, now_ms)
    }

    /// **Test-only** cache primer: seed a never-stale installation token so the
    /// mint fast-path returns WITHOUT reading the on-disk key or signing a JWT.
    ///
    /// This lets the `mint`/`emit_live` happy-path be proven hermetically without
    /// a real RSA private key (RS256 signing itself is covered by
    /// `sign_app_jwt_rejects_a_bad_key`). The revocation gate runs *before* the
    /// cache fast-path, so a primed token is still refused for a revoked
    /// installation — that ordering is exactly what the gate test asserts.
    #[cfg(test)]
    pub(crate) fn prime_installation_token(&self, installation_id: &str, token: &str) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(CachedToken {
            installation_id: installation_id.to_string(),
            token: InstallationToken::new(token),
            refresh_after_ms: u64::MAX,
        });
    }

    /// Return a still-fresh cached token for `installation_id` (or `None`). Split
    /// out so both [`AppAuth::mint_with_transport`] and [`AppAuth::exchange_and_cache`]
    /// share one cache-freshness predicate (no double-lock subtlety).
    fn cached_if_fresh(&self, installation_id: &str, now_ms: u64) -> Option<InstallationToken> {
        let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().and_then(|cached| {
            if cached.installation_id == installation_id && now_ms < cached.refresh_after_ms {
                Some(cached.token.clone())
            } else {
                None
            }
        })
    }

    /// The exchange + cache-store core, SEPARATED from JWT signing so it is proven
    /// hermetically WITHOUT any private-key material (the `app_jwt` is passed in;
    /// tests supply a dummy). Cache-checks first (no network on a hit), otherwise
    /// `POST`s the App JWT to the installation `access_tokens` endpoint over the
    /// (fake, in tests) transport, decodes the token + `expires_at`, computes the
    /// refresh-before-expiry deadline, caches per-installation, and returns.
    /// FAIL-CLOSED: 404 → [`AppAuthError::RepoNotCovered`]; any other non-2xx,
    /// transport fault, empty/undecodable body → [`AppAuthError::ExchangeFailed`];
    /// never a fabricated token.
    fn exchange_and_cache<T: TokenTransport>(
        &self,
        installation_id: &str,
        app_jwt: &str,
        transport: &T,
        now_ms: u64,
    ) -> Result<InstallationToken, AppAuthError> {
        if let Some(hit) = self.cached_if_fresh(installation_id, now_ms) {
            return Ok(hit);
        }
        // ── Live exchange (the ONLY network seam; a fake in tests). ──
        let url =
            format!("https://api.github.com/app/installations/{installation_id}/access_tokens");
        let (status, body) = transport.post_access_token(&url, app_jwt)?;
        match status {
            200 | 201 => {
                let parsed: AccessTokenResponse =
                    serde_json::from_slice(&body).map_err(|_| AppAuthError::ExchangeFailed {
                        status: Some(status),
                    })?;
                if parsed.token.is_empty() {
                    return Err(AppAuthError::ExchangeFailed {
                        status: Some(status),
                    });
                }
                // Compute the refresh deadline from the server `expires_at` (minus
                // skew), falling back to a conservative TTL when it can't be parsed.
                let expiry_ms = parsed
                    .expires_at
                    .as_deref()
                    .and_then(parse_rfc3339_to_epoch_secs)
                    .map(|s| s.saturating_mul(1000))
                    .unwrap_or_else(|| now_ms.saturating_add(FALLBACK_TTL_MS));
                let refresh_after_ms = expiry_ms.saturating_sub(REFRESH_SKEW_MS);
                let token = InstallationToken::new(parsed.token);
                *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(CachedToken {
                    installation_id: installation_id.to_string(),
                    token: token.clone(),
                    refresh_after_ms,
                });
                Ok(token)
            }
            // 404 → the installation does not exist / does not cover the App: the
            // honest coverage-gap outcome the live lane maps to PARTIAL.
            404 => Err(AppAuthError::RepoNotCovered {
                repo: format!("installation {installation_id}"),
            }),
            other => Err(AppAuthError::ExchangeFailed {
                status: Some(other),
            }),
        }
    }
}

/// GitHub's `access_tokens` success body (subset). Liberal in what it accepts
/// (no `deny_unknown_fields`) — GitHub carries many more fields we ignore.
#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    /// The scoped installation token (SECRET — wrapped into [`InstallationToken`]).
    token: String,
    /// RFC-3339 UTC expiry (e.g. `2026-07-08T12:00:00Z`); `None`/unparseable ⇒
    /// the conservative fallback TTL.
    #[serde(default)]
    expires_at: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// JWT signing — RS256 App JWT from the on-disk PEM (jsonwebtoken).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct AppJwtClaims {
    /// Issued-at, backdated 60 s to tolerate minor clock drift (GitHub guidance).
    iat: u64,
    /// Expiry — GitHub caps the App JWT at 10 min; we use a conservative 9 min.
    exp: u64,
    /// Issuer = the numeric App id.
    iss: String,
}

/// Sign the short-lived App JWT (RS256) from the PEM private key. The key bytes
/// are used only inside this scope and never surface in the error (which carries
/// only a generic reason). Fail-closed → [`AppAuthError::JwtSigning`].
fn sign_app_jwt(pem: &[u8], app_id: &str, now_secs: u64) -> Result<String, AppAuthError> {
    let claims = AppJwtClaims {
        iat: now_secs.saturating_sub(60),
        exp: now_secs.saturating_add(9 * 60),
        iss: app_id.to_string(),
    };
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem)
        .map_err(|_| AppAuthError::JwtSigning("invalid RSA private key (PEM)".to_string()))?;
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &key)
        .map_err(|_| AppAuthError::JwtSigning("RS256 encode failed".to_string()))
}

/// Parse an RFC-3339 UTC timestamp of the form `YYYY-MM-DDTHH:MM:SSZ` (the shape
/// GitHub returns for `expires_at`) into an epoch-seconds value. Returns `None`
/// on any deviation (fractional seconds, offsets, malformed) — the caller then
/// uses the conservative fallback TTL, so a parse miss is SAFE, never fatal.
///
/// The civil-days computation is Howard Hinnant's `days_from_civil` (exact,
/// dependency-free), valid for the proleptic Gregorian calendar.
fn parse_rfc3339_to_epoch_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    let bytes = s.as_bytes();
    // Exactly "YYYY-MM-DDTHH:MM:SSZ" = 20 chars.
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if (bytes[10] != b'T' && bytes[10] != b't') || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    if bytes[19] != b'Z' && bytes[19] != b'z' {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    // days_from_civil (Hinnant): days since 1970-01-01.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hour * 3600 + min * 60 + sec;
    if secs < 0 { None } else { Some(secs as u64) }
}

/// Wall-clock epoch-ms (best-effort; a pre-epoch clock degrades to 0).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport — the ONLY part that touches the network (mockable). Mirrors the
// `RunnerTransport`/`UreqRunnerTransport` split in `lease_client.rs`.
// ─────────────────────────────────────────────────────────────────────────────

/// A single HTTP exchange against the GitHub App API — the ONLY network seam.
/// Splitting it behind a trait lets the JWT signing, caching, decode, and error
/// mapping be proven hermetically with a fake transport (no live call).
pub trait TokenTransport {
    /// `POST {url}` with `Authorization: Bearer {app_jwt}` and the GitHub API
    /// headers, empty body. Returns the decoded `(status, body_bytes)`. A
    /// transport-level fault (no response) is [`AppAuthError::ExchangeFailed`]
    /// with `status: None`.
    fn post_access_token(&self, url: &str, app_jwt: &str) -> Result<(u16, Vec<u8>), AppAuthError>;
}

/// The real `ureq`-backed transport — the thin network seam with a BOUNDED
/// timeout (a slow GitHub can never wedge the caller). This is the ONLY code that
/// opens a socket; everything around it is proven without it. Exercised solely in
/// production / the dogfood soak, never in a test.
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqTokenTransport;

impl TokenTransport for UreqTokenTransport {
    fn post_access_token(&self, url: &str, app_jwt: &str) -> Result<(u16, Vec<u8>), AppAuthError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .build();
        let resp = agent
            .post(url)
            .set("Authorization", &format!("Bearer {app_jwt}"))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", "hugit-mirror")
            .call();
        match resp {
            Ok(r) => {
                let status = r.status();
                let mut buf = Vec::new();
                use std::io::Read;
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|_| AppAuthError::ExchangeFailed { status: None })?;
                Ok((status, buf))
            }
            // ureq surfaces non-2xx as `Error::Status(code, resp)`; read the body
            // so the caller can map the status (but the body is never logged).
            Err(ureq::Error::Status(code, resp)) => {
                let mut buf = Vec::new();
                use std::io::Read;
                let _ = resp.into_reader().read_to_end(&mut buf);
                Ok((code, buf))
            }
            Err(_) => Err(AppAuthError::ExchangeFailed { status: None }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    /// An in-memory transport: returns a queued `(status, body)` and counts calls.
    /// NEVER opens a socket.
    #[derive(Default)]
    struct FakeTransport {
        responses: Mutex<std::collections::VecDeque<(u16, Vec<u8>)>>,
        calls: AtomicUsize,
        last_jwt: Mutex<String>,
    }
    impl FakeTransport {
        fn with(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: AtomicUsize::new(0),
                last_jwt: Mutex::new(String::new()),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    impl TokenTransport for FakeTransport {
        fn post_access_token(
            &self,
            _url: &str,
            app_jwt: &str,
        ) -> Result<(u16, Vec<u8>), AppAuthError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_jwt.lock().unwrap() = app_jwt.to_string();
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(AppAuthError::ExchangeFailed { status: None })
        }
    }

    /// Write a secret dir with a (dummy, never-parsed) private key + app-id (+
    /// optional installation-id). The exchange/cache tests use the JWT-free
    /// [`AppAuth::exchange_and_cache`] core, so no REAL RSA key is needed anywhere
    /// (a committed test key would trip the repo's `*.pem` secret-hygiene ignore).
    fn write_creds(dir: &Path, with_installation: bool) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("private-key.pem"), b"dummy-not-a-real-key").unwrap();
        std::fs::write(dir.join("app-id"), "123456\n").unwrap();
        if with_installation {
            std::fs::write(dir.join("installation-id"), "77\n").unwrap();
        }
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "hugit-app-auth-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        p
    }

    fn access_body(token: &str, expires_at: &str) -> Vec<u8> {
        format!(r#"{{"token":"{token}","expires_at":"{expires_at}"}}"#).into_bytes()
    }

    // A dummy App JWT — the exchange core forwards it VERBATIM as the Bearer; its
    // shape is irrelevant to the exchange/cache/decode logic under test (real
    // RS256 signing is covered separately in `sign_app_jwt_rejects_a_bad_key`).
    const DUMMY_JWT: &str = "header.payload.signature";

    #[test]
    fn fake_transport_models_a_real_token_and_forwards_the_jwt() {
        let auth = AppAuth::new(tmp_dir("mint"));
        let transport = FakeTransport::with(vec![(
            201,
            access_body("ghs_realtoken_abc", "2999-01-01T00:00:00Z"),
        )]);
        let token = auth
            .exchange_and_cache("77", DUMMY_JWT, &transport, 1_000)
            .unwrap();
        assert_eq!(token.expose(), "ghs_realtoken_abc");
        assert_eq!(transport.call_count(), 1);
        // The App JWT is forwarded verbatim as the Bearer the transport sends.
        assert_eq!(*transport.last_jwt.lock().unwrap(), DUMMY_JWT);
    }

    #[test]
    fn error_status_is_fail_closed_partial_never_a_token() {
        let auth = AppAuth::new(tmp_dir("err"));
        // 401 from GitHub (e.g. a bad JWT) → ExchangeFailed, never a token.
        let transport = FakeTransport::with(vec![(401, b"{}".to_vec())]);
        assert_eq!(
            auth.exchange_and_cache("77", DUMMY_JWT, &transport, 1_000)
                .unwrap_err(),
            AppAuthError::ExchangeFailed { status: Some(401) }
        );
        // A 404 maps to the coverage-gap PARTIAL outcome.
        let transport404 = FakeTransport::with(vec![(404, b"{}".to_vec())]);
        assert!(matches!(
            auth.exchange_and_cache("77", DUMMY_JWT, &transport404, 1_000)
                .unwrap_err(),
            AppAuthError::RepoNotCovered { .. }
        ));
        // An empty-token 2xx body is NOT accepted (never a fabricated token).
        let empty = FakeTransport::with(vec![(201, br#"{"token":""}"#.to_vec())]);
        assert!(matches!(
            auth.exchange_and_cache("77", DUMMY_JWT, &empty, 1_000)
                .unwrap_err(),
            AppAuthError::ExchangeFailed { .. }
        ));
    }

    #[test]
    fn caches_within_ttl_and_refreshes_after_expiry() {
        let auth = AppAuth::new(tmp_dir("cache"));
        // expires_at = 1970-01-01T01:00:00Z → 3_600_000 ms; refresh_after =
        // 3_600_000 − 300_000 = 3_300_000 ms.
        let transport = FakeTransport::with(vec![
            (201, access_body("tok-A", "1970-01-01T01:00:00Z")),
            (201, access_body("tok-B", "1970-01-01T02:00:00Z")),
        ]);
        // Mint at t=1_000 → tok-A, one network call.
        let a = auth
            .exchange_and_cache("77", DUMMY_JWT, &transport, 1_000)
            .unwrap();
        assert_eq!(a.expose(), "tok-A");
        assert_eq!(transport.call_count(), 1);
        // Within TTL (t=3_000_000 < refresh_after 3_300_000) → CACHED, no new call.
        let a2 = auth
            .exchange_and_cache("77", DUMMY_JWT, &transport, 3_000_000)
            .unwrap();
        assert_eq!(a2.expose(), "tok-A");
        assert_eq!(transport.call_count(), 1);
        // Past the refresh deadline (t=3_400_000 ≥ 3_300_000) → re-mint tok-B.
        let b = auth
            .exchange_and_cache("77", DUMMY_JWT, &transport, 3_400_000)
            .unwrap();
        assert_eq!(b.expose(), "tok-B");
        assert_eq!(transport.call_count(), 2);
    }

    #[test]
    fn a_different_installation_never_reuses_the_cache() {
        let auth = AppAuth::new(tmp_dir("iso"));
        let transport = FakeTransport::with(vec![
            (201, access_body("tok-77", "2999-01-01T00:00:00Z")),
            (201, access_body("tok-88", "2999-01-01T00:00:00Z")),
        ]);
        let a = auth
            .exchange_and_cache("77", DUMMY_JWT, &transport, 1_000)
            .unwrap();
        assert_eq!(a.expose(), "tok-77");
        // A different installation id must NOT be served the cached "77" token.
        let b = auth
            .exchange_and_cache("88", DUMMY_JWT, &transport, 1_000)
            .unwrap();
        assert_eq!(b.expose(), "tok-88");
        assert_eq!(transport.call_count(), 2);
    }

    #[test]
    fn sign_app_jwt_rejects_a_bad_key_fail_closed() {
        // A non-PEM/garbage key must fail-closed with `JwtSigning`, never panic and
        // never a signed token. (The RS256 happy path is a thin `jsonwebtoken` call
        // exercised live in prod; no private key is committed to the repo.)
        let err = sign_app_jwt(b"not-a-pem-key", "123456", 1_000).unwrap_err();
        assert!(matches!(err, AppAuthError::JwtSigning(_)));
    }

    #[test]
    fn rfc3339_parse_matches_known_epochs() {
        assert_eq!(parse_rfc3339_to_epoch_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_to_epoch_secs("1970-01-01T01:00:00Z"),
            Some(3600)
        );
        assert_eq!(
            parse_rfc3339_to_epoch_secs("2000-01-01T00:00:00Z"),
            Some(946_684_800)
        );
        // Malformed / unsupported shapes → None (safe fallback at the call site).
        assert_eq!(parse_rfc3339_to_epoch_secs("2000-01-01T00:00:00.5Z"), None);
        assert_eq!(
            parse_rfc3339_to_epoch_secs("2000-01-01T00:00:00+02:00"),
            None
        );
        assert_eq!(parse_rfc3339_to_epoch_secs("not-a-date"), None);
    }

    #[test]
    fn a_revoked_installation_refuses_the_mint_fail_closed() {
        // WP-B1 ⑤: once an installation is durably revoked, even a still-valid
        // cached-or-mintable token is refused — the mint is never attempted.
        let led_dir = tmp_dir("revoke-gate");
        let ledger = hugit_app::RevocationLedger::open(&led_dir).unwrap();
        let auth = AppAuth::new(tmp_dir("revoke-auth")).with_revocation_ledger(ledger.clone());

        // Before revocation: prime a cached token so the mint fast-path returns it
        // (no on-disk key / signing needed) — the un-revoked installation mints.
        auth.prime_installation_token("55", "tok-live");
        let ok_transport =
            FakeTransport::with(vec![(201, access_body("tok", "2999-01-01T00:00:00Z"))]);
        assert_eq!(
            auth.mint_with_transport("55", &ok_transport, 1_000)
                .unwrap()
                .expose(),
            "tok-live",
            "an un-revoked installation mints normally"
        );
        assert_eq!(ok_transport.call_count(), 0, "cache fast-path, no network");

        // Revoke. The gate runs BEFORE the cache fast-path, so even the SAME
        // AppAuth (with a still-primed cache) now refuses fail-closed.
        ledger.revoke("55").unwrap();
        let refused_transport =
            FakeTransport::with(vec![(201, access_body("tok2", "2999-01-01T00:00:00Z"))]);
        let err = auth
            .mint_with_transport("55", &refused_transport, 2_000)
            .unwrap_err();
        assert_eq!(
            err,
            AppAuthError::InstallationRevoked {
                installation_id: "55".to_string()
            }
        );
        // The transport was NEVER consulted (mint not attempted).
        assert_eq!(refused_transport.call_count(), 0);

        // Durable across a "restart": a fresh ledger over the same dir + a fresh
        // AppAuth still refuses (no cache, no network).
        let reopened = hugit_app::RevocationLedger::open(&led_dir).unwrap();
        let auth3 = AppAuth::new(tmp_dir("revoke-auth3")).with_revocation_ledger(reopened);
        let t3 = FakeTransport::with(vec![(201, access_body("tok3", "2999-01-01T00:00:00Z"))]);
        assert!(matches!(
            auth3.mint_with_transport("55", &t3, 3_000).unwrap_err(),
            AppAuthError::InstallationRevoked { .. }
        ));
        assert_eq!(t3.call_count(), 0);
        std::fs::remove_dir_all(&led_dir).ok();
    }

    #[test]
    fn missing_installation_id_is_credentials_unavailable() {
        let dir = tmp_dir("noinst");
        write_creds(&dir, false); // no installation-id file
        // Ensure the env fallback is not set for this test.
        // SAFETY: single-threaded test; no concurrent env access.
        unsafe {
            std::env::remove_var("HUGIT_GITHUB_INSTALLATION_ID");
        }
        let auth = AppAuth::new(&dir);
        assert_eq!(
            auth.mint_installation_token().unwrap_err(),
            AppAuthError::CredentialsUnavailable
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
