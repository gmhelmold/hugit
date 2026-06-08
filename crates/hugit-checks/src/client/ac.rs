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
}

impl std::fmt::Display for AcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcError::NotWired(ep) => write!(f, "AC HTTP seam not wired (P2): {ep}"),
            AcError::Transport(e) => write!(f, "AC transport error: {e}"),
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
        }
    }
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

/// A deterministic, in-process Action Cache. NOT a test stub bolted onto the
/// test crate — it is the reference semantics of the AC contract (content-keyed,
/// store-then-hit), so the same code proves the client against it and the live
/// client must match its behavior.
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
        Ok(self
            .store
            .lock()
            .expect("store lock poisoned")
            .get(memo_key)
            .cloned())
    }

    fn store(&self, result: &CheckResult) -> Result<(), AcError> {
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
    /// CoreLink AC base URL (e.g. `https://api.corelink.humangr.com`).
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
    fn endpoint(&self, memo_key: &str) -> String {
        format!(
            "{}/v1/ac/{}/{}",
            self.base_url.trim_end_matches('/'),
            self.tenant,
            memo_key
        )
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
fn verify_hit(requested: &str, result: &CheckResult) -> Result<(), AcError> {
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

/// Decode + content-verify a 200 lookup body into a [`CheckResult`] HIT.
/// Pure (no I/O): the hit-path logic is fully testable from recorded bytes.
fn parse_hit(requested: &str, body: &[u8]) -> Result<CheckResult, AcError> {
    let result: CheckResult =
        serde_json::from_slice(body).map_err(|e| AcError::Decode(e.to_string()))?;
    verify_hit(requested, &result)?;
    Ok(result)
}

/// Serialize a [`CheckResult`] to its canonical store body (the opaque payload
/// CoreLink persists). Pure (no I/O).
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
        let url = cfg.endpoint(memo_key);
        let (status, body) = self.transport.get(&url, &cfg.bearer())?;
        match status {
            200 => Ok(Some(parse_hit(memo_key, &body)?)),
            404 => Ok(None),
            other => Err(AcError::Status(other)),
        }
    }

    fn store(&self, result: &CheckResult) -> Result<(), AcError> {
        let Some(cfg) = self.config.as_ref() else {
            return Err(AcError::NotWired(format!(
                "PUT {}/v1/ac/{}",
                self.base_url.trim_end_matches('/'),
                result.memo_key
            )));
        };
        let url = cfg.endpoint(&result.memo_key);
        let body = store_body(result)?;
        let status = self.transport.put(&url, &cfg.bearer(), &body)?;
        match status {
            // 200 = idempotent re-write, 201 = fresh insert — both are success.
            200 | 201 => Ok(()),
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
