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
use std::sync::Mutex;

use hugit_contracts::CheckResult;

/// Errors from an AC interaction. The live client adds transport variants; the
/// in-memory impl never errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcError {
    /// The live HTTP seam is not yet wired (P2). Carries the intended endpoint
    /// so the deferral is self-documenting at the call site.
    NotWired(String),
    /// A transport/protocol failure from the live client (reserved for the
    /// HTTP seam).
    Transport(String),
}

impl std::fmt::Display for AcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcError::NotWired(ep) => write!(f, "AC HTTP seam not wired (P2): {ep}"),
            AcError::Transport(e) => write!(f, "AC transport error: {e}"),
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

/// The live CoreLink Action Cache client — the P2 HTTP seam.
///
/// CoreLink is consumed as a CLIENT only (zero server changes). The protocol is
/// `clw run`'s AC surface, generalized:
///
/// - lookup: `GET  {base}/v1/ac/{memo_key}` → 200 + CheckResult JSON | 404
/// - store:  `PUT  {base}/v1/ac/{memo_key}` with the CheckResult JSON body
///
/// Auth is the CoreLink PAT (consumed, not minted here).
///
/// # P2 SEAM — DEFERRED
/// The transport bodies are intentionally NOT wired in B2a: the live AC tenant
/// (item ①'s "<500ms real AC hit") needs the CoreLink prod tenant + PAT, which
/// is out of this WP's hermetic scope. Both methods return [`AcError::NotWired`]
/// with the endpoint they WILL call. The client logic that consumes this trait
/// is fully proven against [`InMemoryAc`]; only these two bodies remain.
#[derive(Debug, Clone)]
pub struct HttpAcClient {
    /// CoreLink AC base URL (e.g. `https://ac.corelink.dev`).
    base_url: String,
}

impl HttpAcClient {
    /// Construct a client bound to a CoreLink AC base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// The endpoint a given memo key resolves to (used by both verbs).
    fn endpoint(&self, memo_key: &str) -> String {
        format!("{}/v1/ac/{}", self.base_url.trim_end_matches('/'), memo_key)
    }
}

impl ActionCache for HttpAcClient {
    fn lookup(&self, memo_key: &str) -> Result<Option<CheckResult>, AcError> {
        // P2: wire `GET {endpoint}` here; 200 → Some(parse body), 404 → None.
        Err(AcError::NotWired(format!(
            "GET {}",
            self.endpoint(memo_key)
        )))
    }

    fn store(&self, result: &CheckResult) -> Result<(), AcError> {
        // P2: wire `PUT {endpoint}` with the CheckResult JSON body here.
        Err(AcError::NotWired(format!(
            "PUT {}",
            self.endpoint(&result.memo_key)
        )))
    }
}
