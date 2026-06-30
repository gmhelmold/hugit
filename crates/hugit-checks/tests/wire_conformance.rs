//! Hugit-side wire-conformance fixtures for the runner-fabric response shapes.
//!
//! These pin the EXACT JSON shapes hugit's lease client must parse off — AND the
//! acquire REQUEST body it must SEND to — the live CoreLink Runners fabric, so the
//! transcription-drift class that produced the `acquire` wrapper bug (#204), the
//! `close` empty-body bug (#205), and the acquire-REQUEST `unknown field
//! principal_chain` 422 (this change) cannot silently recur. Each fixture is deserialized into hugit's transcribed
//! type and asserted field-for-field; an unannounced field rename / type / shape
//! drift on the fabric breaks these tests, BEFORE any live call.
//!
//! The authoritative shapes are the frozen fabric DTOs
//! (`corelink-fabric-api::dto::{AcquireResponse, CloseResponse, EnvelopeIngest}`)
//! and the §13.1 `IntentMetrics` table of the integration contract.
//!
//! NOTE — cross-repo joint pass (DONE 2026-06-30): the BYTE-frozen cross-repo
//! vectors for all FOUR lease DTOs (`conformance/{AcquireRequest,AcquireResponse,
//! CloseRequest,CloseResponse}.json`) are now committed byte-identical in both repos
//! (Runners TL published #230; hugit mirrored + sha-verified in
//! `conformance/manifest.sha256`). The `canonical_*_subset_round_trips` tests at the
//! bottom of this file pin them on hugit's side (subset round-trip — see the note
//! there for why subset, not blanket byte-identical). The `fixtures/wire/*` tests
//! above remain the per-shape parse assertions.

use std::path::PathBuf;

use hugit_checks::runner::{AcquireLeaseRequest, AcquireResponse, CloseRequest, CloseResponse};
use hugit_contracts::{IntentMetrics, RunnerState};

/// Read a wire fixture under `tests/fixtures/wire/`.
fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wire")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// The acquire REQUEST hugit SENDS serializes to EXACTLY the four keys the frozen
/// `deny_unknown_fields` fabric `AcquireRequest` accepts
/// (`{image_digest, net_policy, tmp_root, expiry_ms}`) and carries NONE of the
/// old/never-live fields (`principal_chain` / `path_set` / `ttl_ms`) that made a
/// real acquire `422 unknown field principal_chain`. The canonical fabric body
/// (`AcquireRequest.json`) pins the key-set so this drift class cannot recur.
#[test]
fn acquire_request_serializes_to_exactly_the_fabric_keys() {
    // The canonical fabric body — the authoritative key-set hugit must produce.
    let fabric: serde_json::Value = serde_json::from_str(&fixture("AcquireRequest.json")).unwrap();
    let mut fabric_keys: Vec<&str> = fabric
        .as_object()
        .expect("the canonical fabric AcquireRequest is a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    fabric_keys.sort_unstable();
    assert_eq!(
        fabric_keys,
        vec!["expiry_ms", "image_digest", "net_policy", "tmp_root"],
        "the fixture pins the frozen fabric key-set"
    );

    // hugit's transcribed request must serialize to EXACTLY that key-set.
    let req = AcquireLeaseRequest {
        image_digest: "alpine@sha256:d9e853af2c8e".to_string(),
        net_policy: "isolated".to_string(),
        tmp_root: "/work/tmp".to_string(),
        expiry_ms: 60_000,
    };
    let value = serde_json::to_value(&req).unwrap();
    let obj = value.as_object().expect("acquire request → JSON object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, fabric_keys,
        "hugit's acquire body must carry ONLY the four frozen fabric keys"
    );
    for retired in ["principal_chain", "path_set", "ttl_ms"] {
        assert!(
            !obj.contains_key(retired),
            "the retired field `{retired}` must never serialize (it 422s the fabric)"
        );
    }

    // It also round-trips back through hugit's transcribed type from the canonical
    // fabric body (liberal-in: hugit parses what the fabric accepts).
    let parsed: AcquireLeaseRequest =
        serde_json::from_str(&fixture("AcquireRequest.json")).unwrap();
    assert_eq!(parsed.net_policy, "isolated");
    assert_eq!(parsed.tmp_root, "/work/tmp");
    assert_eq!(parsed.expiry_ms, 60_000);
    assert!(parsed.image_digest.contains("sha256:"));
}

/// The `AcquireResponse` WRAPPER for a CHECK lease deserializes byte-faithfully:
/// the inner `RunnerLease`, the `exec_endpoint`, AND the §13.2 `envelope_ingest`
/// credential (present for check leases) all parse.
#[test]
fn acquire_response_check_lease_parses() {
    let resp: AcquireResponse =
        serde_json::from_str(&fixture("AcquireResponse_check_lease.json")).unwrap();

    assert_eq!(resp.lease.lease_id, "lease-0001-held");
    assert_eq!(resp.lease.state, RunnerState::Held);
    assert_eq!(resp.lease.net_policy, "egress-deny-all");
    assert_eq!(resp.lease.principal_chain.len(), 2);
    assert_eq!(resp.exec_endpoint, "/v1/leases/lease-0001-held/exec");

    let ingest = resp
        .envelope_ingest
        .as_ref()
        .expect("a check lease carries the §13.2 ingest credential");
    assert_eq!(
        ingest.ingest_path,
        "/v1/leases/lease-0001-held/envelope/ingest"
    );
    assert_eq!(
        ingest.credential,
        "scoped-write-only-ingest-token-do-not-leak"
    );
}

/// The `AcquireResponse` WRAPPER for a RUNNER lease (no §13 ingest) deserializes
/// byte-faithfully: the absent `envelope_ingest` defaults to `None` (hugit is
/// liberal — a runner-lease response parses identically to before the field
/// existed), never a decode error.
#[test]
fn acquire_response_runner_lease_parses_without_ingest() {
    let resp: AcquireResponse =
        serde_json::from_str(&fixture("AcquireResponse_runner_lease.json")).unwrap();

    assert_eq!(resp.lease.lease_id, "lease-0002-held");
    assert_eq!(resp.exec_endpoint, "/v1/leases/lease-0002-held/exec");
    assert!(
        resp.envelope_ingest.is_none(),
        "a runner lease has no §13.2 ingest credential"
    );
}

/// The `CloseResponse` deserializes byte-faithfully — the load-bearing `metrics`
/// (the finalized §13.1 figure) maps into the frozen `IntentMetrics` with the
/// FULL token cache-split preserved, and the fabric extras (`attestation`, the
/// result-binding sigs, `fabric_key_id`, echoed `check_result`) are
/// tolerated-and-ignored (hugit is liberal — no `deny_unknown_fields`).
#[test]
fn close_response_parses_and_metrics_map_to_intent_metrics() {
    let resp: CloseResponse = serde_json::from_str(&fixture("CloseResponse.json")).unwrap();

    assert_eq!(resp.lease_id, "lease-0001-held");
    assert!(resp.released);
    assert!(!resp.capture_incomplete);

    // The §13.1 metrics map into the frozen IntentMetrics, cache-split intact.
    let im: IntentMetrics = resp.metrics.clone().into_intent_metrics();
    assert_eq!(im.tokens.input, 48211);
    assert_eq!(im.tokens.output, 9143);
    assert_eq!(im.tokens.cache_read, 120557);
    assert_eq!(im.tokens.cache_write, 3361);
    assert_eq!(im.tokens.total, 181272);
    assert_eq!(im.wall_ms, 754000);
    assert_eq!(im.active_ms, 612450);
    assert_eq!(im.tool_calls, 41);
    assert_eq!(im.model_turns, 58);
    assert_eq!(im.cost_usd_micros, 1834290);
    assert_eq!(im.tool_breakdown.len(), 3);
    assert_eq!(im.tool_breakdown[0].tool, "Bash");
    assert_eq!(im.tool_breakdown[0].count, 17);
}

/// The `CloseRequest` hugit SENDS serializes EXACTLY to the keys the frozen
/// `deny_unknown_fields` fabric `CloseRequest` accepts — and crucially OMITS
/// `cost_usd_micros` when `None` (the `skip_serializing_if` invariant). This locks
/// the #64 submit-side contract: the body stays byte-identical to today's
/// `{"status":"…"}` for a `None` cost (accepted by BOTH the not-yet-redeployed
/// fabric with no such field AND the new one that `#[serde(default)]`s it), and
/// carries the EXACT `cost_usd_micros` key when `Some`. A regression that emitted
/// the field as `null` (or under a wrong key) would 400 the `deny_unknown_fields`
/// fabric — this test fails first.
#[test]
fn close_request_omits_cost_when_none_and_includes_it_when_some() {
    // (a) None → EXACTLY `{"status":"succeeded"}` (the skip works; byte-identical
    // to today's body, no `cost_usd_micros` key, never a `null`).
    let none = CloseRequest {
        status: "succeeded".to_string(),
        cost_usd_micros: None,
    };
    let none_json = serde_json::to_string(&none).unwrap();
    assert_eq!(
        none_json, r#"{"status":"succeeded"}"#,
        "None must omit cost_usd_micros — byte-identical to today's status-only body"
    );
    let none_obj: serde_json::Value = serde_json::from_str(&none_json).unwrap();
    assert!(
        none_obj
            .as_object()
            .unwrap()
            .get("cost_usd_micros")
            .is_none(),
        "the cost key must NOT appear when None (deny_unknown_fields safety)"
    );

    // (b) Some(4_200_000) → carries the EXACT `cost_usd_micros` key + value, the
    // shape the canonical fabric fixture pins.
    let some = CloseRequest {
        status: "succeeded".to_string(),
        cost_usd_micros: Some(4_200_000),
    };
    let some_value = serde_json::to_value(&some).unwrap();
    let fabric: serde_json::Value =
        serde_json::from_str(&fixture("CloseRequest_with_cost.json")).unwrap();
    assert_eq!(
        some_value, fabric,
        "Some(n) must serialize to EXACTLY the canonical fabric CloseRequest body"
    );
    assert_eq!(some_value["status"], "succeeded");
    assert_eq!(some_value["cost_usd_micros"], 4_200_000);
}

/// A `CloseResponse` MUST carry `metrics` — §13.1 "never optional when the job
/// succeeded". A metrics-less body is a decode error, never a fabricated zero
/// (the honesty law: hugit never invents the attested figure).
#[test]
fn close_response_without_metrics_is_a_decode_error() {
    let no_metrics = r#"{"lease_id": "lease-x", "released": true, "capture_incomplete": false}"#;
    assert!(
        serde_json::from_str::<CloseResponse>(no_metrics).is_err(),
        "a metrics-less close must fail closed, never default to zero"
    );
}

// ── The CANONICAL byte-frozen lease DTO vectors (drift tripwire, lockstep with
//    corelink-runners) ──────────────────────────────────────────────────────────
//
// `conformance/{AcquireRequest,AcquireResponse,CloseRequest,CloseResponse}.json` are
// copied BYTE-IDENTICAL from `corelink-runners/conformance/` (#230; sha256 in
// `conformance/manifest.sha256`, matched against the Runners TL's published hashes).
// These pin the wire SHAPE so the 3-wire-drift history (acquire-req/-resp/close) ends.
//
// NOTE on the assertion: hugit's CLIENT DTOs are intentional SUBSETS of the full
// fabric DTOs — `AcquireLeaseRequest` omits `runner`/`toolchain_digest` (the classic
// off-box path never sets them), `CloseRequest` omits `check_result`, and
// `CloseResponse` captures only the fields hugit consumes (`lease_id`/`released`/
// `capture_incomplete`/`metrics`), liberally ignoring the attestation block. A blanket
// byte-identical re-serialize would therefore FALSELY fail on those subsets. The
// correct tripwire is SUBSET round-trip equality: deserialize the canonical vector into
// hugit's DTO, re-serialize, and assert EVERY key hugit produces equals the canonical's
// value for that key. A renamed/retyped/restructured field hugit consumes trips this; a
// field hugit deliberately doesn't carry is simply absent from its output (not asserted).

/// Read a canonical byte-frozen vector from the repo-root `conformance/` dir.
fn conformance(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read conformance vector {}: {e}", path.display()))
}

/// Assert every key hugit's DTO PRODUCES round-trips to exactly the canonical value
/// (subset equality — hugit may omit fabric-only optional fields, but must never drift
/// on a field it carries).
fn assert_subset_roundtrip(produced: &serde_json::Value, canonical_src: &str) {
    let canonical: serde_json::Value = serde_json::from_str(canonical_src).unwrap();
    for (key, val) in produced
        .as_object()
        .expect("hugit DTO serializes to an object")
    {
        assert_eq!(
            val, &canonical[key],
            "hugit drifts from the canonical fabric vector on key `{key}`: \
             hugit={val} canonical={}",
            canonical[key]
        );
    }
}

#[test]
fn canonical_acquire_request_subset_round_trips() {
    let src = conformance("AcquireRequest.json");
    let dto: AcquireLeaseRequest = serde_json::from_str(&src).expect("hugit parses canonical");
    assert_subset_roundtrip(&serde_json::to_value(&dto).unwrap(), &src);
}

#[test]
fn canonical_acquire_response_subset_round_trips() {
    let src = conformance("AcquireResponse.json");
    let dto: AcquireResponse = serde_json::from_str(&src).expect("hugit parses canonical");
    assert_subset_roundtrip(&serde_json::to_value(&dto).unwrap(), &src);
    // The §13.2 off-box ingest credential round-trips (the field the cost-killer needs).
    assert!(
        dto.envelope_ingest.is_some(),
        "the canonical AcquireResponse exercises envelope_ingest; hugit must capture it"
    );
}

#[test]
fn canonical_close_request_subset_round_trips() {
    let src = conformance("CloseRequest.json");
    let dto: CloseRequest = serde_json::from_str(&src).expect("hugit parses canonical");
    assert_subset_roundtrip(&serde_json::to_value(&dto).unwrap(), &src);
    // The provider-billed cost round-trips verbatim (the #64 submit field).
    assert_eq!(dto.cost_usd_micros, Some(4_200_000));
}

#[test]
fn canonical_close_response_subset_round_trips() {
    let src = conformance("CloseResponse.json");
    let dto: CloseResponse = serde_json::from_str(&src).expect("hugit parses canonical");
    assert_subset_roundtrip(&serde_json::to_value(&dto).unwrap(), &src);
    // hugit consumes the finalized metrics (incl. the attested cost); the attestation
    // block is liberally ignored on this DTO (a separate verification path owns sigs).
    assert_eq!(dto.metrics.cost_usd_micros, 4_200_000);
}
