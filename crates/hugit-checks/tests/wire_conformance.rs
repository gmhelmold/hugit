//! Hugit-side wire-conformance fixtures for the runner-fabric response shapes.
//!
//! These pin the EXACT JSON shapes hugit's lease client must parse off the live
//! CoreLink Runners fabric, so the transcription-drift class that produced the
//! `acquire` wrapper bug (#204) and the `close` empty-body bug (this change)
//! cannot silently recur. Each fixture is deserialized into hugit's transcribed
//! type and asserted field-for-field; an unannounced field rename / type / shape
//! drift on the fabric breaks these tests, BEFORE any live call.
//!
//! The authoritative shapes are the frozen fabric DTOs
//! (`corelink-fabric-api::dto::{AcquireResponse, CloseResponse, EnvelopeIngest}`)
//! and the §13.1 `IntentMetrics` table of the integration contract.
//!
//! NOTE — cross-repo joint pass (Runners TL's lane): the BYTE-frozen cross-repo
//! vectors (`corelink-runners/conformance/AcquireResponse.json` +
//! `CloseResponse.json`, the §13.4-style drift tripwire committed byte-identical
//! in both repos) are the Runners TL's joint-pass lane and are PENDING that pass.
//! These hugit-side fixtures pin what hugit PARSES; when the cross-repo vectors
//! land, this test should additionally pin them byte-identically (as
//! `runner/metrics.rs` already does for `conformance/IntentMetrics.json`).

use std::path::PathBuf;

use hugit_checks::runner::{AcquireResponse, CloseResponse};
use hugit_contracts::{IntentMetrics, RunnerState};

/// Read a wire fixture under `tests/fixtures/wire/`.
fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wire")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
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
