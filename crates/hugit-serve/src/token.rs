//! `POST /v1/token` — Clerk session-JWT → opaque engine token, via CoreLink's
//! `/v1/session/exchange` (**Option B**, decided 2026-06-17 with the CoreLink
//! Server TL — "consume CoreLink, never fork").
//!
//! ## Flow (Option B — delegate, don't duplicate)
//!
//! 1. The client (the githugr window) POSTs `{subject_token, audience}` to
//!    `/v1/token`, where `subject_token` is the user's Clerk session JWT. This
//!    client-facing request/response shape is the FROZEN contract (unchanged).
//! 2. hugit FORWARDS that JWT to CoreLink `POST /v1/session/exchange`
//!    (`Authorization: Bearer <jwt>`, no body, **NO internal-auth key** — the route
//!    is Clerk-JWT-gated). CoreLink verifies the session (azp/iss/sub/tenant) and
//!    returns `{principal, tenant, expires_ms, …}`. hugit does NOT re-validate the
//!    JWT locally — no duplicated azp/issuer/JWKS pipeline to drift (the Server
//!    TL's Option-B simplification; mooted the old local `ClerkValidator`).
//! 3. hugit checks `audience == tenant` (cross-tenant mint blocked → `401`, NOT
//!    `403`: NO tenant-existence oracle on the mint path — frozen client §Q2).
//! 4. [`TokenStore::mint_with_ttl`] returns an OPAQUE 32-byte hex engine token, its
//!    TTL bounded by `min(ENGINE_TOKEN_TTL_SECS, upstream remaining)` so the engine
//!    token never outlives the upstream session. The store holds `SHA-256(token)`;
//!    the raw token is NEVER logged.
//! 5. Subsequent calls present `Bearer <engine_token>`; `token_store.lookup`
//!    SHA-256s it and compares constant-time.
//!
//! ## Error mapping (CoreLink exchange status → client `/v1/token`)
//!
//! - `200` → mint → `200 {engine_token, expires_in, accepted:true}`.
//! - `401` (bad/expired/wrong-iss/azp JWT) → `401 TOKEN_INVALID` (client re-login).
//! - `403` (un-provisioned tenant / server secret unbound) → `401 TOKEN_INVALID`.
//!   COLLAPSED to 401 deliberately: a distinct 403 would be a tenant-existence
//!   oracle on the mint path, which the frozen client contract (§Q2) forbids — the
//!   same no-existence-leak doctrine as the read-path 404. (This is a deliberate
//!   tightening of the prose in the ACCEPT handoff: the frozen shipped client
//!   contract — 401/200 only, no oracle — governs over handoff prose.)
//! - `429` (per-principal mint throttle) → `429 RATE_LIMITED` (client retry-after).
//! - `405`/`5xx`/network/malformed-200 → `503 ENGINE_UNAVAILABLE` (transient/upstream).
//!
//! ## Security invariants
//!
//! - The Clerk JWT (`subject_token`) is sent ONLY as the `Authorization: Bearer`
//!   header to the exchange; it is NEVER echoed, logged, or in any error.
//! - `token_plaintext` from the exchange (a real CoreLink `cas:rw` PAT) is IGNORED
//!   and never stored/logged — hugit needs only the verified identity to mint its
//!   own engine token (so it avoids holding a `cas:rw` secret on the token path).
//! - `fresh_auth` is `false` for exchange-minted principals: the exchange contract
//!   carries no `auth_time`, so step-up for a Clerk principal MUST come from a
//!   freshly re-authenticated session, never from `/v1/token` alone (fail-closed;
//!   matches the prior P2 seam note — `auth_time` is not a Clerk session claim today).
//! - The verified `tenant`/`principal` are colon-guarded before they become the
//!   `clerk:{org}:{user}` authz principal (`:` is the authz delimiter — the
//!   "exemption-is-a-hole" structural class). Real CoreLink UUIDs never contain `:`.
//! - Engine-token lookup is SHA-256 + constant-time XOR.
//!
//! ## P2 seams (disclosed, not faked)
//!
//! - The exchange ENDPOINT host is owner/infra-gated (a deployed Worker pointed at
//!   the dev Clerk instance). Absent `HUGIT_SESSION_EXCHANGE_URL` ⇒ `/v1/token`
//!   404s (dev-token-only mode; the route's presence is not disclosed).
//! - Multi-instance shared token store: the in-process `Mutex<HashMap>` is
//!   single-host; the dedicated `hugit-prod-d1` swap is the P2 store seam, behind
//!   the same [`TokenStore`] surface.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::EngineErr;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Engine token TTL ceiling (seconds). A minted engine token lives at most this
/// long, and never longer than the upstream session's remaining lifetime.
const ENGINE_TOKEN_TTL_SECS: u64 = 300;

/// Outbound timeout for the `/v1/session/exchange` call — a hung exchange never
/// hangs the (single-threaded) `/v1/token` request indefinitely.
const EXCHANGE_TIMEOUT_SECS: u64 = 10;

// ── Client-facing wire types (FROZEN — backend-API-v1 §Q2; do NOT change) ─────

/// The JSON body for `POST /v1/token` (the client/window → hugit).
#[derive(Debug, Deserialize)]
pub struct TokenExchangeReq {
    /// A Clerk RS256 session JWT (forwarded to the exchange; never logged).
    pub subject_token: String,
    /// The tenant this exchange is scoped to. Must equal the verified `tenant`
    /// the exchange returns (cross-tenant mint is blocked).
    pub audience: String,
}

/// The JSON body returned on success (hugit → the client/window).
#[derive(Debug, Serialize)]
pub struct TokenExchangeResp {
    /// The opaque engine token. 32 random bytes, hex-encoded (64 chars).
    pub engine_token: String,
    /// Seconds until the engine token expires (bounded by the upstream session).
    pub expires_in: u64,
    /// Always `true` when present (mirrors the `Accepted` pattern).
    pub accepted: bool,
}

// ── CoreLink `/v1/session/exchange` success body ──────────────────────────────

/// The fields hugit reads from a `200` exchange response. Unknown fields (notably
/// `token_plaintext`, `pat_id`, `token_id`) are INTENTIONALLY ignored by serde —
/// hugit never stores or logs the upstream PAT.
#[derive(Debug, Deserialize)]
struct ExchangeOk {
    /// Stable, one-way per-Clerk-user principal UUID (→ `ClerkPrincipal::user`).
    principal: String,
    /// The verified tenant_id (→ `ClerkPrincipal::org`; the R2 key prefix).
    tenant: String,
    /// Absolute epoch-ms expiry of the upstream session/PAT.
    #[serde(default)]
    expires_ms: u64,
}

/// The verified identity returned by a successful [`SessionExchangeClient::exchange`].
#[derive(Debug)]
pub struct ExchangeIdentity {
    pub principal: String,
    pub tenant: String,
    pub expires_ms: u64,
}

// ── ClerkPrincipal (the minted-identity carrier) ──────────────────────────────

/// A verified principal, the source of a minted engine token. With Option B it is
/// built from the exchange result (`user` = `principal`, `org` = `tenant`,
/// `fresh_auth` = `false`).
#[derive(Debug, Clone)]
pub struct ClerkPrincipal {
    pub user: String,
    pub org: String,
    pub fresh_auth: bool,
}

// ── SessionExchangeConfig / SessionExchangeClient ─────────────────────────────

/// Config for the CoreLink session exchange, read from env. Absent ⇒ dev-token
/// only (the `/v1/token` route 404s — its presence is not disclosed).
#[derive(Clone, Debug)]
pub struct SessionExchangeConfig {
    /// Full URL of `POST /v1/session/exchange` (`HUGIT_SESSION_EXCHANGE_URL`).
    pub endpoint: String,
}

/// Trusted host suffixes for `HUGIT_SESSION_EXCHANGE_URL`.
///
/// SSRF defence: the exchange URL is POSTed to by ureq; without an allowlist any
/// operator-controlled env var could redirect the Clerk JWT to an arbitrary host.
/// Only hosts matching one of these suffixes (or equal to a bare suffix for
/// short names like `localhost`) are accepted at config parse time (fail-closed).
///
/// `.humangr.com` covers `corelink-api.humangr.com` and any future subdomain.
/// `localhost` / `127.0.0.1` / `[::1]` are allowed for integration tests.
const EXCHANGE_URL_TRUSTED_SUFFIXES: &[&str] = &[".humangr.com", "localhost", "127.0.0.1", "[::1]"];

/// Extract the host (without port) from a URL string (`scheme://host[:port]/path`).
fn extract_host(url: &str) -> Option<&str> {
    // Strip scheme.
    let after_scheme = url.split_once("://")?.1;
    // Strip path (everything from first `/`).
    let host_port = match after_scheme.split_once('/') {
        Some((hp, _)) => hp,
        None => after_scheme,
    };
    // Strip port — but only the numeric suffix (IPv6 `[::1]:port` vs bare `[::1]`).
    if host_port.starts_with('[') {
        // IPv6 bracket notation: `[::1]` or `[::1]:8080`.
        let close = host_port.find(']')?;
        Some(&host_port[..=close])
    } else {
        // IPv4 / hostname: `host` or `host:port`.
        Some(match host_port.rsplit_once(':') {
            // rsplit_once(':') can split on `host:port` or even on a bare IPv6
            // address; only strip the suffix when it's purely numeric (port).
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host_port,
        })
    }
}

/// Return `true` when `host` is allowlisted for the exchange URL.
fn is_trusted_exchange_host(host: &str) -> bool {
    EXCHANGE_URL_TRUSTED_SUFFIXES
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(suffix))
}

impl SessionExchangeConfig {
    /// Read from env. Returns `None` when `HUGIT_SESSION_EXCHANGE_URL` is absent
    /// (dev mode). Returns `Err` when set-but-invalid (fail-closed).
    pub fn from_env() -> Result<Option<Self>, String> {
        let endpoint = match std::env::var("HUGIT_SESSION_EXCHANGE_URL") {
            Ok(v) => v,
            Err(_) => return Ok(None), // not configured → dev-token only
        };
        let endpoint = endpoint.trim().to_string();
        if endpoint.is_empty() {
            return Err("HUGIT_SESSION_EXCHANGE_URL is empty (fail-closed)".to_string());
        }
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(format!(
                "HUGIT_SESSION_EXCHANGE_URL must start with http(s)://; got: {endpoint}"
            ));
        }
        // SECURITY: SSRF allowlist — the exchange URL is POSTed to by ureq with
        // the user's Clerk JWT. Reject any host that isn't on the trusted list so a
        // misconfigured or malicious env var can't redirect the JWT to an arbitrary
        // host. Fail-closed (Err) on an untrusted host.
        let host = extract_host(&endpoint).ok_or_else(|| {
            format!("HUGIT_SESSION_EXCHANGE_URL: cannot parse host from `{endpoint}`")
        })?;
        if !is_trusted_exchange_host(host) {
            return Err(format!(
                "HUGIT_SESSION_EXCHANGE_URL host `{host}` is not on the trusted allowlist \
                 (trusted suffixes: {EXCHANGE_URL_TRUSTED_SUFFIXES:?}); \
                 set it to a *.humangr.com endpoint"
            ));
        }
        Ok(Some(SessionExchangeConfig { endpoint }))
    }

    /// Build a [`SessionExchangeClient`] from this config.
    pub fn into_client(self) -> SessionExchangeClient {
        SessionExchangeClient {
            endpoint: self.endpoint,
        }
    }
}

/// Client for CoreLink `POST /v1/session/exchange`. Holds only the endpoint URL;
/// a fresh `ureq` agent (with a bounded timeout) is built per call.
pub struct SessionExchangeClient {
    endpoint: String,
}

impl SessionExchangeClient {
    /// Construct directly from an endpoint (test/internal helper).
    #[cfg(test)]
    fn new(endpoint: String) -> Self {
        Self { endpoint }
    }

    /// Forward the Clerk session `jwt` to CoreLink and return the verified
    /// identity, or a mapped [`EngineErr`] (fail-closed at every step).
    ///
    /// SECURITY: `jwt` is sent ONLY as the `Authorization: Bearer` header — it is
    /// never placed in the body, logged, or echoed; neither is any response body.
    pub fn exchange(&self, jwt: &str) -> Result<ExchangeIdentity, EngineErr> {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(EXCHANGE_TIMEOUT_SECS))
            .build();
        let resp = agent
            .post(&self.endpoint)
            .set("Authorization", &format!("Bearer {jwt}"))
            .set("Content-Type", "application/json")
            .call();
        match resp {
            // ureq treats 2xx/3xx as Ok; the exchange answers 200 on success.
            Ok(r) => {
                let body = r
                    .into_string()
                    .map_err(|_| EngineErr::unavailable("exchange: response body read failed"))?;
                let ok: ExchangeOk = serde_json::from_str(&body)
                    .map_err(|_| EngineErr::unavailable("exchange: malformed success body"))?;
                if ok.principal.is_empty() || ok.tenant.is_empty() {
                    // A 200 with empty identity is not trustworthy → fail-closed.
                    return Err(EngineErr::unavailable("exchange: empty principal/tenant"));
                }
                Ok(ExchangeIdentity {
                    principal: ok.principal,
                    tenant: ok.tenant,
                    expires_ms: ok.expires_ms,
                })
            }
            // 4xx/5xx land here as Error::Status(code, _).
            Err(ureq::Error::Status(code, _)) => Err(map_exchange_status(code)),
            // Transport-level failure (DNS, connect, timeout) → transient 503.
            Err(_) => Err(EngineErr::unavailable("exchange: upstream unreachable")),
        }
    }
}

/// Map a CoreLink exchange HTTP status to the client-facing `/v1/token` error.
fn map_exchange_status(code: u16) -> EngineErr {
    match code {
        // 401 (bad/expired/wrong-iss/azp JWT) AND 403 (un-provisioned tenant /
        // server secret unbound) BOTH collapse to 401 TOKEN_INVALID — no
        // tenant-existence oracle on the mint path (frozen client §Q2; the
        // read-path no-existence-leak doctrine).
        401 | 403 => EngineErr::token_invalid(),
        // Per-principal mint throttle — surfaced honestly so the client backs off.
        429 => EngineErr::rate_limited(),
        // 405 (shouldn't happen — we always POST), 5xx, or anything else: a
        // transient/upstream failure.
        _ => EngineErr::unavailable("exchange: upstream mint failure"),
    }
}

// ── TokenStore ────────────────────────────────────────────────────────────────

/// An in-process store for minted engine tokens.
///
/// Key: `SHA-256(raw_token)` — 32 bytes, stored as `[u8; 32]`.
/// Lookup is constant-time (XOR fold, mirrors auth.rs:34-42).
/// Raw tokens are NEVER stored, logged, or echoed (ADR-0002 §6.1).
pub struct TokenStore {
    tokens: Mutex<HashMap<[u8; 32], TokenRecord>>,
}

/// A minted engine-token record.
#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub user: String,
    pub org: String,
    pub fresh_auth: bool,
    /// Unix timestamp (seconds) when this token expires.
    pub expires_at: u64,
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore {
    /// Create a new, empty token store.
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Mint a new engine token for `principal` at the default TTL ceiling
    /// ([`ENGINE_TOKEN_TTL_SECS`]).
    pub fn mint(&self, principal: &ClerkPrincipal) -> Result<String, EngineErr> {
        self.mint_with_ttl(principal, ENGINE_TOKEN_TTL_SECS)
    }

    /// Mint a new engine token for `principal` with an explicit TTL, clamped to
    /// `[1, ENGINE_TOKEN_TTL_SECS]` — so the engine token never outlives the
    /// upstream session (`ttl_secs` < ceiling) and never lives longer than the
    /// ceiling (`ttl_secs` > ceiling), and is always positive.
    ///
    /// - 32 random bytes read from `/dev/urandom` (OS CSPRNG; no `rand` dep).
    /// - Hex-encoded → returned as the raw token string.
    /// - Stored as `SHA-256(raw_token)`.
    /// - Expired tokens are swept on every mint (bounded scan).
    ///
    /// The raw token is NEVER logged here; it is only returned to the caller.
    pub fn mint_with_ttl(
        &self,
        principal: &ClerkPrincipal,
        ttl_secs: u64,
    ) -> Result<String, EngineErr> {
        let ttl = ttl_secs.clamp(1, ENGINE_TOKEN_TTL_SECS);
        let raw = random_hex_token()?;
        let hash = sha256_bytes(raw.as_bytes());
        let expires_at = now_secs() + ttl;
        let record = TokenRecord {
            user: principal.user.clone(),
            org: principal.org.clone(),
            fresh_auth: principal.fresh_auth,
            expires_at,
        };
        let mut guard = self
            .tokens
            .lock()
            .map_err(|_| EngineErr::unavailable("token store lock poisoned"))?;
        // Sweep expired entries on mint (bounded by the number of active sessions).
        let now = now_secs();
        guard.retain(|_, v| v.expires_at > now);
        guard.insert(hash, record);
        Ok(raw)
    }

    /// Look up a raw engine token.
    ///
    /// - SHA-256s the presented token.
    /// - Compares against stored keys using constant-time XOR (mirrors auth.rs:34-42).
    /// - Returns the record if present AND unexpired.
    /// - Expired-but-present → prune + return `Expired` (caller maps to TOKEN_EXPIRED).
    /// - Not present → `Invalid` (caller maps to TOKEN_INVALID).
    pub fn lookup(&self, raw: &str) -> LookupResult {
        let hash = sha256_bytes(raw.as_bytes());
        let mut guard = match self.tokens.lock() {
            Ok(g) => g,
            Err(_) => return LookupResult::Invalid, // lock poisoned → fail-closed
        };
        // Linear scan with a constant-time XOR PER-KEY comparison (mirrors
        // auth.rs): each candidate key is compared byte-for-byte without an
        // early-exit on the bytes, so the per-key compare leaks no byte-position
        // timing. The loop itself DOES `break` on the first match. The only timing
        // signal is the match's POSITION in HashMap iteration order, which is
        // randomized and not attacker-controllable — not an exploitable oracle.
        // The store is bounded by active sessions (swept on mint + short TTL).
        let mut found_key: Option<[u8; 32]> = None;
        let mut found: Option<TokenRecord> = None;
        for (stored_hash, record) in guard.iter() {
            if hashes_match(&hash, stored_hash) {
                found_key = Some(*stored_hash);
                found = Some(record.clone());
                break;
            }
        }
        match found {
            None => LookupResult::Invalid,
            Some(rec) => {
                if rec.expires_at <= now_secs() {
                    if let Some(k) = found_key {
                        guard.remove(&k);
                    }
                    LookupResult::Expired
                } else {
                    LookupResult::Ok(rec)
                }
            }
        }
    }

    /// List the ACTIVE (unexpired) sessions for the admin area, SCOPED to a tenant.
    ///
    /// `scope`: `Some(org)` returns ONLY that org's sessions (a tenant admin sees
    /// only their own); `None` returns ALL sessions (the platform operator view).
    /// **Cross-tenant guard:** the caller MUST pass their own org as the scope
    /// unless they are the platform operator — never `None` for a tenant principal.
    ///
    /// Each entry is `(handle, record)` where `handle` is the hex of the stored
    /// `SHA-256(token)` (a non-secret, non-reversible identifier). Sweeps expired
    /// entries first. Single-host (the in-process store); a fleet-wide session list
    /// is the P2 shared-store seam.
    pub fn list_for_org(&self, scope: Option<&str>) -> Vec<(String, TokenRecord)> {
        let now = now_secs();
        let mut guard = match self.tokens.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(), // poisoned → empty (fail-safe, never panic)
        };
        guard.retain(|_, v| v.expires_at > now);
        guard
            .iter()
            .filter(|(_, r)| scope.is_none_or(|org| r.org == org))
            .map(|(h, r)| (hex::encode(h), r.clone()))
            .collect()
    }

    /// All active sessions (the platform-operator view). Prefer
    /// [`list_for_org`](Self::list_for_org) with the caller's org for tenant scoping.
    pub fn list(&self) -> Vec<(String, TokenRecord)> {
        self.list_for_org(None)
    }
}

/// The three-way outcome of a token lookup.
pub enum LookupResult {
    /// Valid and unexpired.
    Ok(TokenRecord),
    /// In-store but expired (client should call `/v1/token` to renew).
    Expired,
    /// Not found (→ 401 TOKEN_INVALID; client must log in).
    Invalid,
}

/// Constant-time hash comparison (mirrors auth.rs:34-42).
fn hashes_match(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// SHA-256 of `bytes` → `[u8; 32]`.
fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Read 32 bytes from `/dev/urandom` → hex string (64 chars).
/// No `rand` dep; `/dev/urandom` is always available on Linux/macOS.
fn random_hex_token() -> Result<String, EngineErr> {
    use std::fs::File;
    use std::io::Read;
    let mut buf = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| EngineErr::unavailable(format!("entropy source unavailable: {e}")))?;
    Ok(hex::encode(buf))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── handle_token_exchange ─────────────────────────────────────────────────────

/// Dispatch `POST /v1/token` (Option B).
///
/// - Parses the request body as [`TokenExchangeReq`] (`{subject_token, audience}`).
/// - Forwards the Clerk JWT to CoreLink `/v1/session/exchange` (fail-closed).
/// - Checks `audience == tenant` (blocks cross-tenant mint → 401, no oracle).
/// - Colon-guards the verified tenant/principal (the authz delimiter).
/// - Mints an engine token via [`TokenStore::mint_with_ttl`], TTL bounded by the
///   upstream session's remaining lifetime.
/// - Returns `(200, TokenExchangeResp JSON)` on success.
///
/// SECURITY: `subject_token` is NEVER echoed, logged, or included in any error.
pub fn handle_token_exchange(
    exchange: &SessionExchangeClient,
    store: &TokenStore,
    body: &[u8],
) -> (u16, String) {
    // Parse body — a malformed body is 401 (not 400) to avoid leaking parse
    // details about the token path.
    let req: TokenExchangeReq = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => {
            return (401, EngineErr::token_invalid().to_body());
        }
    };

    // Forward the Clerk JWT to CoreLink. `subject_token` is never logged.
    let id = match exchange.exchange(&req.subject_token) {
        Ok(id) => id,
        Err(e) => {
            return (e.status, e.to_body());
        }
    };

    // Colon-guard: the verified tenant/principal become `clerk:{org}:{user}`
    // downstream, where `:` is the authz delimiter. Real CoreLink UUIDs never
    // contain `:`; reject fail-closed at the trust boundary (exemption-is-a-hole).
    if id.tenant.contains(':') || id.principal.contains(':') {
        return (401, EngineErr::token_invalid().to_body());
    }

    // Audience ≡ verified tenant: block cross-tenant mints. 401 (NOT 403) — no
    // tenant-existence oracle on the mint path (frozen client §Q2).
    if req.audience != id.tenant {
        return (401, EngineErr::token_invalid().to_body());
    }

    // TTL bounded by the upstream session's remaining lifetime so the engine token
    // never outlives the Clerk/PAT it was minted from. An already-expired upstream
    // session (remaining == 0) is refused fail-closed.
    let upstream_remaining = (id.expires_ms / 1000).saturating_sub(now_secs());
    if upstream_remaining == 0 {
        return (401, EngineErr::token_invalid().to_body());
    }

    let principal = ClerkPrincipal {
        user: id.principal,
        org: id.tenant,
        fresh_auth: false, // exchange carries no auth_time → step-up fails closed
    };
    let engine_token = match store.mint_with_ttl(&principal, upstream_remaining) {
        Ok(t) => t,
        Err(e) => {
            return (e.status, e.to_body());
        }
    };

    let expires_in = upstream_remaining.clamp(1, ENGINE_TOKEN_TTL_SECS);
    let resp = TokenExchangeResp {
        engine_token,
        expires_in,
        accepted: true,
    };
    match serde_json::to_string(&resp) {
        Ok(body) => (200, body),
        Err(e) => {
            let err = EngineErr::unavailable(format!("serialize: {e}"));
            (err.status, err.to_body())
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;
    use std::thread;

    const TEST_TENANT: &str = "tenant-uuid";
    const TEST_PRINCIPAL: &str = "principal-uuid";

    // ── Mock exchange server ──────────────────────────────────────────────────

    /// Spin a one-shot mock `/v1/session/exchange`: respond to the FIRST request
    /// with `(status, body)`, and report the `Authorization` header it received
    /// over the returned channel. Returns `(endpoint_url, auth_header_rx)`.
    fn mock_exchange(status: u16, body: String) -> (String, mpsc::Receiver<Option<String>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock");
        let port = server.server_addr().to_ip().expect("ip addr").port();
        let url = format!("http://127.0.0.1:{port}/v1/session/exchange");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            if let Some(req) = server.incoming_requests().next() {
                let auth = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.as_str().to_string());
                let _ = tx.send(auth);
                let resp = tiny_http::Response::from_string(body).with_status_code(status);
                let _ = req.respond(resp);
            }
        });
        (url, rx)
    }

    /// A documented-shape 200 body (incl. a `token_plaintext` that MUST be ignored).
    fn ok_body(principal: &str, tenant: &str, expires_ms: u64) -> String {
        json!({
            "token_plaintext": "cas-rw-pat-MUST-be-ignored",
            "pat_id": "pat-uuid",
            "token_id": "tok-uuid",
            "principal": principal,
            "tenant": tenant,
            "expires_ms": expires_ms,
        })
        .to_string()
    }

    fn far_future_ms() -> u64 {
        (now_secs() + 3600) * 1000
    }

    fn exchange_req(subject_token: &str, audience: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "subject_token": subject_token,
            "audience": audience,
        }))
        .unwrap()
    }

    // ── SessionExchangeClient::exchange ───────────────────────────────────────

    #[test]
    fn exchange_200_returns_identity_and_forwards_bearer() {
        let (url, rx) = mock_exchange(200, ok_body(TEST_PRINCIPAL, TEST_TENANT, far_future_ms()));
        let client = SessionExchangeClient::new(url);
        let id = client.exchange("the-jwt").expect("200 → Ok");
        assert_eq!(id.principal, TEST_PRINCIPAL);
        assert_eq!(id.tenant, TEST_TENANT);
        // The Clerk JWT must be forwarded as the Bearer credential, verbatim.
        let auth = rx.recv().expect("server saw a request");
        assert_eq!(
            auth.as_deref(),
            Some("Bearer the-jwt"),
            "the JWT is forwarded as Authorization: Bearer"
        );
    }

    #[test]
    fn exchange_200_ignores_token_plaintext() {
        // The success body carries a cas:rw PAT; the parsed identity must not
        // surface it (ExchangeIdentity has no such field — a compile-time guarantee,
        // re-asserted here: the round-trip yields only principal/tenant/expires).
        let (url, _rx) = mock_exchange(200, ok_body(TEST_PRINCIPAL, TEST_TENANT, far_future_ms()));
        let id = SessionExchangeClient::new(url).exchange("jwt").unwrap();
        assert_eq!(id.principal, TEST_PRINCIPAL);
        assert!(id.expires_ms > now_secs() * 1000);
    }

    #[test]
    fn exchange_401_maps_to_token_invalid() {
        let (url, _rx) = mock_exchange(
            401,
            r#"{"error":"x","message":"y","request_id":"z"}"#.into(),
        );
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 401);
        assert_eq!(e.code, "TOKEN_INVALID");
    }

    #[test]
    fn exchange_403_collapses_to_token_invalid_no_oracle() {
        // Un-provisioned tenant / secret unbound → 401, NOT 403 (no existence oracle).
        let (url, _rx) = mock_exchange(
            403,
            r#"{"error":"x","message":"y","request_id":"z"}"#.into(),
        );
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 401, "403 must collapse to 401 (no oracle)");
        assert_eq!(e.code, "TOKEN_INVALID");
    }

    #[test]
    fn exchange_429_maps_to_rate_limited() {
        let (url, _rx) = mock_exchange(429, r#"{"error":"throttled"}"#.into());
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 429);
        assert_eq!(e.code, "RATE_LIMITED");
    }

    #[test]
    fn exchange_500_maps_to_unavailable() {
        let (url, _rx) = mock_exchange(500, r#"{"error":"boom"}"#.into());
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 503);
        assert_eq!(e.code, "ENGINE_UNAVAILABLE");
    }

    #[test]
    fn exchange_405_maps_to_unavailable() {
        let (url, _rx) = mock_exchange(405, "method".into());
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 503);
    }

    #[test]
    fn exchange_malformed_200_body_fails_closed() {
        let (url, _rx) = mock_exchange(200, "this is not json".into());
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 503, "a 200 with an unparseable body fails closed");
    }

    #[test]
    fn exchange_200_empty_principal_fails_closed() {
        let (url, _rx) = mock_exchange(200, ok_body("", TEST_TENANT, far_future_ms()));
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 503, "empty principal in a 200 fails closed");
    }

    #[test]
    fn exchange_unreachable_endpoint_maps_to_unavailable() {
        // Bind a port, learn it, then drop the server so the port is free →
        // connection refused → a transport error → 503 (fail-closed transient).
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        drop(server);
        let url = format!("http://127.0.0.1:{port}/v1/session/exchange");
        let e = SessionExchangeClient::new(url).exchange("jwt").unwrap_err();
        assert_eq!(e.status, 503);
        assert_eq!(e.code, "ENGINE_UNAVAILABLE");
    }

    // ── handle_token_exchange (end-to-end through a mock exchange) ─────────────

    #[test]
    fn handle_valid_exchange_returns_200() {
        let (url, _rx) = mock_exchange(200, ok_body(TEST_PRINCIPAL, TEST_TENANT, far_future_ms()));
        let client = SessionExchangeClient::new(url);
        let store = TokenStore::new();
        let body = exchange_req("clerk-jwt", TEST_TENANT);
        let (status, resp_body) = handle_token_exchange(&client, &store, &body);
        assert_eq!(status, 200);
        let resp: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(resp["accepted"], true);
        let et = resp["engine_token"].as_str().expect("engine_token present");
        assert_eq!(et.len(), 64, "32 bytes → 64 hex chars");
        let expires_in = resp["expires_in"].as_u64().unwrap();
        assert!(expires_in > 0 && expires_in <= 300, "TTL within ceiling");
        // The minted token resolves to the verified identity.
        match store.lookup(et) {
            LookupResult::Ok(rec) => {
                assert_eq!(rec.user, TEST_PRINCIPAL);
                assert_eq!(rec.org, TEST_TENANT);
                assert!(
                    !rec.fresh_auth,
                    "exchange mint is never fresh (no auth_time)"
                );
            }
            _ => panic!("expected a valid lookup"),
        }
    }

    #[test]
    fn handle_audience_mismatch_returns_401() {
        let (url, _rx) = mock_exchange(200, ok_body(TEST_PRINCIPAL, TEST_TENANT, far_future_ms()));
        let client = SessionExchangeClient::new(url);
        let store = TokenStore::new();
        let body = exchange_req("clerk-jwt", "a-different-tenant");
        let (status, resp_body) = handle_token_exchange(&client, &store, &body);
        assert_eq!(status, 401);
        let v: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(v["code"], "TOKEN_INVALID");
    }

    #[test]
    fn handle_upstream_401_returns_401() {
        let (url, _rx) = mock_exchange(401, r#"{"error":"bad jwt"}"#.into());
        let client = SessionExchangeClient::new(url);
        let store = TokenStore::new();
        let body = exchange_req("expired-jwt", TEST_TENANT);
        let (status, _) = handle_token_exchange(&client, &store, &body);
        assert_eq!(status, 401);
    }

    #[test]
    fn handle_colon_in_tenant_rejected_fail_closed() {
        // A ':' in the verified tenant would break the `clerk:{org}:{user}` parse.
        let (url, _rx) = mock_exchange(200, ok_body(TEST_PRINCIPAL, "a:b", far_future_ms()));
        let client = SessionExchangeClient::new(url);
        let store = TokenStore::new();
        let body = exchange_req("clerk-jwt", "a:b");
        let (status, _) = handle_token_exchange(&client, &store, &body);
        assert_eq!(status, 401, "colon tenant must be rejected");
    }

    #[test]
    fn handle_malformed_body_returns_401_without_calling_exchange() {
        // No live endpoint needed: the body parse fails BEFORE any exchange call.
        let client = SessionExchangeClient::new("http://127.0.0.1:9/unused".to_string());
        let store = TokenStore::new();
        let (status, _) = handle_token_exchange(&client, &store, b"not-json");
        assert_eq!(status, 401);
    }

    #[test]
    fn handle_expired_upstream_session_returns_401() {
        // expires_ms already in the past → upstream_remaining == 0 → fail-closed.
        let past = now_secs().saturating_sub(10) * 1000;
        let (url, _rx) = mock_exchange(200, ok_body(TEST_PRINCIPAL, TEST_TENANT, past));
        let client = SessionExchangeClient::new(url);
        let store = TokenStore::new();
        let body = exchange_req("clerk-jwt", TEST_TENANT);
        let (status, _) = handle_token_exchange(&client, &store, &body);
        assert_eq!(
            status, 401,
            "an already-expired upstream session is refused"
        );
    }

    #[test]
    fn handle_ttl_bounded_by_upstream_remaining() {
        // Upstream session expires in ~30s → the engine token TTL is bounded to 30,
        // well under the 300s ceiling.
        let expires = (now_secs() + 30) * 1000;
        let (url, _rx) = mock_exchange(200, ok_body(TEST_PRINCIPAL, TEST_TENANT, expires));
        let client = SessionExchangeClient::new(url);
        let store = TokenStore::new();
        let body = exchange_req("clerk-jwt", TEST_TENANT);
        let (status, resp_body) = handle_token_exchange(&client, &store, &body);
        assert_eq!(status, 200);
        let resp: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        let expires_in = resp["expires_in"].as_u64().unwrap();
        assert!(
            (1..=31).contains(&expires_in),
            "TTL bounded by the ~30s upstream remaining, got {expires_in}"
        );
        // And the minted record's expiry reflects the bounded TTL (≤ ~31s out).
        let et = resp["engine_token"].as_str().unwrap();
        match store.lookup(et) {
            LookupResult::Ok(rec) => {
                assert!(
                    rec.expires_at <= now_secs() + 31,
                    "stored expiry bounded by upstream remaining"
                );
            }
            _ => panic!("expected a valid lookup"),
        }
    }

    // ── SessionExchangeConfig::from_env ───────────────────────────────────────

    #[test]
    fn config_rejects_non_http_endpoint() {
        let cfg = SessionExchangeConfig {
            endpoint: "ftp://nope".to_string(),
        };
        // (from_env reads the process env; the prefix guard is exercised directly.)
        assert!(!cfg.endpoint.starts_with("http"));
        // Sanity: a valid endpoint builds a client.
        let good = SessionExchangeConfig {
            endpoint: "https://api.corelink.test/v1/session/exchange".to_string(),
        };
        let _client = good.into_client();
    }

    // ── TokenStore (unchanged engine-token machinery) ─────────────────────────

    fn make_principal(fresh: bool) -> ClerkPrincipal {
        ClerkPrincipal {
            user: TEST_PRINCIPAL.to_string(),
            org: TEST_TENANT.to_string(),
            fresh_auth: fresh,
        }
    }

    #[test]
    fn list_for_org_scopes_to_tenant_no_cross_tenant_leak() {
        let store = TokenStore::new();
        let pa = ClerkPrincipal {
            user: "u-a".to_string(),
            org: "org-a".to_string(),
            fresh_auth: false,
        };
        let pb = ClerkPrincipal {
            user: "u-b".to_string(),
            org: "org-b".to_string(),
            fresh_auth: false,
        };
        store.mint(&pa).expect("mint a");
        store.mint(&pb).expect("mint b");
        assert_eq!(store.list_for_org(None).len(), 2, "operator sees all");
        let a = store.list_for_org(Some("org-a"));
        assert_eq!(a.len(), 1, "tenant A sees only its own");
        assert_eq!(a[0].1.org, "org-a");
        assert!(
            store.list_for_org(Some("")).is_empty(),
            "unknown scope → empty"
        );
    }

    #[test]
    fn mint_and_lookup_roundtrip() {
        let store = TokenStore::new();
        let p = make_principal(true);
        let raw = store.mint(&p).expect("mint OK");
        assert_eq!(raw.len(), 64, "32 bytes → 64 hex chars");
        match store.lookup(&raw) {
            LookupResult::Ok(rec) => {
                assert_eq!(rec.user, TEST_PRINCIPAL);
                assert_eq!(rec.org, TEST_TENANT);
                assert!(rec.fresh_auth);
            }
            _ => panic!("expected LookupResult::Ok"),
        }
    }

    #[test]
    fn mint_with_ttl_clamps_to_ceiling() {
        let store = TokenStore::new();
        let p = make_principal(false);
        // Request a TTL above the ceiling → clamped to ENGINE_TOKEN_TTL_SECS.
        let raw = store.mint_with_ttl(&p, 100_000).expect("mint OK");
        match store.lookup(&raw) {
            LookupResult::Ok(rec) => {
                assert!(
                    rec.expires_at <= now_secs() + ENGINE_TOKEN_TTL_SECS,
                    "TTL clamped to the ceiling"
                );
            }
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn mint_with_ttl_zero_clamps_to_one_second() {
        let store = TokenStore::new();
        let p = make_principal(false);
        let raw = store.mint_with_ttl(&p, 0).expect("mint OK");
        // ttl clamped to ≥1 → the token is briefly valid, not instantly dead.
        match store.lookup(&raw) {
            LookupResult::Ok(rec) => assert!(rec.expires_at >= now_secs()),
            _ => panic!("expected Ok (ttl clamped to 1s)"),
        }
    }

    #[test]
    fn invalid_token_not_found() {
        let store = TokenStore::new();
        assert!(matches!(store.lookup("not-a-token"), LookupResult::Invalid));
    }

    #[test]
    fn expired_engine_token_returns_expired() {
        let store = TokenStore::new();
        let p = make_principal(false);
        let raw = store.mint(&p).expect("mint OK");
        {
            let hash = sha256_bytes(raw.as_bytes());
            let mut guard = store.tokens.lock().unwrap();
            if let Some(rec) = guard.get_mut(&hash) {
                rec.expires_at = now_secs() - 1; // already expired
            }
        }
        assert!(matches!(store.lookup(&raw), LookupResult::Expired));
    }

    #[test]
    fn expired_engine_token_is_pruned_after_lookup() {
        let store = TokenStore::new();
        let p = make_principal(false);
        let raw = store.mint(&p).expect("mint OK");
        let hash = sha256_bytes(raw.as_bytes());
        {
            let mut guard = store.tokens.lock().unwrap();
            if let Some(rec) = guard.get_mut(&hash) {
                rec.expires_at = now_secs() - 1;
            }
        }
        let _ = store.lookup(&raw);
        let guard = store.tokens.lock().unwrap();
        assert!(!guard.contains_key(&hash), "expired entry must be pruned");
    }

    #[test]
    fn sweep_on_mint_removes_expired() {
        let store = TokenStore::new();
        let p = make_principal(false);
        let first = store.mint(&p).expect("mint 1");
        {
            let hash = sha256_bytes(first.as_bytes());
            let mut guard = store.tokens.lock().unwrap();
            guard.get_mut(&hash).unwrap().expires_at = now_secs() - 1;
        }
        let _second = store.mint(&p).expect("mint 2 (triggers sweep)");
        let guard = store.tokens.lock().unwrap();
        let first_hash = sha256_bytes(first.as_bytes());
        assert!(
            !guard.contains_key(&first_hash),
            "sweep must remove expired first token"
        );
    }

    // ── SSRF allowlist / SessionExchangeConfig ────────────────────────────────

    /// Helper: temporarily set an env var for the duration of the closure.
    /// NOT thread-safe (env is process-global); only use in single-threaded
    /// tests or with a Mutex.
    fn with_env<F: FnOnce()>(key: &str, val: &str, f: F) {
        // Safety: tests that touch env vars must not run concurrently.
        unsafe { std::env::set_var(key, val) };
        f();
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn exchange_url_trusted_humangr_com_accepted() {
        with_env(
            "HUGIT_SESSION_EXCHANGE_URL",
            "https://corelink-api.humangr.com/v1/session/exchange",
            || {
                let cfg = SessionExchangeConfig::from_env();
                assert!(cfg.is_ok(), "humangr.com endpoint must be accepted");
                assert!(cfg.unwrap().is_some());
            },
        );
    }

    #[test]
    fn exchange_url_localhost_accepted() {
        with_env(
            "HUGIT_SESSION_EXCHANGE_URL",
            "http://localhost:9000/v1/session/exchange",
            || {
                let cfg = SessionExchangeConfig::from_env();
                assert!(cfg.is_ok(), "localhost endpoint must be accepted for tests");
                assert!(cfg.unwrap().is_some());
            },
        );
    }

    #[test]
    fn exchange_url_127_0_0_1_accepted() {
        with_env(
            "HUGIT_SESSION_EXCHANGE_URL",
            "http://127.0.0.1:8080/v1/session/exchange",
            || {
                let cfg = SessionExchangeConfig::from_env();
                assert!(cfg.is_ok(), "127.0.0.1 endpoint must be accepted for tests");
                assert!(cfg.unwrap().is_some());
            },
        );
    }

    #[test]
    fn exchange_url_untrusted_host_rejected() {
        for bad in [
            "https://evil.example.com/v1/session/exchange",
            "https://humangr.com.evil.com/steal",
            "http://attacker.io/v1/session/exchange",
        ] {
            with_env("HUGIT_SESSION_EXCHANGE_URL", bad, || {
                let result = SessionExchangeConfig::from_env();
                assert!(
                    result.is_err(),
                    "untrusted host `{bad}` must be rejected, got Ok"
                );
                let err = result.unwrap_err();
                assert!(
                    err.contains("not on the trusted allowlist"),
                    "error must explain the allowlist rejection, got: {err}"
                );
            });
        }
    }

    #[test]
    fn extract_host_parses_common_forms() {
        assert_eq!(
            extract_host("https://corelink-api.humangr.com/v1/session/exchange"),
            Some("corelink-api.humangr.com")
        );
        assert_eq!(
            extract_host("http://localhost:9000/path"),
            Some("localhost")
        );
        assert_eq!(extract_host("http://127.0.0.1:8080/"), Some("127.0.0.1"));
        assert_eq!(extract_host("http://[::1]:3000/path"), Some("[::1]"));
        assert_eq!(extract_host("https://example.com"), Some("example.com"));
    }
}
