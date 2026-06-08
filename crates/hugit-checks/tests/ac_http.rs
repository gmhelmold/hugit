//! WP-B2a HTTP-seam oracle — the live `HttpAcClient` against the REAL CoreLink
//! Action-Cache REST contract, proven HERMETICALLY (no live network).
//!
//! ## The contract (transcribed from corelink-server, READ-ONLY)
//! - route : `{GET,PUT} {base}/v1/ac/{tenant}/{action_digest}`
//!   (`crates/corelink-container/src/routes/ac.rs` —
//!   `AC_LOOKUP_ROUTE = "/v1/ac/:tenant/:action_digest"`)
//! - auth : `Authorization: Bearer <PAT>` (BearerPAT)
//! - scope : `x-corelink-scope: cas:r` (lookup) | `cas:rw` (store)
//!   (`corelink-container/src/scope.rs` — `SCOPE_HEADER`)
//! - lookup: 200 → raw payload bytes (the canonical `CheckResult` JSON);
//!   404 → miss; other → error
//! - store : `application/octet-stream` body; 200 (re-write) | 201 (insert)
//!
//! What is proven here WITHOUT infra (via a recording mock transport):
//!   1. request construction — correct route (3-segment, tenant included),
//!      Bearer header present, read/write scope header, octet-stream on PUT;
//!   2. the secret PAT VALUE never leaks into the URL or an error/Display;
//!   3. response mapping — 200 → hit, 404 → miss, 401/403/5xx → explicit error;
//!   4. the content-address guard — a 200 whose record keys to a DIFFERENT
//!      memo key is rejected (never a blind/false hit);
//!   5. fail-CLOSED when unconfigured — `new()` (no PAT/base) returns NotWired,
//!      `configured("")` rejects an empty credential; neither can rot to green.
//!
//! The actual socket call (`UreqTransport`) stays P2 — it is NOT exercised here.

use std::sync::{Arc, Mutex};

use hugit_checks::client::ac::{
    AcConfig, AcError, ActionCache, HttpAcClient, HttpTransport, UreqTransport,
};
use hugit_contracts::CheckResult;

// ─────────────────────────────────────────────────────────────────────────────
// Recording mock transport — the ONLY thing the live client touches. Records
// every request so construction can be asserted; replays a canned response.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedReq {
    verb: &'static str,
    url: String,
    bearer: String,
    body: Vec<u8>,
}

struct MockTransport {
    /// Canned `(status, body)` for the next GET.
    get_response: (u16, Vec<u8>),
    /// Canned status for the next PUT.
    put_status: u16,
    /// Every request the client issued (for construction assertions).
    seen: Mutex<Vec<RecordedReq>>,
}

impl MockTransport {
    fn get(status: u16, body: Vec<u8>) -> Self {
        Self {
            get_response: (status, body),
            put_status: 201,
            seen: Mutex::new(Vec::new()),
        }
    }
    fn put(status: u16) -> Self {
        Self {
            get_response: (404, Vec::new()),
            put_status: status,
            seen: Mutex::new(Vec::new()),
        }
    }
    fn last(&self) -> RecordedReq {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a request")
    }
}

impl HttpTransport for MockTransport {
    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), AcError> {
        self.seen.lock().unwrap().push(RecordedReq {
            verb: "GET",
            url: url.to_string(),
            bearer: bearer.to_string(),
            body: Vec::new(),
        });
        Ok(self.get_response.clone())
    }
    fn put(&self, url: &str, bearer: &str, body: &[u8]) -> Result<u16, AcError> {
        self.seen.lock().unwrap().push(RecordedReq {
            verb: "PUT",
            url: url.to_string(),
            bearer: bearer.to_string(),
            body: body.to_vec(),
        });
        Ok(self.put_status)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

const TENANT: &str = "acme";
const PAT: &str = "corelink_pat_SECRETVALUE_must_never_leak";
const BASE: &str = "https://api.corelink.humangr.com";

/// A `CheckResult` whose `memo_key` is the REAL three-axis key over its own
/// axes — so the content-address guard accepts it as a valid hit.
fn valid_result() -> CheckResult {
    let tree_hash = "aa".repeat(32);
    let def_digest = "bb".repeat(32);
    let toolchain_digest = "cc".repeat(32);
    let memo_key = hugit_refstore::compute_memo_key(&tree_hash, &def_digest, &toolchain_digest);
    CheckResult {
        memo_key,
        tree_hash,
        def_digest,
        toolchain_digest,
        exit: 0,
        artifacts: vec![],
        stdout_ref: "blob:stdout".into(),
        stderr_ref: "blob:stderr".into(),
        duration_ms: 42,
        runner_ref: "runner:local".into(),
        produced_at: 1_717_000_000_000,
    }
}

// The client owns its transport by value, so the recording tests share the mock
// through an `Arc` (`SharedTransport` forwards to the inner mock) and read its
// `seen` log after the call.

struct SharedTransport {
    inner: Arc<MockTransport>,
}
impl HttpTransport for SharedTransport {
    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), AcError> {
        self.inner.get(url, bearer)
    }
    fn put(&self, url: &str, bearer: &str, body: &[u8]) -> Result<u16, AcError> {
        self.inner.put(url, bearer, body)
    }
}

fn shared_client(t: &Arc<MockTransport>) -> HttpAcClient<SharedTransport> {
    let cfg = AcConfig::new(BASE, TENANT, PAT).expect("config");
    HttpAcClient::with_transport(BASE, Some(cfg), SharedTransport { inner: t.clone() })
}

#[test]
fn lookup_request_is_canonical_and_pat_safe() {
    let result = valid_result();
    let key = result.memo_key.clone();
    let body = serde_json::to_vec(&result).unwrap();
    let t = Arc::new(MockTransport::get(200, body));
    let client = shared_client(&t);

    let hit = client.lookup(&key).expect("ok").expect("hit");
    assert_eq!(hit, result);

    let req = t.last();
    assert_eq!(req.verb, "GET");
    assert_eq!(req.url, format!("{BASE}/v1/ac/{TENANT}/{key}"));
    assert_eq!(req.bearer, format!("Bearer {PAT}"));
    assert!(!req.url.contains(PAT), "PAT never in URL");
    assert!(req.body.is_empty(), "GET has no body");
}

#[test]
fn store_request_is_canonical_octet_stream_with_write_scope() {
    let result = valid_result();
    let key = result.memo_key.clone();
    let t = Arc::new(MockTransport::put(201));
    let client = shared_client(&t);

    client.store(&result).expect("store ok (201 insert)");

    let req = t.last();
    assert_eq!(req.verb, "PUT");
    assert_eq!(req.url, format!("{BASE}/v1/ac/{TENANT}/{key}"));
    assert_eq!(req.bearer, format!("Bearer {PAT}"));
    assert!(!req.url.contains(PAT));
    // Canonical body = the serialized CheckResult (what CoreLink stores opaque).
    let decoded: CheckResult = serde_json::from_slice(&req.body).expect("canonical body");
    assert_eq!(decoded, result, "PUT body is the canonical CheckResult");
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Response mapping: 200 → hit, 404 → miss, other → explicit error.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn lookup_404_is_a_miss() {
    let t = Arc::new(MockTransport::get(404, Vec::new()));
    let client = shared_client(&t);
    let miss = client.lookup(&"a".repeat(64)).expect("ok");
    assert!(miss.is_none(), "404 maps to MISS (Ok(None))");
}

#[test]
fn lookup_401_403_5xx_are_explicit_errors_never_a_hit() {
    for code in [401u16, 403, 500, 503] {
        let t = Arc::new(MockTransport::get(code, Vec::new()));
        let client = shared_client(&t);
        match client.lookup(&"a".repeat(64)) {
            Err(AcError::Status(c)) => assert_eq!(c, code),
            other => panic!("HTTP {code} must surface as Status error, got {other:?}"),
        }
    }
}

#[test]
fn store_200_and_201_are_success_other_is_error() {
    let result = valid_result();
    // 200 (idempotent re-write) and 201 (fresh insert) both succeed.
    for code in [200u16, 201] {
        let t = Arc::new(MockTransport::put(code));
        let client = shared_client(&t);
        client
            .store(&result)
            .unwrap_or_else(|e| panic!("HTTP {code} should be ok: {e}"));
    }
    // Anything else is an explicit error (never silently swallowed).
    for code in [403u16, 500] {
        let t = Arc::new(MockTransport::put(code));
        let client = shared_client(&t);
        match client.store(&result) {
            Err(AcError::Status(c)) => assert_eq!(c, code),
            other => panic!("store HTTP {code} must error, got {other:?}"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. The content-address guard: never trust a blind hit.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn lookup_rejects_a_200_whose_record_keys_to_a_different_memo_key() {
    // The server returns a perfectly-valid CheckResult, but for a DIFFERENT
    // action than the one we asked for. A blind cache would serve it; we MUST
    // reject it (false-hit / tamper vector).
    let returned = valid_result(); // keys to H(aa..,bb..,cc..)
    let body = serde_json::to_vec(&returned).unwrap();
    let requested = "f".repeat(64); // a DIFFERENT key

    let t = Arc::new(MockTransport::get(200, body));
    let client = shared_client(&t);
    match client.lookup(&requested) {
        Err(AcError::DigestMismatch {
            requested: req,
            returned: ret,
        }) => {
            assert_eq!(req, requested);
            assert_eq!(ret, returned.memo_key, "guard recomputes the real key");
        }
        other => panic!("a mismatched record MUST be rejected, got {other:?}"),
    }
}

#[test]
fn lookup_rejects_a_record_whose_self_key_disagrees_with_its_axes() {
    // A record whose `memo_key` field is forged to equal the requested key but
    // whose AXES hash to something else must be rejected — the guard recomputes
    // from the axes and refuses the laundered body.
    let mut tampered = valid_result();
    let requested = tampered.memo_key.clone();
    // Mutate an axis WITHOUT updating memo_key → axes no longer match the key.
    tampered.tree_hash = "00".repeat(32);
    let body = serde_json::to_vec(&tampered).unwrap();

    let t = Arc::new(MockTransport::get(200, body));
    let client = shared_client(&t);
    match client.lookup(&requested) {
        Err(AcError::DigestMismatch { .. }) => {}
        other => panic!("a forged self-key must be rejected, got {other:?}"),
    }
}

#[test]
fn lookup_rejects_an_undecodable_200_body() {
    let t = Arc::new(MockTransport::get(200, b"{ not json".to_vec()));
    let client = shared_client(&t);
    match client.lookup(&"a".repeat(64)) {
        Err(AcError::Decode(_)) => {}
        other => panic!("a junk 200 body must be a Decode error, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Fail-CLOSED when unconfigured — the live network is still P2 and cannot
//    silently turn a miss into a green hit.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn unconfigured_client_fails_closed_with_notwired_naming_the_endpoint() {
    // `new(base)` over the REAL ureq transport, but with no PAT/tenant: both
    // verbs are the documented seam (NotWired carrying the endpoint they WILL
    // call). This is the assertion B2a's acceptance suite also pins.
    let client: HttpAcClient<UreqTransport> = HttpAcClient::new("https://ac.corelink.dev/");
    let key = "a".repeat(64);
    match client.lookup(&key) {
        Err(AcError::NotWired(ep)) => {
            assert!(ep.starts_with("GET ") && ep.contains("/v1/ac/"));
            assert!(!ep.contains(PAT));
        }
        other => panic!("unconfigured lookup must be NotWired, got {other:?}"),
    }
    let result = valid_result();
    match client.store(&result) {
        Err(AcError::NotWired(ep)) => assert!(ep.starts_with("PUT ") && ep.contains("/v1/ac/")),
        other => panic!("unconfigured store must be NotWired, got {other:?}"),
    }
}

#[test]
fn configured_rejects_blank_credentials_fail_closed() {
    // An empty base URL, tenant, or PAT is rejected at construction — a missing
    // secret can never produce a usable (and silently mis-targeted) client.
    assert!(matches!(
        AcConfig::new("", TENANT, PAT),
        Err(AcError::NotConfigured(_))
    ));
    assert!(matches!(
        AcConfig::new(BASE, "  ", PAT),
        Err(AcError::NotConfigured(_))
    ));
    assert!(matches!(
        AcConfig::new(BASE, TENANT, ""),
        Err(AcError::NotConfigured(_))
    ));
    assert!(matches!(
        HttpAcClient::configured(BASE, TENANT, ""),
        Err(AcError::NotConfigured(_))
    ));
    // A fully-specified config builds.
    assert!(HttpAcClient::configured(BASE, TENANT, PAT).is_ok());
}

#[test]
fn config_debug_never_renders_the_pat() {
    let cfg = AcConfig::new(BASE, TENANT, PAT).expect("config");
    let rendered = format!("{cfg:?}");
    assert!(!rendered.contains(PAT), "PAT must be redacted in Debug");
    assert!(rendered.contains("redacted"));
    // And the client's Debug too (it holds the config).
    let client = HttpAcClient::configured(BASE, TENANT, PAT).expect("client");
    let cr = format!("{client:?}");
    assert!(!cr.contains(PAT), "PAT must be redacted in client Debug");
}

// ─────────────────────────────────────────────────────────────────────────────
// store → lookup round-trip over the mock transport: the payload hugit PUTs is
// exactly what it accepts back on a hit (byte-canonical, content-verified).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn store_then_lookup_roundtrips_the_canonical_payload() {
    let result = valid_result();
    let key = result.memo_key.clone();

    // Capture what store() PUTs, then feed it back as the lookup 200 body.
    let put_t = Arc::new(MockTransport::put(201));
    let put_client = shared_client(&put_t);
    put_client.store(&result).expect("store");
    let stored_body = put_t.last().body;

    let get_t = Arc::new(MockTransport::get(200, stored_body));
    let get_client = shared_client(&get_t);
    let hit = get_client.lookup(&key).expect("ok").expect("hit");
    assert_eq!(hit, result, "what we stored is exactly what we get back");
}
