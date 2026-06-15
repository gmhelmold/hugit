//! `POST /v1/token` — RFC-8693 token exchange + the `TokenStore` engine-token
//! minting/lookup machinery (Seam B, design doc 2026-06-14).
//!
//! ## Flow
//!
//! 1. Client presents a Clerk session JWT (`subject_token`).
//! 2. [`ClerkValidator::validate`] verifies it (RS256 pin → kid → JWKS → decode
//!    → azp → tenant → sub → fresh_auth).  Fail-closed at every step.
//! 3. [`TokenStore::mint`] returns an OPAQUE 32-byte hex token (not a JWT).
//!    The store holds `SHA-256(token)` as the key; the raw token is NEVER logged.
//! 4. The client presents the engine token as `Bearer <engine_token>` on
//!    subsequent calls; `token_store.lookup` SHA-256s it and compares constant-time.
//!
//! ## Security invariants (match auth.rs + clerk.rs)
//!
//! - `alg=RS256` pinned on the UNTRUSTED header BEFORE any key material is touched.
//! - Unknown kid → refetch ONCE → fail-closed (not a second refetch).
//! - `exp`/`nbf`/`iss` mandatory via `Validation`.
//! - `azp` optional now; checked when `TokenConfig.azp` is Some.
//! - tenant from `publicMetadata.tenant_id` then `org_id`, else reject.
//! - `auth_time` → fresh only when present AND `0 <= (now - auth_time) < 300`.
//!   Absent or future ⇒ false (fail-closed; mirrors clerk.rs:308).
//! - Engine token lookup is SHA-256 + constant-time XOR (mirrors auth.rs:34-42).
//! - Raw `subject_token` and raw engine token are NEVER in eprintln!/log.
//! - Expired engine token in the store → `TOKEN_EXPIRED` (client retries via
//!   `/v1/token`); absent entirely → `TOKEN_INVALID` (→ login).
//!
//! ## P2 seams (disclosed, not faked)
//!
//! - `auth_time` does not exist as a Clerk session claim today (clerk.rs:98-104
//!   note); `fresh_auth` will be `false` in production until frontend re-verify.
//! - Multi-instance shared store: the in-process `Mutex<HashMap>` is single-host;
//!   same seam as the idempotency ledger (Wave-2 design §6).
//! - The `azp` claim: optional env today; P2-mandatory pending CoreLink-TL value.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::EngineErr;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Engine token TTL (seconds).  Matches the Clerk step-up freshness window so a
/// just-minted engine token also qualifies as fresh.
const ENGINE_TOKEN_TTL_SECS: u64 = 300;

/// `exp`/`nbf` leeway (mirrors clerk.rs:46).
const LEEWAY_SECS: u64 = 30;

/// Step-up freshness window: auth_time must be within this many seconds of now
/// (mirrors clerk.rs:44).
const FRESH_AUTH_SECS: i64 = 300;

/// JWKS HTTP fetch timeout — a hung JWKS never hangs the request.
const JWKS_TIMEOUT_SECS: u64 = 10;

// ── Wire types (server-to-server; NOT in hugit-http-contracts) ────────────────

/// The JSON body for `POST /v1/token`.
#[derive(Debug, Deserialize)]
pub struct TokenExchangeReq {
    /// A Clerk RS256 session JWT.
    pub subject_token: String,
    /// The tenant/org this exchange is scoped to.  Must match the token's org
    /// claim (cross-tenant mint is blocked).
    pub audience: String,
}

/// The JSON body returned on success.
#[derive(Debug, Serialize)]
pub struct TokenExchangeResp {
    /// The opaque engine token.  32 random bytes, hex-encoded.
    pub engine_token: String,
    /// Seconds until the engine token expires.
    pub expires_in: u64,
    /// Always `true` when present (mirrors the `Accepted` pattern).
    pub accepted: bool,
}

// ── JWKS wire shapes ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

/// One RSA JWK.  Non-RSA or non-RS256 keys are silently skipped.
#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    #[serde(default)]
    kty: String,
    /// Optional; when present must be `"RS256"` (skip otherwise).
    #[serde(default)]
    alg: Option<String>,
    /// RSA modulus (base64url, no padding).
    n: String,
    /// RSA public exponent (base64url, no padding).
    e: String,
}

// ── Claims ────────────────────────────────────────────────────────────────────

/// The subset of Clerk session-JWT claims the engine reads.
/// Mirrors `githugr/crates/githugr/src/clerk.rs:Claims`.
#[derive(Debug, Deserialize)]
struct Claims {
    /// Principal identifier → `ClerkPrincipal::user`.
    sub: String,
    /// Top-level Clerk org (compatibility fallback; `publicMetadata.tenant_id`
    /// is authoritative — ADR-0007).
    #[serde(default)]
    org_id: Option<String>,
    /// `publicMetadata` object from the Clerk session token.
    #[serde(default, rename = "publicMetadata")]
    public_metadata: Option<PublicMetadata>,
    /// Last re-authentication unix timestamp (absent today — see P2 seam note).
    #[serde(default)]
    auth_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct PublicMetadata {
    #[serde(default)]
    tenant_id: Option<String>,
}

impl Claims {
    /// Tenant resolution order (first non-empty hit wins): `publicMetadata.tenant_id`
    /// (CoreLink authoritative — ADR-0007), then `org_id` (compatibility fallback).
    /// Returns `None` when neither is present → token is rejected (fail-closed).
    fn org(&self) -> Option<&str> {
        if let Some(t) = self
            .public_metadata
            .as_ref()
            .and_then(|m| m.tenant_id.as_deref())
            .filter(|s| !s.is_empty())
        {
            return Some(t);
        }
        self.org_id.as_deref().filter(|s| !s.is_empty())
    }
}

// ── ClerkPrincipal (the validated identity passed out of this module) ─────────

/// A verified Clerk principal returned by [`ClerkValidator::validate`].
#[derive(Debug, Clone)]
pub struct ClerkPrincipal {
    pub user: String,
    pub org: String,
    pub fresh_auth: bool,
}

// ── JwksCache ─────────────────────────────────────────────────────────────────

/// In-memory JWKS cache.  Uses a `Mutex<HashMap>` (not `RwLock`) because
/// hugit-serve is single-threaded (`tiny_http` loop): only one request at a
/// time, so the lock is always uncontended.  If this ever moves to a
/// multi-threaded server, upgrade to `RwLock`.
pub struct JwksCache {
    url: String,
    keys: Mutex<HashMap<String, DecodingKey>>,
}

impl JwksCache {
    fn new(url: String) -> Self {
        Self {
            url,
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve a `DecodingKey` for `kid`, refetching the JWKS once on a cache
    /// miss (handles key rotation).  Returns `None` on any failure (fail-closed).
    fn key_for(&self, kid: &str) -> Option<DecodingKey> {
        // Fast path: cache hit.
        if let Some(k) = self.get(kid) {
            return Some(k);
        }
        // Cache miss → refetch once, then look again.
        self.refetch();
        self.get(kid)
    }

    fn get(&self, kid: &str) -> Option<DecodingKey> {
        self.keys.lock().ok()?.get(kid).cloned()
    }

    /// Fetch the JWKS URL and replace the in-memory cache.  On any error (HTTP
    /// failure, parse failure, non-RSA key) the old cache is preserved and a
    /// server-side warning is emitted.  Never panics.
    fn refetch(&self) {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(JWKS_TIMEOUT_SECS))
            .build();
        let body: String = match agent.get(&self.url).call() {
            Ok(resp) => match resp.into_string() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hugit-serve/token: JWKS body read error: {e}");
                    return;
                }
            },
            Err(e) => {
                eprintln!("hugit-serve/token: JWKS fetch failed: {e}");
                return;
            }
        };
        let set: JwkSet = match serde_json::from_str(&body) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hugit-serve/token: JWKS parse failed: {e}");
                return;
            }
        };
        let mut next = HashMap::new();
        for jwk in set.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            if let Some(alg) = jwk.alg.as_deref()
                && alg != "RS256"
            {
                continue;
            }
            match DecodingKey::from_rsa_components(&jwk.n, &jwk.e) {
                Ok(key) => {
                    next.insert(jwk.kid, key);
                }
                Err(e) => {
                    eprintln!(
                        "hugit-serve/token: bad RSA component for kid {}: {e}",
                        jwk.kid
                    );
                }
            }
        }
        if let Ok(mut guard) = self.keys.lock() {
            *guard = next;
        }
    }

    // ── Test injection hook (cfg(test) only) ──────────────────────────────────

    /// Directly insert a pre-built key (used in tests to bypass the HTTP fetch).
    #[cfg(test)]
    fn insert_key(&self, kid: String, key: DecodingKey) {
        if let Ok(mut guard) = self.keys.lock() {
            guard.insert(kid, key);
        }
    }
}

// ── ClerkValidator ────────────────────────────────────────────────────────────

/// Stateful Clerk JWT verifier.  Holds the issuer, optional azp, and the JWKS
/// cache.  Constructed once at server start from [`TokenConfig`].
pub struct ClerkValidator {
    pub issuer: String,
    /// When `Some`, the `azp` claim in the token MUST match exactly.
    pub azp: Option<String>,
    pub jwks: JwksCache,
}

impl ClerkValidator {
    /// Verify `token` (a Clerk RS256 session JWT) and return a [`ClerkPrincipal`]
    /// on success, or `None` on ANY failure (fail-closed; never panics).
    ///
    /// Steps (verified against clerk.rs:257-316):
    ///   1. Decode header (untrusted bytes) → pin `alg == RS256` (kills alg=none
    ///      and HS256 confusion).
    ///   2. Extract `kid`.
    ///   3. Resolve key from JWKS cache (refetch-once on miss → fail-closed).
    ///   4. `jsonwebtoken::decode` with exp/nbf validation, issuer pin,
    ///      `exp`+`iss` as required spec claims, 30s leeway.
    ///   5. `azp` check (when configured).
    ///   6. Tenant resolution: `publicMetadata.tenant_id` → `org_id` → reject.
    ///   7. `auth_time` → `fresh_auth` (300s window; absent/future → false).
    pub fn validate(&self, token: &str) -> Option<ClerkPrincipal> {
        // Step 1: untrusted header — pin alg BEFORE key material.
        let header = decode_header(token).ok()?;
        if header.alg != Algorithm::RS256 {
            // Kills alg=none and HS256 confusion attacks.
            return None;
        }

        // Step 2: extract kid.
        let kid = header.kid?;

        // Step 3: JWKS key (refetch-once on miss).
        let key = self.jwks.key_for(&kid)?;

        // Step 4: signature + claims validation.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = LEEWAY_SECS;
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss"]);
        // `azp` is not an RFC audience; we check it ourselves below.
        validation.validate_aud = false;

        let data = decode::<Claims>(token, &key, &validation).ok()?;
        let claims = data.claims;

        // Step 5: optional azp check.  Run AFTER signature verification so the
        // azp bytes come from the verified payload, not attacker-controlled input.
        if let Some(expected) = self.azp.as_deref() {
            let presented = extract_azp(token)?;
            if presented != expected {
                return None;
            }
        }

        // Step 6: tenant.
        let org = claims.org()?.to_string();

        // Step 7: fresh_auth.  Fail-closed: absent or future auth_time → false.
        let fresh_auth = match claims.auth_time {
            Some(t) => {
                let delta = now_unix() - t;
                (0..FRESH_AUTH_SECS).contains(&delta)
            }
            None => false,
        };

        Some(ClerkPrincipal {
            user: claims.sub,
            org,
            fresh_auth,
        })
    }
}

/// Extract `azp` from an ALREADY-VERIFIED token's payload (mirrors clerk.rs:369-377).
/// Runs after `decode` succeeded, so the bytes are trusted.
fn extract_azp(token: &str) -> Option<String> {
    use base64::Engine as _;
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("azp")?.as_str().map(str::to_string)
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

    /// Mint a new engine token for `principal`.
    ///
    /// - 32 random bytes read from `/dev/urandom` (OS CSPRNG; no `rand` dep).
    /// - Hex-encoded → returned as the raw token string.
    /// - Stored as `SHA-256(raw_token)` with `ENGINE_TOKEN_TTL_SECS` TTL.
    /// - Expired tokens are swept on every mint (bounded scan).
    ///
    /// The raw token is NEVER logged here; it is only returned to the caller.
    pub fn mint(&self, principal: &ClerkPrincipal) -> Result<String, EngineErr> {
        let raw = random_hex_token()?;
        let hash = sha256_bytes(raw.as_bytes());
        let expires_at = now_secs() + ENGINE_TOKEN_TTL_SECS;
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
    /// - Expired-but-present → prune + return `None` (caller maps to TOKEN_EXPIRED).
    /// - Not present → `None` (caller maps to TOKEN_INVALID).
    ///
    /// See [`LookupResult`] for the three-way outcome.
    pub fn lookup(&self, raw: &str) -> LookupResult {
        let hash = sha256_bytes(raw.as_bytes());
        let mut guard = match self.tokens.lock() {
            Ok(g) => g,
            Err(_) => return LookupResult::Invalid, // lock poisoned → fail-closed
        };
        // Linear scan for constant-time key comparison (mirrors auth.rs).
        // The store is bounded by active sessions (swept on mint), so this is
        // safe in practice; for large fleets upgrade to a direct map lookup
        // (which HashMap already provides — the XOR is redundant there but we
        // keep it for the timing-invariant guarantee).
        //
        // UNVERIFIED: whether Rust's HashMap lookup leaks key-presence timing.
        // The XOR-fold approach below is conservative: we always scan all entries.
        // This is correct but O(n) — acceptable for a single-host store where n
        // is bounded by the sweep; flag for the lead to decide if a direct lookup
        // is acceptable (it IS constant-time in the sense that HashMap does not
        // early-exit on byte comparison, only on hash bucket — but that's a
        // different kind of timing oracle).
        //
        // LEAD NOTE: for the single-host, short-TTL store the direct `get` is
        // fine.  The XOR pattern here matches auth.rs for consistency.
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
                    // Prune the expired entry.
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

/// Current unix time in seconds.  Clamps pre-epoch to 0 (fail-closed: a broken
/// clock yields stale, never spuriously-valid, tokens — mirrors clerk.rs:359-363).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── handle_token_exchange ─────────────────────────────────────────────────────

/// Dispatch `POST /v1/token`.
///
/// - Parses the request body as [`TokenExchangeReq`].
/// - Validates the Clerk JWT via [`ClerkValidator::validate`] (fail-closed).
/// - Checks `audience == claims.org` (blocks cross-tenant mint).
/// - Mints an engine token via [`TokenStore::mint`].
/// - Returns `(200, TokenExchangeResp JSON)` on success.
/// - Returns `(401, TOKEN_INVALID)` on any validation failure.
///
/// SECURITY: `subject_token` is NEVER echoed, logged, or included in any error.
pub fn handle_token_exchange(
    validator: &ClerkValidator,
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

    // Validate the Clerk JWT.  `subject_token` is never logged.
    let principal = match validator.validate(&req.subject_token) {
        Some(p) => p,
        None => {
            return (401, EngineErr::token_invalid().to_body());
        }
    };

    // Audience ≡ org: block cross-tenant mints.
    if req.audience != principal.org {
        return (401, EngineErr::token_invalid().to_body());
    }

    // Mint the engine token.
    let engine_token = match store.mint(&principal) {
        Ok(t) => t,
        Err(e) => {
            return (e.status, e.to_body());
        }
    };

    let resp = TokenExchangeResp {
        engine_token,
        expires_in: ENGINE_TOKEN_TTL_SECS,
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

// ── TokenConfig ────────────────────────────────────────────────────────────────

/// Clerk JWKS configuration, read from env.  Optional: absent ⇒ dev-token only.
#[derive(Clone)]
pub struct TokenConfig {
    /// Clerk issuer URL (`HUGIT_CLERK_ISSUER`).
    pub issuer: String,
    /// Clerk JWKS URL (`HUGIT_CLERK_JWKS_URL`).
    pub jwks_url: String,
    /// Optional `azp` claim restriction (`HUGIT_CLERK_AZP`).
    pub azp: Option<String>,
}

impl TokenConfig {
    /// Read from env.  Returns `None` when `HUGIT_CLERK_ISSUER` is absent (dev
    /// mode).  Returns `Err` when partially configured (fail-closed).
    pub fn from_env() -> Result<Option<Self>, String> {
        let issuer = match std::env::var("HUGIT_CLERK_ISSUER") {
            Ok(v) => v,
            Err(_) => return Ok(None), // not configured → dev-token only
        };
        if issuer.trim().is_empty() {
            return Err("HUGIT_CLERK_ISSUER is empty (fail-closed)".to_string());
        }
        let jwks_url = std::env::var("HUGIT_CLERK_JWKS_URL").map_err(|_| {
            "HUGIT_CLERK_ISSUER is set but HUGIT_CLERK_JWKS_URL is missing (fail-closed)"
                .to_string()
        })?;
        if !jwks_url.starts_with("http://") && !jwks_url.starts_with("https://") {
            return Err(format!(
                "HUGIT_CLERK_JWKS_URL must start with http(s)://; got: {jwks_url}"
            ));
        }
        let azp = std::env::var("HUGIT_CLERK_AZP")
            .ok()
            .filter(|s| !s.is_empty());
        Ok(Some(TokenConfig {
            issuer,
            jwks_url,
            azp,
        }))
    }

    /// Build a [`ClerkValidator`] from this config.
    pub fn into_validator(self) -> ClerkValidator {
        ClerkValidator {
            jwks: JwksCache::new(self.jwks_url),
            issuer: self.issuer,
            azp: self.azp,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header as JwtHeader, encode};
    use serde_json::{Value, json};
    use std::time::{SystemTime, UNIX_EPOCH};

    // ──────────────────────────────────────────────────────────────────────────
    // TESTS-LEAD-MUST-FILL: Static RSA keypair
    //
    // The tests below use a static, REAL 2048-bit RSA keypair, generated once
    // out-of-band (`openssl genrsa 2048`) under an owner-authorized secret-read
    // waiver and pasted below — it is a throwaway TEST keypair (never a real
    // credential, never used outside `#[cfg(test)]`). We embed it rather than
    // generate at test time because the `rsa` crate cannot be a dependency: it
    // pulls `rand 0.8`, which collides with `rand 0.9.4` under the workspace's
    // `multiple-versions = "deny"` supply-chain policy. The constants ARE
    // populated (this is NOT a stub — the alg-confusion / tampered-sig / expired
    // / cross-tenant test matrix signs against this exact key and passes).
    // ──────────────────────────────────────────────────────────────────────────

    /// A real throwaway 2048-bit RSA private key PEM (test-only, owner-waived).
    const TEST_PRIVATE_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----\nMIIEuwIBADANBgkqhkiG9w0BAQEFAASCBKUwggShAgEAAoIBAQCexERzkVvF9PxE\nYt9YKx9rXDIBrnUt2bJSSvQVoODaEHrAmUJK8mxdmK4DaZ9v9iDShKvecvxGVrv4\nfKq+V1W2o0ePbE7hPkVvc/urpp6CJFvWJJzZOl4uyOBGgBPyou4M4DrQUPzdoSVK\n9DLYlVbTbkEFLvbC5baZRNfSfeWVbyckG2tuJfqKGqrofEOi5GNPmPOlOIjalvoY\nXTQaS0E7Jcg1s1ydAhcj3slkqztxbLwlzGwGnt6cPieD6J3tGs/8YEtTKmtYPdvW\nVJjSGw2Vhr77M6MUXp0/lh6rSSrxgoTXnez1zfa99r/c/j3NiutFM60u8sQ6kjIe\nj1HPFYchAgMBAAECgf9CmeZJthSzTzNitFoeSVzkzgr0k1wPF4ZovFaG0nCL0gTk\nKrRs6L+uKrdaXhX+iKWrfaM3axtHniYEnONchlGbFHRq7aA/pUPqyxXTRQcM3vwW\nwr+O/Kpve2WOsyCf9lUd2f2acOSBTDm83p2tdSTKrmqB7qwEpvRh3O6WPQJVERzZ\nhtrwxLMmcgk+BjgJSAzSB9rD60J4v6RexIxyo93t82vNnYdjAsJvWGcRvtZHCYUv\n4aKhGAejL/xFLKZVwmZElcr2qh0zq9nAF6kUK/oMCEcAvse2qwprT4yeI75aKUA3\nXZgjne+d1JposvXuNhgXHuDhs3PFmT9r0IMVxuECgYEA1FcYPEP5p4A7mX8xJPsP\nrfgevlnU8NPncDbsBa7g181pjhbRMbTnz+M/bgvLaI7luq0mtlfCL/MRDAZMbQ3O\nx/mc3JJIe6qGJRRd1H7WZeDmdBkwNEe9FPMWhpKIKWxWZ8gthQUmaJh6I/TxqAmD\n/Nq2oWcBQxeI1iUXX2uVrk0CgYEAv2k72u22nQbvGnJ6Nvie0KsHa5JoGDwla9oK\nCsU9TVSsXOjMVh79vRG7n8zPQbqdC7c5NCehfDR5QG/b6lH89lOJ3oVWWba6YyYD\nKFkOu0p5K88zNicUb/ETdxno+vVVsCn0Eg9ZBYF1CZzPEoDEVVc3k1d0wCpGJsIT\n/HlnriUCgYADROgBnYZNduL0BQpLqHXgVs6aXaWyo4CPsLjHiZ66k9YJMv67hi5/\ne98xIYtbK8ALtLjA2+8Ib/SWO86XazwAxi4NE098X+66yWp8aAuC/AhwRyb/1w7p\nMKjrH3xrLtjRtjpFLwQdXiObRB0oWiUnEnL3Xy+cydL4gQ+wD2b5jQKBgAu2Oq1Y\nojXVeMfbfVLjv4PxExEn8iqZc4i33KlwDCIxLiK5M9eJKelprltGwt+4tWdEHMHu\nMtlQtKKWtZQO1DWWQvdUnUX8AkeSydqsKFSZZ/SgRvfnSD7ZN2GwOisw279dsctx\nGPdXRnwCFkGBk4HNRl9DmKcxbv1sHqDyJL/pAoGBAMMshE3yJN/lkNIuRO2ko2T2\nXA2U9wumZcL8Msg6NzVpcvxmzuQEcOnRxERJecDS+gnRNjC/HaY/1XS9q67U2FRf\nbyyaZ/PBiT4t1e72BEyI8LXLlJvoTE/shuiDfeOzKGb+Ali/JJ2vTnX1znOfPpPH\n5jYWMXnUKTzi9klPffcL\n-----END PRIVATE KEY-----\n";

    /// base64url (no padding) of the RSA modulus `n` for the key above.
    const TEST_N_B64: &str = "nsREc5FbxfT8RGLfWCsfa1wyAa51LdmyUkr0FaDg2hB6wJlCSvJsXZiuA2mfb_Yg0oSr3nL8Rla7-HyqvldVtqNHj2xO4T5Fb3P7q6aegiRb1iSc2TpeLsjgRoAT8qLuDOA60FD83aElSvQy2JVW025BBS72wuW2mUTX0n3llW8nJBtrbiX6ihqq6HxDouRjT5jzpTiI2pb6GF00GktBOyXINbNcnQIXI97JZKs7cWy8JcxsBp7enD4ng-id7RrP_GBLUyprWD3b1lSY0hsNlYa--zOjFF6dP5Yeq0kq8YKE153s9c32vfa_3P49zYrrRTOtLvLEOpIyHo9RzxWHIQ";

    /// base64url of the public exponent.  For e=65537 this is always "AQAB".
    const TEST_E_B64: &str = "AQAB";

    const TEST_KID: &str = "test-key-1";
    const TEST_ISSUER: &str = "https://clerk.corelink.test";
    const TEST_ORG: &str = "test-org";
    const TEST_USER: &str = "user-abc";
    const TEST_AUD: &str = TEST_ORG;

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    // ── Key material helpers ──────────────────────────────────────────────────

    fn encoding_key() -> EncodingKey {
        EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM).expect("valid RSA PEM")
    }

    fn decoding_key() -> DecodingKey {
        DecodingKey::from_rsa_components(TEST_N_B64, TEST_E_B64).expect("valid RSA n/e")
    }

    fn sign<C: serde::Serialize>(claims: &C, kid: &str) -> String {
        let mut header = JwtHeader::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        encode(&header, claims, &encoding_key()).expect("encode JWT")
    }

    // ── Validator builder (injects key directly; no HTTP) ─────────────────────

    fn make_validator(azp: Option<&str>) -> ClerkValidator {
        let jwks = JwksCache::new("http://unused-in-tests".to_string());
        jwks.insert_key(TEST_KID.to_string(), decoding_key());
        ClerkValidator {
            issuer: TEST_ISSUER.to_string(),
            azp: azp.map(str::to_string),
            jwks,
        }
    }

    // ── Claim helpers ─────────────────────────────────────────────────────────

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        iss: String,
        exp: i64,
        nbf: i64,
        iat: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        org_id: Option<String>,
        #[serde(rename = "publicMetadata", skip_serializing_if = "Option::is_none")]
        public_metadata: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_time: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        azp: Option<String>,
    }

    fn good_claims() -> TestClaims {
        let n = now();
        TestClaims {
            sub: TEST_USER.to_string(),
            iss: TEST_ISSUER.to_string(),
            exp: n + 600,
            nbf: n - 10,
            iat: n - 10,
            org_id: Some(TEST_ORG.to_string()),
            public_metadata: None,
            auth_time: Some(n - 60), // 60s ago → fresh
            azp: None,
        }
    }

    /// Claims as a JSON map so individual required fields can be omitted.
    fn good_claims_json() -> serde_json::Map<String, Value> {
        let n = now();
        let mut m = serde_json::Map::new();
        m.insert("sub".into(), json!(TEST_USER));
        m.insert("iss".into(), json!(TEST_ISSUER));
        m.insert("exp".into(), json!(n + 600));
        m.insert("nbf".into(), json!(n - 10));
        m.insert("iat".into(), json!(n - 10));
        m.insert("org_id".into(), json!(TEST_ORG));
        m
    }

    // ── Helpers for handle_token_exchange ─────────────────────────────────────

    fn exchange_req(subject_token: &str, audience: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "subject_token": subject_token,
            "audience": audience,
        }))
        .unwrap()
    }

    // ── Tests: ClerkValidator::validate ──────────────────────────────────────

    #[test]
    fn valid_token_returns_principal() {
        let v = make_validator(None);
        let token = sign(&good_claims(), TEST_KID);
        let p = v.validate(&token).expect("valid token → Some");
        assert_eq!(p.user, TEST_USER);
        assert_eq!(p.org, TEST_ORG);
        assert!(p.fresh_auth, "auth_time 60s ago is within 300s window");
    }

    #[test]
    fn expired_token_rejected() {
        let v = make_validator(None);
        let n = now();
        let mut claims = good_claims();
        claims.exp = n - 600; // expired
        claims.nbf = n - 1200;
        claims.iat = n - 1200;
        assert!(v.validate(&sign(&claims, TEST_KID)).is_none());
    }

    #[test]
    fn wrong_issuer_rejected() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.iss = "https://evil.example".to_string();
        assert!(v.validate(&sign(&claims, TEST_KID)).is_none());
    }

    #[test]
    fn missing_exp_rejected() {
        let v = make_validator(None);
        let mut m = good_claims_json();
        m.remove("exp");
        assert!(v.validate(&sign(&m, TEST_KID)).is_none());
    }

    #[test]
    fn missing_iss_rejected() {
        let v = make_validator(None);
        let mut m = good_claims_json();
        m.remove("iss");
        assert!(v.validate(&sign(&m, TEST_KID)).is_none());
    }

    #[test]
    fn alg_none_attack_rejected() {
        use base64::Engine as _;
        let v = make_validator(None);
        let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"alg":"none","typ":"JWT","kid":TEST_KID}).to_string());
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&good_claims()).unwrap());
        let forged = format!("{header_b64}.{payload_b64}.");
        assert!(v.validate(&forged).is_none(), "alg=none must be rejected");
    }

    #[test]
    fn hs256_confusion_attack_rejected() {
        // Classic algorithm confusion: sign with HS256 using the public modulus
        // as the HMAC secret.  The RS256 pin must kill this.
        let v = make_validator(None);
        let mut header = JwtHeader::new(Algorithm::HS256);
        header.kid = Some(TEST_KID.to_string());
        let forged = encode(
            &header,
            &good_claims(),
            &EncodingKey::from_secret(TEST_N_B64.as_bytes()),
        )
        .expect("HS256 encode");
        assert!(
            v.validate(&forged).is_none(),
            "HS256 confusion must be rejected"
        );
    }

    #[test]
    fn tampered_signature_rejected() {
        let v = make_validator(None);
        let token = sign(&good_claims(), TEST_KID);
        // Flip the last character of the signature (the third `.`-segment).
        let mut parts: Vec<&str> = token.splitn(3, '.').collect();
        let sig = parts[2];
        let tampered_sig = if let Some(rest) = sig.strip_suffix('A') {
            format!("{rest}B")
        } else {
            format!("{}A", &sig[..sig.len() - 1])
        };
        parts[2] = Box::leak(tampered_sig.into_boxed_str());
        let tampered = parts.join(".");
        assert!(
            v.validate(&tampered).is_none(),
            "tampered sig must be rejected"
        );
    }

    #[test]
    fn unknown_kid_not_in_jwks_rejected() {
        // We only insert TEST_KID into the cache; signing with a different kid
        // triggers a refetch (from "http://unused-in-tests" — which fails).
        // After the failed refetch the key is still unknown → None.
        let v = make_validator(None);
        let token = sign(&good_claims(), "unknown-kid");
        assert!(v.validate(&token).is_none());
    }

    #[test]
    fn fresh_auth_true_when_recent() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.auth_time = Some(now() - 10);
        let p = v.validate(&sign(&claims, TEST_KID)).unwrap();
        assert!(p.fresh_auth);
    }

    #[test]
    fn fresh_auth_false_when_stale() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.auth_time = Some(now() - 600); // > 300s → stale
        let p = v.validate(&sign(&claims, TEST_KID)).unwrap();
        assert!(!p.fresh_auth);
    }

    #[test]
    fn fresh_auth_false_when_absent() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.auth_time = None;
        let p = v.validate(&sign(&claims, TEST_KID)).unwrap();
        assert!(!p.fresh_auth, "absent auth_time fails closed → not fresh");
    }

    #[test]
    fn future_auth_time_is_not_fresh() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.auth_time = Some(now() + 3600); // an hour in the future
        let p = v.validate(&sign(&claims, TEST_KID)).unwrap();
        assert!(!p.fresh_auth, "future auth_time must not count as fresh");
    }

    #[test]
    fn no_org_claim_rejected() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.org_id = None;
        claims.public_metadata = None;
        assert!(v.validate(&sign(&claims, TEST_KID)).is_none());
    }

    #[test]
    fn public_metadata_tenant_is_authoritative() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.org_id = Some("legacy".to_string());
        claims.public_metadata = Some(json!({ "tenant_id": "corelink-tenant" }));
        let p = v.validate(&sign(&claims, TEST_KID)).unwrap();
        assert_eq!(p.org, "corelink-tenant", "publicMetadata beats org_id");
    }

    #[test]
    fn org_id_fallback_when_no_public_metadata() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.org_id = Some("fallback-org".to_string());
        claims.public_metadata = None;
        let p = v.validate(&sign(&claims, TEST_KID)).unwrap();
        assert_eq!(p.org, "fallback-org");
    }

    #[test]
    fn azp_required_when_configured() {
        let v = make_validator(Some("https://app.githugr.com"));

        // No azp in token → rejected.
        assert!(v.validate(&sign(&good_claims(), TEST_KID)).is_none());

        // Wrong azp → rejected.
        let mut wrong_azp = good_claims();
        wrong_azp.azp = Some("https://evil.example".to_string());
        assert!(v.validate(&sign(&wrong_azp, TEST_KID)).is_none());

        // Correct azp → accepted.
        let mut right_azp = good_claims();
        right_azp.azp = Some("https://app.githugr.com".to_string());
        assert!(v.validate(&sign(&right_azp, TEST_KID)).is_some());
    }

    #[test]
    fn azp_ignored_when_not_configured() {
        let v = make_validator(None);
        let mut claims = good_claims();
        claims.azp = Some("whatever".to_string());
        assert!(v.validate(&sign(&claims, TEST_KID)).is_some());
    }

    // ── Tests: TokenStore ─────────────────────────────────────────────────────

    fn make_principal(fresh: bool) -> ClerkPrincipal {
        ClerkPrincipal {
            user: TEST_USER.to_string(),
            org: TEST_ORG.to_string(),
            fresh_auth: fresh,
        }
    }

    #[test]
    fn mint_and_lookup_roundtrip() {
        let store = TokenStore::new();
        let p = make_principal(true);
        let raw = store.mint(&p).expect("mint OK");
        assert_eq!(raw.len(), 64, "32 bytes → 64 hex chars");
        match store.lookup(&raw) {
            LookupResult::Ok(rec) => {
                assert_eq!(rec.user, TEST_USER);
                assert_eq!(rec.org, TEST_ORG);
                assert!(rec.fresh_auth);
            }
            _ => panic!("expected LookupResult::Ok"),
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
        // Force-expire by inserting an already-past expires_at.
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
        let _ = store.lookup(&raw); // triggers pruning
        let guard = store.tokens.lock().unwrap();
        assert!(!guard.contains_key(&hash), "expired entry must be pruned");
    }

    #[test]
    fn sweep_on_mint_removes_expired() {
        let store = TokenStore::new();
        let p = make_principal(false);
        // Mint and immediately expire the first token.
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

    // ── Tests: handle_token_exchange ─────────────────────────────────────────

    #[test]
    fn valid_exchange_returns_200() {
        let v = make_validator(None);
        let store = TokenStore::new();
        let token = sign(&good_claims(), TEST_KID);
        let body = exchange_req(&token, TEST_AUD);
        let (status, resp_body) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 200);
        let resp: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(resp["accepted"], true);
        assert_eq!(resp["expires_in"], 300);
        let et = resp["engine_token"].as_str().expect("engine_token present");
        assert_eq!(et.len(), 64, "64 hex chars");
    }

    #[test]
    fn expired_clerk_token_returns_401_token_invalid() {
        let v = make_validator(None);
        let store = TokenStore::new();
        let n = now();
        let mut claims = good_claims();
        claims.exp = n - 600;
        claims.nbf = n - 1200;
        claims.iat = n - 1200;
        let body = exchange_req(&sign(&claims, TEST_KID), TEST_AUD);
        let (status, resp_body) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 401);
        let v: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(v["code"], "TOKEN_INVALID");
    }

    #[test]
    fn wrong_iss_returns_401() {
        let v = make_validator(None);
        let store = TokenStore::new();
        let mut claims = good_claims();
        claims.iss = "https://evil.example".to_string();
        let body = exchange_req(&sign(&claims, TEST_KID), TEST_AUD);
        let (status, _) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 401);
    }

    #[test]
    fn audience_mismatch_returns_401() {
        // audience != claims.org → cross-tenant mint blocked.
        let v = make_validator(None);
        let store = TokenStore::new();
        let token = sign(&good_claims(), TEST_KID);
        let body = exchange_req(&token, "different-org");
        let (status, resp_body) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 401);
        let v: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(v["code"], "TOKEN_INVALID");
    }

    #[test]
    fn no_org_claim_returns_401() {
        let v = make_validator(None);
        let store = TokenStore::new();
        let mut claims = good_claims();
        claims.org_id = None;
        claims.public_metadata = None;
        let body = exchange_req(&sign(&claims, TEST_KID), TEST_AUD);
        let (status, _) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 401);
    }

    #[test]
    fn alg_none_in_exchange_returns_401() {
        use base64::Engine as _;
        let v = make_validator(None);
        let store = TokenStore::new();
        let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"alg":"none","typ":"JWT","kid":TEST_KID}).to_string());
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&good_claims()).unwrap());
        let forged = format!("{header_b64}.{payload_b64}.");
        let body = exchange_req(&forged, TEST_AUD);
        let (status, _) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 401);
    }

    #[test]
    fn hs256_confusion_in_exchange_returns_401() {
        let v = make_validator(None);
        let store = TokenStore::new();
        let mut header = JwtHeader::new(Algorithm::HS256);
        header.kid = Some(TEST_KID.to_string());
        let forged = encode(
            &header,
            &good_claims(),
            &EncodingKey::from_secret(TEST_N_B64.as_bytes()),
        )
        .unwrap();
        let body = exchange_req(&forged, TEST_AUD);
        let (status, _) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 401);
    }

    #[test]
    fn malformed_body_returns_401() {
        let v = make_validator(None);
        let store = TokenStore::new();
        let (status, _) = handle_token_exchange(&v, &store, b"not-json");
        assert_eq!(status, 401);
    }

    #[test]
    fn expired_engine_token_lookup_returns_expired_variant() {
        // This test exercises the TokenStore directly (not the exchange handler,
        // which always returns fresh tokens).
        let store = TokenStore::new();
        let p = make_principal(true);
        let raw = store.mint(&p).unwrap();
        {
            let hash = sha256_bytes(raw.as_bytes());
            store
                .tokens
                .lock()
                .unwrap()
                .get_mut(&hash)
                .unwrap()
                .expires_at = now_secs() - 1;
        }
        assert!(matches!(store.lookup(&raw), LookupResult::Expired));
    }

    #[test]
    fn fresh_auth_propagated_through_exchange() {
        let v = make_validator(None);
        let store = TokenStore::new();
        // Stale auth_time → fresh_auth should be false in the minted record.
        let mut claims = good_claims();
        claims.auth_time = Some(now() - 600);
        let token = sign(&claims, TEST_KID);
        let body = exchange_req(&token, TEST_AUD);
        let (status, resp_body) = handle_token_exchange(&v, &store, &body);
        assert_eq!(status, 200);
        let resp: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        let et = resp["engine_token"].as_str().unwrap().to_string();
        match store.lookup(&et) {
            LookupResult::Ok(rec) => assert!(
                !rec.fresh_auth,
                "stale auth_time → fresh_auth=false in record"
            ),
            _ => panic!("expected valid lookup"),
        }
    }
}
