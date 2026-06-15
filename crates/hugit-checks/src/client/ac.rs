//! The Action Cache (AC) client surface.
//!
//! The memoized-CI wedge: "your green checks never re-run". A check's
//! [`CheckResult`] is stored in CoreLink's Action Cache under its three-axis
//! [`memo_key`](super::memo_key). On a repeat with the same key the client
//! returns the stored result with ZERO local execution (item ①).
//!
//! ## The trait/seam split (PARTIAL-over-fake is law)
//!
//! The client logic — lookup → hit/miss → store, and crucially the
//! "zero local execution on hit" path — is proven NOW against a real in-process
//! AC ([`InMemoryAc`]) implementing the exact [`ActionCache`] interface the live
//! client uses. The ONLY thing deferred is the live HTTP wiring: [`HttpAcClient`]
//! is a thin, explicitly-marked P2 seam over CoreLink's REST surface
//! (`GET/PUT /v1/ac/{memo_key}`), behind the same trait. When CoreLink's prod AC
//! tenant + PAT auth are available, that seam is filled — the surrounding logic
//! does not change.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hugit_contracts::CheckResult;

/// Errors from an AC interaction. The live client adds transport variants; the
/// in-memory impl never errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcError {
    /// The live HTTP seam is not yet wired (P2). Carries the intended endpoint
    /// so the deferral is self-documenting at the call site.
    NotWired(String),
    /// A transport/protocol failure from the live client (network, TLS, I/O).
    Transport(String),
    /// A RETRYABLE server-busy / contention condition: AC-file lock exhaustion,
    /// or an HTTP 429 (rate-limited) / 503 (service unavailable) from the live
    /// fleet-shared cache. Carried as a TYPED variant — never an ad-hoc string —
    /// so the downstream `map_exec_error` matches it structurally and the
    /// compiler's exhaustiveness check forces every new variant to be classified
    /// retryable-or-terminal (mirrors `LockError::Busy`). The loser of a race
    /// must surface a retryable `ac_busy`, never collapse to terminal `ac_error`.
    Busy {
        /// A short, secret-free description of what was busy (the lock path or
        /// the HTTP status). Never carries a credential.
        detail: String,
    },
    /// The required runtime configuration (tenant PAT and/or base URL) was not
    /// supplied. Fail-closed: the client refuses to make a network call without
    /// a credential rather than silently degrading. This is what an unconfigured
    /// P2 deployment surfaces — it can never rot to a green "hit".
    NotConfigured(String),
    /// The server answered with an HTTP status the protocol does not map to a
    /// hit (200) or miss (404): e.g. 401 (bad PAT), 403 (cross-tenant),
    /// 5xx (server fault). Carries the status for the operator.
    Status(u16),
    /// A 200 hit whose body could not be decoded into a canonical
    /// [`CheckResult`].
    Decode(String),
    /// A 200 hit that decoded but FAILED the content-address guard: the
    /// returned result's memo key (re-derived from its own three axes) does not
    /// match the key we looked up. NEVER trust a blind hit — a mismatch means
    /// the cache returned a record for a different action, so we reject it
    /// instead of serving a false hit. Carries `(requested, returned)`.
    DigestMismatch {
        /// The memo key we asked the cache for.
        requested: String,
        /// The memo key the returned record actually keys to.
        returned: String,
    },
    /// The `memo_key` to be interpolated into the request URL is not a canonical
    /// 64-char lowercase-hex digest (`^[0-9a-f]{64}$`). This is a request-target
    /// trust-boundary guard (defense-in-depth against path traversal /
    /// cross-tenant escape via the action-digest path segment) — distinct from
    /// the content-address guard, which protects the RESPONSE. Carries a short
    /// description of the violation; never the live PAT.
    InvalidKey(String),
}

impl std::fmt::Display for AcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcError::NotWired(ep) => write!(f, "AC HTTP seam not wired (P2): {ep}"),
            AcError::Transport(e) => write!(f, "AC transport error: {e}"),
            AcError::Busy { detail } => write!(f, "AC busy (retryable): {detail}"),
            AcError::NotConfigured(what) => {
                write!(f, "AC client not configured (P2): {what}")
            }
            AcError::Status(code) => write!(f, "AC server returned unexpected HTTP {code}"),
            AcError::Decode(e) => write!(f, "AC hit body decode failed: {e}"),
            AcError::DigestMismatch {
                requested,
                returned,
            } => write!(
                f,
                "AC content-address violation: looked up {requested} but the \
                 returned record keys to {returned} (refusing a blind hit)"
            ),
            AcError::InvalidKey(what) => {
                write!(f, "AC invalid memo key (refusing to build request): {what}")
            }
        }
    }
}

/// A canonical AC memo key is a 64-char lowercase-hex SHA-256 digest. Validate it
/// BEFORE it is interpolated into the request URL: a key carrying path separators
/// (or any non-hex byte) could escape the `/v1/ac/{tenant}/` prefix and target a
/// different tenant or route. Returns [`AcError::InvalidKey`] on any violation.
fn validate_memo_key(memo_key: &str) -> Result<(), AcError> {
    if memo_key.len() == 64
        && memo_key
            .bytes()
            // lowercase hex only: is_ascii_hexdigit would also accept A-F.
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(AcError::InvalidKey(format!(
            "memo key must match ^[0-9a-f]{{64}}$ (got {} chars)",
            memo_key.len()
        )))
    }
}

/// A tenant slug is the first AC path segment and comes from an env var, so it is
/// validated `^[a-z0-9][a-z0-9-]{0,62}$` at config construction. A bad slug could
/// otherwise alter the request route. Returns `false` on any violation.
fn is_valid_tenant_slug(tenant: &str) -> bool {
    let bytes = tenant.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    let is_lower_alnum = |b: u8| b.is_ascii_digit() || b.is_ascii_lowercase();
    if !is_lower_alnum(bytes[0]) {
        return false;
    }
    bytes[1..].iter().all(|&b| is_lower_alnum(b) || b == b'-')
}

impl std::error::Error for AcError {}

/// The Action Cache interface the check client depends on. The live HTTP client
/// and the in-memory fake implement THIS — so the hit/miss/store logic and the
/// zero-execution-on-hit guarantee are exercised identically against both.
pub trait ActionCache {
    /// Look up a memoized [`CheckResult`] by its memo key.
    /// `Ok(Some(_))` = HIT, `Ok(None)` = MISS.
    fn lookup(&self, memo_key: &str) -> Result<Option<CheckResult>, AcError>;

    /// Store a [`CheckResult`] under its `memo_key` (the result carries its own
    /// key). Idempotent: storing the same key twice is a no-op overwrite.
    fn store(&self, result: &CheckResult) -> Result<(), AcError>;
}

// TEST FIXTURE — not for production use; substituting this for HttpAcClient
// silently bypasses CoreLink (no PAT, no tenant scope, no content-address wire
// guard). It ships in the public API only so cross-crate acceptance tests can
// reach it via the public path; `#[doc(hidden)]` keeps it out of the rendered
// docs so it is never mistaken for a production cache backend.
/// A deterministic, in-process Action Cache. NOT a test stub bolted onto the
/// test crate — it is the reference semantics of the AC contract (content-keyed,
/// store-then-hit), so the same code proves the client against it and the live
/// client must match its behavior.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct InMemoryAc {
    /// Guards the map; `Mutex` keeps the impl `Sync` without leaking the lock
    /// into the trait surface.
    store: Mutex<HashMap<String, CheckResult>>,
    /// Total lookups served (for hit-rate / timing evidence).
    lookups: Mutex<u64>,
}

impl InMemoryAc {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many lookups have been served (item ⑤ hit-rate accounting; useful
    /// evidence even though ⑤ is B2b's owned item).
    pub fn lookups(&self) -> u64 {
        *self.lookups.lock().expect("lookups lock poisoned")
    }

    /// How many results are currently memoized.
    pub fn len(&self) -> usize {
        self.store.lock().expect("store lock poisoned").len()
    }

    /// True if no results are memoized.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ActionCache for InMemoryAc {
    fn lookup(&self, memo_key: &str) -> Result<Option<CheckResult>, AcError> {
        *self.lookups.lock().expect("lookups lock poisoned") += 1;
        let hit = self
            .store
            .lock()
            .expect("store lock poisoned")
            .get(memo_key)
            .cloned();
        // Run the content-address guard on the in-memory backend too (WG-CACHE):
        // a record whose memo axes do not key to `memo_key` is a content-address
        // fault, never a silent hit — the same law the HTTP client enforces.
        match hit {
            Some(result) => {
                verify_hit(memo_key, &result)?;
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }

    fn store(&self, result: &CheckResult) -> Result<(), AcError> {
        // PS-10 write-boundary guard — shared across every backend.
        guard_axes_not_secret(result)?;
        self.store
            .lock()
            .expect("store lock poisoned")
            .insert(result.memo_key.clone(), result.clone());
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The live CoreLink Action Cache client (B2a → P2).
//
// hugit CONSUMES CoreLink's AC — it does not reimplement it. The wire contract
// below is transcribed from the CoreLink REST surface (READ-ONLY source):
//
//   - route : `{GET,PUT} {base}/v1/ac/{tenant}/{action_digest}`
//             (`corelink-server/crates/corelink-container/src/routes/ac.rs:62`,
//              `AC_LOOKUP_ROUTE = "/v1/ac/:tenant/:action_digest"`)
//   - auth  : `Authorization: Bearer <PAT>`  (BearerPAT; the edge Worker
//             resolves the PAT → tenant and enforces path-tenant == PAT-tenant)
//             (`apps/docs/.../get-v1-ac-by-tenant-by-action_digest.mdx`)
//   - scope : `x-corelink-scope: cas:r` (lookup) | `cas:rw` (store)
//             (`corelink-container/src/scope.rs:44 SCOPE_HEADER`,
//              fail-CLOSED: missing scope → 403)
//   - lookup: 200 → raw `result_payload` bytes (opaque to CoreLink; hugit
//             stores the canonical `CheckResult` JSON as that payload)
//             404 → miss; 401 bad PAT; 403 cross-tenant/scope
//             (`routes/ac.rs::handle_lookup` → `(OK, resp.result_payload)`;
//              `map_err`: Miss → 404)
//   - store : body = `application/octet-stream` raw payload bytes; idempotent
//             upsert → 200 (already existed) | 201 (fresh insert)
//             (`routes/ac.rs::handle_update`, `put-v1-ac-*.mdx`)
//
// The `{action_digest}` path segment is the content address; for a hugit check
// it is the three-axis `memo_key`. CoreLink stores the payload opaquely keyed by
// `(tenant, action_digest)` and never inspects it
// (`corelink-handler-ac/src/handler.rs::AcLookupResponse.result_payload: Vec<u8>`).
// ─────────────────────────────────────────────────────────────────────────────

/// CoreLink scope header name + values (transcribed from CoreLink, fail-closed).
const SCOPE_HEADER: &str = "x-corelink-scope";
const SCOPE_READ: &str = "cas:r";
const SCOPE_READ_WRITE: &str = "cas:rw";

/// A single HTTP exchange against the CoreLink AC surface — the ONLY part that
/// touches the network. Splitting it behind a trait lets the request building,
/// response parsing, and the content-address hit guard be proven hermetically
/// with a mock transport (no live call), while the real `ureq` transport is the
/// thin P2 seam.
pub trait HttpTransport {
    /// `GET {url}` with a Bearer PAT + read scope. Returns the decoded
    /// `(status, body_bytes)`. `body_bytes` is meaningful on 200.
    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), AcError>;

    /// `PUT {url}` with a Bearer PAT + write scope and an octet-stream body.
    /// Returns the response status.
    fn put(&self, url: &str, bearer: &str, body: &[u8]) -> Result<u16, AcError>;
}

/// Runtime configuration for the live client. Injected at P2 (NOT hardcoded,
/// NOT compiled in): the tenant, the base URL, and the secret PAT. The PAT is
/// held privately and only ever placed in the `Authorization` header — it is
/// never logged, never put in an error/`Display`, never in the endpoint string.
#[derive(Clone)]
pub struct AcConfig {
    /// CoreLink AC base URL (e.g. `https://corelink-api.humangr.com`).
    base_url: String,
    /// The tenant slug — first AC path segment, must match the PAT's tenant.
    tenant: String,
    /// The CoreLink PAT (Bearer). SECRET — never rendered.
    pat: String,
}

impl AcConfig {
    /// Build the config from injected values. Returns [`AcError::NotConfigured`]
    /// (fail-closed) if any field is blank, so an unset deployment surfaces a
    /// clear error and can never silently turn a miss into a green hit.
    pub fn new(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        pat: impl Into<String>,
    ) -> Result<Self, AcError> {
        let base_url = base_url.into();
        let tenant = tenant.into();
        let pat = pat.into();
        if base_url.trim().is_empty() {
            return Err(AcError::NotConfigured("base URL is empty".into()));
        }
        if tenant.trim().is_empty() {
            return Err(AcError::NotConfigured("tenant is empty".into()));
        }
        // The tenant is the first AC path segment and comes from an env var;
        // validate the slug shape (`^[a-z0-9][a-z0-9-]{0,62}$`) so a crafted
        // value cannot alter the request route. Fail-closed, naming the piece —
        // never the PAT.
        if !is_valid_tenant_slug(&tenant) {
            return Err(AcError::NotConfigured(
                "tenant slug must match ^[a-z0-9][a-z0-9-]{0,62}$".into(),
            ));
        }
        if pat.trim().is_empty() {
            return Err(AcError::NotConfigured("PAT is empty".into()));
        }
        Ok(Self {
            base_url,
            tenant,
            pat,
        })
    }

    /// The `Authorization` header value. Crate-private so the secret never
    /// escapes the transport boundary.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.pat)
    }

    /// The full endpoint URL for a memo key (the action-digest path segment).
    ///
    /// The `memo_key` is validated `^[0-9a-f]{64}$` BEFORE interpolation — a key
    /// carrying path separators could escape the `/v1/ac/{tenant}/` prefix, so a
    /// non-canonical key is rejected as [`AcError::InvalidKey`] rather than built
    /// into a request. (The tenant slug is validated once, at construction.)
    fn endpoint(&self, memo_key: &str) -> Result<String, AcError> {
        validate_memo_key(memo_key)?;
        Ok(format!(
            "{}/v1/ac/{}/{}",
            self.base_url.trim_end_matches('/'),
            self.tenant,
            memo_key
        ))
    }
}

// `Debug` is hand-written so the PAT can NEVER leak through `{:?}`.
impl std::fmt::Debug for AcConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcConfig")
            .field("base_url", &self.base_url)
            .field("tenant", &self.tenant)
            .field("pat", &"<redacted>")
            .finish()
    }
}

/// Re-derive a stored [`CheckResult`]'s memo key from its OWN three axes and
/// confirm it equals both the key we requested AND the key the record carries.
///
/// This is the "never trust a blind hit" guard: a cache that returns a record
/// for the wrong action — by bug or by tampering — is rejected instead of being
/// served as a false hit. The key formula is single-sourced via
/// [`hugit_refstore::compute_memo_key`] (never re-transcribed), so this verifies
/// against the exact same content-address the lookup used.
///
/// `pub` (re-exported via the `client::ac` path) so EVERY [`ActionCache`] backend
/// — the HTTP client, the in-memory fake, AND the CLI's file-backed cache — runs
/// the same content-address guard, not just the HTTP path (the WG-CACHE wiring).
/// This is the AXIS-level half of tamper-evidence: it rejects a record whose
/// three memo axes were altered. A flipped `exit`/`ok` leaves the axes (hence the
/// key) intact, so the file backend pairs this with a self-hash over the canonical
/// record bytes to also catch that vector.
pub fn verify_hit(requested: &str, result: &CheckResult) -> Result<(), AcError> {
    let recomputed = hugit_refstore::compute_memo_key(
        &result.tree_hash,
        &result.def_digest,
        &result.toolchain_digest,
    );
    // The record must (a) self-certify (its memo_key field matches its axes) and
    // (b) match the key we asked for. Either mismatch is a content-address fault.
    if recomputed != requested || result.memo_key != requested {
        return Err(AcError::DigestMismatch {
            requested: requested.to_string(),
            returned: recomputed,
        });
    }
    Ok(())
}

/// Refuse to STORE a [`CheckResult`] whose any memo axis (`tree_hash` /
/// `def_digest` / `toolchain_digest`) carries a structural-secret shape (PS-10).
///
/// The axes are content-addresses, persisted UNREDACTED — redacting one would
/// change the recomputed memo key and break every cache HIT ([`verify_hit`]).
/// So a credential smuggled into an axis (the WK-AC vector: a secret-shaped
/// `--toolchain`) cannot be persisted at all — it is refused at the write
/// boundary rather than scrubbed. DENY-BY-DEFAULT: an axis survives only if it
/// PROVES a bounded safe-address shape
/// ([`hugit_ledger::secret_shape::is_safe_identifier_shape`] — the SAME
/// predicate the CLI identifier door uses, so a prefix-less high-entropy
/// credential is caught too). "An exemption is a hole" — all three axes are
/// guarded uniformly (in practice only `toolchain_digest` can carry a
/// user-supplied value; the other two are computed hashes).
///
/// `pub` and called by EVERY [`ActionCache`] backend — the in-memory fake, the
/// HTTP client, AND the CLI's file-backed cache — exactly mirroring how
/// [`verify_hit`] is shared. PS-10 closes the prior gap where this guard lived
/// only on the CLI `FileAc`, leaving the shared trait's contract weaker than one
/// of its implementations.
pub fn guard_axes_not_secret(result: &CheckResult) -> Result<(), AcError> {
    for (field, value) in [
        ("tree_hash", result.tree_hash.as_str()),
        ("def_digest", result.def_digest.as_str()),
        ("toolchain_digest", result.toolchain_digest.as_str()),
    ] {
        if !hugit_ledger::secret_shape::is_safe_identifier_shape(value) {
            return Err(AcError::Transport(format!(
                "AC store refused: memo axis `{field}` carries a structural-secret \
                 shape — an axis is a content-address stored unredacted, so a \
                 credential in it cannot be persisted (it would also be unscrubbable \
                 without busting the cache key)"
            )));
        }
    }
    Ok(())
}

/// Decode + content-verify a 200 lookup body into a [`CheckResult`] HIT.
/// Pure (no I/O): the hit-path logic is fully testable from recorded bytes.
fn parse_hit(requested: &str, body: &[u8]) -> Result<CheckResult, AcError> {
    let result: CheckResult =
        serde_json::from_slice(body).map_err(|e| AcError::Decode(e.to_string()))?;
    verify_hit(requested, &result)?;
    Ok(result)
}

/// Serialize a [`CheckResult`] to its canonical store body (the opaque payload
/// CoreLink persists). Pure (no I/O). Serialization failure is mapped to
/// [`AcError::Decode`] — the only serialization-related error variant available
/// (a `CheckResult` that cannot round-trip through JSON is a structural defect,
/// not just a response decode problem).
fn store_body(result: &CheckResult) -> Result<Vec<u8>, AcError> {
    serde_json::to_vec(result).map_err(|e| AcError::Decode(e.to_string()))
}

/// The live CoreLink Action Cache client over a pluggable [`HttpTransport`].
///
/// The hit/miss/store/verify LOGIC lives here and is proven hermetically (mock
/// transport + recorded fixtures). What stays P2 is supplying a CONFIGURED
/// client (tenant PAT + base URL) at runtime:
///
/// - [`HttpAcClient::new`] builds an UN-configured client (the documented seam):
///   both verbs fail CLOSED with [`AcError::NotWired`] carrying the endpoint
///   they WILL call — the deferral cannot rot to a silent green.
/// - [`HttpAcClient::configured`] is the single P2 wire-up: inject base URL,
///   tenant, and secret PAT and the client speaks the real CoreLink contract.
#[derive(Debug, Clone)]
pub struct HttpAcClient<T: HttpTransport = UreqTransport> {
    /// CoreLink AC base URL recorded for the un-configured SEAM state, so the
    /// `NotWired` error can name the endpoint it will call. `configured` puts
    /// the authoritative copy inside [`AcConfig`].
    base_url: String,
    /// Runtime config (PAT/tenant/base-URL): `None` until injected at P2.
    config: Option<AcConfig>,
    /// The HTTP transport (real `ureq` by default; a mock in tests).
    transport: T,
}

impl HttpAcClient<UreqTransport> {
    /// Construct an UN-configured client bound to a CoreLink AC base URL — the
    /// documented P2 seam. Both verbs return [`AcError::NotWired`] (carrying the
    /// endpoint) until [`Self::configured`] supplies the tenant PAT.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            config: None,
            transport: UreqTransport,
        }
    }

    /// Construct a CONFIGURED client over the real `ureq` transport. This is the
    /// single P2 entry point: inject the base URL, tenant, and secret PAT.
    pub fn configured(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        pat: impl Into<String>,
    ) -> Result<Self, AcError> {
        let base_url = base_url.into();
        Ok(Self {
            base_url: base_url.clone(),
            config: Some(AcConfig::new(base_url, tenant, pat)?),
            transport: UreqTransport,
        })
    }
}

impl<T: HttpTransport> HttpAcClient<T> {
    /// Construct a client over an explicit transport + config. Used in tests to
    /// inject a mock transport; in production `T = UreqTransport`.
    pub fn with_transport(
        base_url: impl Into<String>,
        config: Option<AcConfig>,
        transport: T,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            config,
            transport,
        }
    }
}

impl<T: HttpTransport> ActionCache for HttpAcClient<T> {
    fn lookup(&self, memo_key: &str) -> Result<Option<CheckResult>, AcError> {
        let Some(cfg) = self.config.as_ref() else {
            // Un-configured: the documented seam. Fail CLOSED, naming the call.
            return Err(AcError::NotWired(format!(
                "GET {}/v1/ac/{}",
                self.base_url.trim_end_matches('/'),
                memo_key
            )));
        };
        let url = cfg.endpoint(memo_key)?;
        let (status, body) = self.transport.get(&url, &cfg.bearer())?;
        match status {
            200 => Ok(Some(parse_hit(memo_key, &body)?)),
            404 => Ok(None),
            // 429 (rate-limited) / 503 (unavailable) are RETRYABLE server-busy
            // conditions from the live fleet-shared cache — classify them as the
            // typed `Busy` variant so the loser of a race retries instead of
            // collapsing to a terminal `ac_error`.
            429 | 503 => Err(AcError::Busy {
                detail: format!("AC GET returned HTTP {status}"),
            }),
            other => Err(AcError::Status(other)),
        }
    }

    fn store(&self, result: &CheckResult) -> Result<(), AcError> {
        // PS-10 write-boundary guard — shared across every backend, enforced
        // before the network PUT so a secret-shaped axis never leaves the box.
        guard_axes_not_secret(result)?;
        let Some(cfg) = self.config.as_ref() else {
            return Err(AcError::NotWired(format!(
                "PUT {}/v1/ac/{}",
                self.base_url.trim_end_matches('/'),
                result.memo_key
            )));
        };
        let url = cfg.endpoint(&result.memo_key)?;
        let body = store_body(result)?;
        let status = self.transport.put(&url, &cfg.bearer(), &body)?;
        match status {
            // 200 = idempotent re-write, 201 = fresh insert — both are success.
            200 | 201 => Ok(()),
            // 429 / 503 = retryable server-busy (see `lookup`).
            429 | 503 => Err(AcError::Busy {
                detail: format!("AC PUT returned HTTP {status}"),
            }),
            other => Err(AcError::Status(other)),
        }
    }
}

/// The real `ureq`-backed transport — the thin network seam. This is the ONLY
/// code that opens a socket; everything around it is proven without it.
///
/// `ureq` is already in the workspace lock (used by `hugit-queue`), so no new
/// transitive surface is added.
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqTransport;

impl HttpTransport for UreqTransport {
    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), AcError> {
        let resp = ureq::get(url)
            .set("Authorization", bearer)
            .set(SCOPE_HEADER, SCOPE_READ)
            .call();
        match resp {
            Ok(r) => {
                let status = r.status();
                let mut buf = Vec::new();
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| AcError::Transport(e.to_string()))?;
                Ok((status, buf))
            }
            // ureq surfaces non-2xx as `Error::Status(code, resp)`; map it to a
            // status the protocol logic can branch on (esp. 404 → miss).
            Err(ureq::Error::Status(code, _resp)) => Ok((code, Vec::new())),
            Err(e) => Err(AcError::Transport(e.to_string())),
        }
    }

    fn put(&self, url: &str, bearer: &str, body: &[u8]) -> Result<u16, AcError> {
        let resp = ureq::put(url)
            .set("Authorization", bearer)
            .set(SCOPE_HEADER, SCOPE_READ_WRITE)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(body);
        match resp {
            Ok(r) => Ok(r.status()),
            Err(ureq::Error::Status(code, _resp)) => Ok(code),
            Err(e) => Err(AcError::Transport(e.to_string())),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The runtime config LOADER (P2 plug-and-play) — make the CoreLink AC seam
// plug-and-play once the tenant + PAT exist, WITHOUT compiling any of them in.
//
// Names are the binding §5 delivery contract:
//   - HUGIT_CORELINK_AC_URL  — the AC base URL (e.g. https://corelink-api.humangr.com)
//   - HUGIT_CORELINK_TENANT  — the tenant slug (first AC path segment)
//   - the PAT, read from the FILE `~/.hugit/secrets/corelink/pat` (preferred),
//     falling back to the `HUGIT_CORELINK_PAT` env var ONLY if that file is absent.
//
// Fail-closed is LAW: a missing/blank piece returns `AcError::NotConfigured`
// NAMING which piece is missing — and never the value. The PAT is read into the
// private `AcConfig` and from there only ever placed in the `Authorization`
// header; it never appears in Debug/Display/logs/errors.
// ─────────────────────────────────────────────────────────────────────────────

/// Env var holding the CoreLink AC base URL.
pub const ENV_AC_URL: &str = "HUGIT_CORELINK_AC_URL";
/// Env var holding the CoreLink tenant slug.
pub const ENV_TENANT: &str = "HUGIT_CORELINK_TENANT";
/// Env var holding the CoreLink PAT — the FALLBACK source, used only when the
/// secret file is absent.
pub const ENV_PAT: &str = "HUGIT_CORELINK_PAT";
/// Env var overriding the PAT secret-file path (for hermetic tests; production
/// uses the default handoff path under `~/.hugit`).
pub const ENV_PAT_FILE: &str = "HUGIT_CORELINK_PAT_FILE";

/// The production default PAT secret-file path, relative to `$HOME`.
/// Joined with `$HOME` so the absolute path is `~/.hugit/secrets/corelink/pat`.
const DEFAULT_PAT_FILE_REL: &str = ".hugit/secrets/corelink/pat";

/// Resolve the PAT secret-file path: an explicit `HUGIT_CORELINK_PAT_FILE`
/// override (tests point this at a temp file) wins; otherwise the production
/// default `~/.hugit/secrets/corelink/pat` resolved against `$HOME`.
///
/// Returns `NotConfigured` only when neither an override nor `$HOME` is set
/// (so the loader cannot silently fall through to "no file" on a broken env).
fn resolve_pat_file() -> Result<PathBuf, AcError> {
    if let Ok(p) = std::env::var(ENV_PAT_FILE)
        && !p.trim().is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME").map_err(|_| {
        AcError::NotConfigured(format!(
            "PAT: no {ENV_PAT_FILE} override and $HOME is unset (cannot locate the \
             default ~/{DEFAULT_PAT_FILE_REL})"
        ))
    })?;
    Ok(Path::new(&home).join(DEFAULT_PAT_FILE_REL))
}

/// Read the PAT, preferring the secret file at `pat_file` (trimming a trailing
/// newline) and falling back to the `HUGIT_CORELINK_PAT` env var ONLY if the
/// file is absent. Returns the secret string on success; on every failure path
/// returns `NotConfigured` whose message names the missing piece but NEVER the
/// value. A present-but-blank file/env is treated as missing (fail-closed).
fn read_pat(pat_file: &Path) -> Result<String, AcError> {
    match std::fs::read_to_string(pat_file) {
        Ok(contents) => {
            // A secret file must not be readable/writable by group or other.
            // On Unix, reject any mode with `& 0o077 != 0`, naming the FILE PATH
            // (never the value): a world-readable PAT is a credential leak, so we
            // fail closed rather than load it.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let meta = std::fs::metadata(pat_file).map_err(|e| {
                    AcError::NotConfigured(format!(
                        "PAT: cannot stat secret file {}: {}",
                        pat_file.display(),
                        e.kind()
                    ))
                })?;
                let mode = meta.permissions().mode();
                if mode & 0o077 != 0 {
                    return Err(AcError::NotConfigured(format!(
                        "PAT: secret file {} has insecure permissions {:o} \
                         (group/other access); chmod 600 it",
                        pat_file.display(),
                        mode & 0o777
                    )));
                }
            }
            // Trim only trailing newline(s)/whitespace — a secret never has
            // meaningful trailing whitespace, and editors append a newline.
            let pat = contents.trim_end().to_string();
            if pat.is_empty() {
                return Err(AcError::NotConfigured(format!(
                    "PAT: secret file {} is empty",
                    pat_file.display()
                )));
            }
            Ok(pat)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File absent → fall back to the env var (the ONLY fallback).
            match std::env::var(ENV_PAT) {
                Ok(v) if !v.trim().is_empty() => Ok(v),
                _ => Err(AcError::NotConfigured(format!(
                    "PAT: secret file {} absent and {ENV_PAT} unset/empty",
                    pat_file.display()
                ))),
            }
        }
        // Any other I/O error (permissions, etc.) is surfaced fail-closed, named
        // by piece — never the value (there is none to leak here anyway).
        Err(e) => Err(AcError::NotConfigured(format!(
            "PAT: cannot read secret file {}: {}",
            pat_file.display(),
            e.kind()
        ))),
    }
}

impl HttpAcClient<UreqTransport> {
    /// Build a CONFIGURED client from the runtime environment — the P2
    /// plug-and-play entry point. Reads EXACTLY:
    ///
    /// - `HUGIT_CORELINK_AC_URL` — the AC base URL,
    /// - `HUGIT_CORELINK_TENANT` — the tenant slug,
    /// - the PAT from `~/.hugit/secrets/corelink/pat` (preferred; trailing
    ///   newline trimmed), falling back to the `HUGIT_CORELINK_PAT` env var ONLY
    ///   if that file is absent. The file path is overridable via
    ///   `HUGIT_CORELINK_PAT_FILE` (for hermetic tests).
    ///
    /// Returns a configured [`HttpAcClient`] over the real [`UreqTransport`] when
    /// all three pieces are present and non-empty; otherwise returns
    /// [`AcError::NotConfigured`] whose message NAMES the missing piece (never
    /// the value). Fail-closed: an unset deployment can never produce a usable
    /// (silently mis-targeted) client, and the PAT is held only inside the
    /// private [`AcConfig`] — never logged, never in Debug/Display/errors.
    pub fn from_runtime() -> Result<Self, AcError> {
        corelink_ac_from_env()
    }
}

/// Free-function form of [`HttpAcClient::from_runtime`] — reads the CoreLink AC
/// runtime config from the environment and returns a configured client (real
/// `ureq` transport) or [`AcError::NotConfigured`] naming the missing piece.
///
/// See [`HttpAcClient::from_runtime`] for the exact env/path contract.
pub fn corelink_ac_from_env() -> Result<HttpAcClient<UreqTransport>, AcError> {
    // Base URL — present + non-empty, else NotConfigured naming it.
    let base_url = match std::env::var(ENV_AC_URL) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            return Err(AcError::NotConfigured(format!(
                "base URL: {ENV_AC_URL} unset/empty"
            )));
        }
    };
    // Tenant — present + non-empty, else NotConfigured naming it.
    let tenant = match std::env::var(ENV_TENANT) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            return Err(AcError::NotConfigured(format!(
                "tenant: {ENV_TENANT} unset/empty"
            )));
        }
    };
    // PAT — file-preferred, env-fallback; NotConfigured naming it on any miss.
    let pat_file = resolve_pat_file()?;
    let pat = read_pat(&pat_file)?;

    // All three present → build the configured client over the real transport.
    // `AcConfig::new` re-validates non-emptiness (defense in depth) and holds the
    // PAT privately; the PAT only ever leaves via the Authorization header.
    HttpAcClient::configured(base_url, tenant, pat)
}
