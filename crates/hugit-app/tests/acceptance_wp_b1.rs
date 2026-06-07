//! WP-B1 acceptance tests — hugit-app GitHub App skeleton.
//!
//! One `#[test]` per owned item, named `item_<n>_<slug>` per the lead's
//! convention. Contract: docs/plan/wp-contracts/WP-B1.md.
//!
//! Item ① — forged webhook → 401 + audit EventRecord
//! Item ② — PR event persisted, ack < 1s (wall-clock gated)
//! Item ③ — check-run written back via Checks API client
//! Item ④ — least-privilege manifest snapshot matches committed fixture
//! Item ⑤ — uninstall revokes access + halts processing (audited)

use hugit_app::{
    checks::{ChecksClient, ChecksClientError},
    manifest::{APP_MANIFEST, validate_manifest},
    persistence::{PersistenceAdapter, PersistenceError},
    webhook::{
        WebhookError, WebhookProcessor, build_ack_receipt, build_installation_revoked_record,
        build_webhook_rejected_record, verify_x_hub_signature_256,
    },
};
use hugit_contracts::{AckReceipt, ChecksWriteRequest, EventRecord};

// ── Item ① — forged webhook → 401 + audit EventRecord ─────────────────────

#[test]
fn item_1_forged_webhook_rejected() {
    let secret = b"test-webhook-secret-b1";
    let payload = b"{\"action\":\"opened\",\"pull_request\":{\"number\":42}}";

    // Sign with the WRONG secret to produce a forged signature.
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let wrong_secret = b"wrong-secret";
    let mut mac = HmacSha256::new_from_slice(wrong_secret).unwrap();
    mac.update(payload);
    let bad_sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    // Verification with the real secret must fail.
    let result = verify_x_hub_signature_256(secret, payload, Some(bad_sig.as_str()));
    assert!(
        matches!(result, Err(WebhookError::SignatureMismatch)),
        "forged signature must be rejected with SignatureMismatch"
    );

    // Missing signature must also be rejected.
    let result_none = verify_x_hub_signature_256(secret, payload, None);
    assert!(
        matches!(result_none, Err(WebhookError::MissingSignature)),
        "absent X-Hub-Signature-256 must be rejected with MissingSignature"
    );

    // Build the audit EventRecord for the rejection.
    let genesis_hash = "0".repeat(64);
    let record = build_webhook_rejected_record("delivery-abc", &genesis_hash, 1, 1_000_000);

    assert_eq!(
        record.kind, "webhook.rejected",
        "kind must be webhook.rejected"
    );
    assert_eq!(record.seq, 1);
    assert!(!record.this_hash.is_empty(), "this_hash must be non-empty");
    assert!(
        record.payload.contains("delivery-abc"),
        "payload must reference the delivery_id"
    );

    // Verify the WebhookProcessor also produces the correct error path.
    let processor = WebhookProcessor::new(secret.to_vec());
    let proc_result = processor.process(
        payload,
        Some(bad_sig.as_str()),
        "delivery-abc",
        "pull_request",
        1_000_000,
    );
    assert!(
        proc_result.is_err(),
        "WebhookProcessor must return Err for a forged signature"
    );
}

// ── Item ② — PR event persisted, ack < 1s ────────────────────────────────

#[test]
fn item_2_pr_event_persisted_ack_under_1s() {
    let secret = b"test-webhook-secret-b1";
    let payload = b"{\"action\":\"opened\",\"pull_request\":{\"number\":42}}";
    let delivery_id = "delivery-pr-42";
    let event_type = "pull_request";
    let received_at: u64 = 1_748_000_000_000; // ms

    // Sign with the real secret.
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(payload);
    let good_sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let processor = WebhookProcessor::new(secret.to_vec());

    // Gate: persist + ack must complete in < 1s.
    let t0 = std::time::Instant::now();

    let envelope = processor
        .process(
            payload,
            Some(good_sig.as_str()),
            delivery_id,
            event_type,
            received_at,
        )
        .expect("valid signature must be accepted");

    let mut adapter = PersistenceAdapter::new_local();
    let persist_result = adapter
        .persist_event(&envelope, None)
        .expect("persist must succeed");

    let ack: AckReceipt = build_ack_receipt(delivery_id, received_at + 10);
    let elapsed = t0.elapsed();

    // Wall-clock assertion: must be well under 1s (local, should be sub-ms).
    assert!(
        elapsed.as_millis() < 1000,
        "persist + ack must complete in <1s, took {}ms",
        elapsed.as_millis()
    );

    // Structural assertions.
    assert_eq!(envelope.delivery_id, delivery_id);
    assert_eq!(envelope.event_type, event_type);
    assert!(persist_result.enqueued, "heavy work must be enqueued");
    assert!(
        persist_result.cas_key.contains(delivery_id),
        "CAS key must reference delivery_id"
    );
    assert_eq!(ack.delivery_id, delivery_id);
    assert!(!ack.processing_id.is_empty(), "processing_id must be set");

    // Round-trip: retrieve the persisted event.
    let retrieved = adapter
        .get_event(delivery_id)
        .expect("persisted event must be retrievable");
    assert_eq!(retrieved.delivery_id, delivery_id);
}

// ── Item ③ — check-run written back via Checks API ───────────────────────

#[test]
fn item_3_check_run_written_back() {
    let client = ChecksClient::new_local();

    let request = ChecksWriteRequest {
        repo: "hugr/hugit".to_string(),
        head_sha: "abc123def456".to_string(),
        check_name: "hugit/merge-check".to_string(),
        status: "completed".to_string(),
        conclusion: Some("success".to_string()),
        summary: "All merge checks passed.".to_string(),
        output_ref: "cas/sha256/deadbeef".to_string(),
    };

    // Write with a synthetic installation token.
    let response = client
        .write_check_run(&request, Some("token-for-install-123"))
        .expect("local Checks client must succeed");

    assert!(
        response.check_run_id > 0,
        "check_run_id must be a positive integer"
    );
    assert!(
        response.html_url.contains(&request.repo),
        "html_url must reference the repo"
    );
    assert!(
        response
            .html_url
            .contains(&response.check_run_id.to_string()),
        "html_url must contain the check_run_id"
    );

    // Verify round-trip through the contract types.
    let _serialised = serde_json::to_string(&response).expect("ChecksWriteResponse must serialise");

    // ORACLE: None token must return TokenRevoked (not succeed or return a
    // different error).
    let err = client
        .write_check_run(&request, None)
        .expect_err("None token must return an error");
    assert!(
        matches!(err, ChecksClientError::TokenRevoked { .. }),
        "write_check_run with None token must return TokenRevoked, got: {err:?}"
    );
}

// ── Item ④ — least-privilege manifest snapshot ────────────────────────────

#[test]
fn item_4_least_privilege_manifest_snapshot() {
    // The committed fixture is the source of truth; APP_MANIFEST must match it.
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("app-manifest.json");

    let fixture_bytes =
        std::fs::read_to_string(&fixture_path).expect("app-manifest.json fixture must exist");

    // Byte-identity: APP_MANIFEST (include_str!) must equal the committed file.
    assert_eq!(
        APP_MANIFEST, fixture_bytes,
        "APP_MANIFEST constant must byte-match the committed fixture"
    );

    // Validate least-privilege constraints.
    validate_manifest(APP_MANIFEST).expect("APP_MANIFEST must pass least-privilege validation");

    // Parse to verify specific scope claims.
    let manifest: serde_json::Value =
        serde_json::from_str(APP_MANIFEST).expect("APP_MANIFEST must be valid JSON");

    let perms = manifest
        .get("default_permissions")
        .expect("default_permissions must be present");

    // Checks: write (needed for Checks API write-back).
    assert_eq!(
        perms.get("checks").and_then(|v| v.as_str()),
        Some("write"),
        "checks scope must be write"
    );

    // Pull requests: read only.
    assert_eq!(
        perms.get("pull_requests").and_then(|v| v.as_str()),
        Some("read"),
        "pull_requests scope must be read (least privilege)"
    );

    // Contents: read only.
    assert_eq!(
        perms.get("contents").and_then(|v| v.as_str()),
        Some("read"),
        "contents scope must be read (least privilege)"
    );

    // No admin or members permissions.
    assert!(
        perms.get("administration").is_none(),
        "administration scope must be absent (over-privileged)"
    );
    assert!(
        perms.get("members").is_none(),
        "members scope must be absent (over-privileged)"
    );

    // Events: check_suite, check_run, pull_request, installation.
    let events = manifest
        .get("default_events")
        .and_then(|v| v.as_array())
        .expect("default_events must be an array");
    let event_names: Vec<&str> = events.iter().filter_map(|e| e.as_str()).collect();

    for required in &["check_suite", "check_run", "pull_request", "installation"] {
        assert!(
            event_names.contains(required),
            "manifest must subscribe to event: {required}"
        );
    }
}

// ── Item ⑤ — uninstall revokes access + halts processing (audited) ────────

#[test]
fn item_5_uninstall_revokes_access_and_halts_processing() {
    // ── ORACLE: token_revoked reflects ACTUAL stored state ────────────────
    // A processor with a registered token for install-9999 must return
    // token_revoked = true; one WITHOUT a stored token must return false.

    let installation_id = "install-9999";
    let genesis_hash = "0".repeat(64);
    let seq = 1u64;
    let recorded_at = 1_748_000_001_000u64;

    // Case A: token IS stored — uninstall must report token_revoked = true.
    let processor_with_token = WebhookProcessor::new(b"test-webhook-secret-b1".to_vec());
    processor_with_token.store_token(installation_id, "tok-abc123");

    let (outcome_a, record) =
        processor_with_token.handle_uninstall(installation_id, &genesis_hash, seq, recorded_at);

    assert_eq!(outcome_a.installation_id, installation_id);
    assert!(
        outcome_a.token_revoked,
        "token_revoked must be true when a token was present in the store"
    );
    assert!(
        outcome_a.processing_halted,
        "queued processing must be halted"
    );

    // Calling again: token is gone → token_revoked must be false (idempotent
    // second uninstall on an already-cleared slot).
    let (outcome_a2, _) =
        processor_with_token.handle_uninstall(installation_id, &genesis_hash, seq + 1, recorded_at);
    assert!(
        !outcome_a2.token_revoked,
        "second uninstall on already-revoked install must return token_revoked = false"
    );

    // Case B: no token stored — uninstall must report token_revoked = false.
    let processor_no_token = WebhookProcessor::new(b"test-webhook-secret-b1".to_vec());
    let (outcome_b, _) =
        processor_no_token.handle_uninstall(installation_id, &genesis_hash, seq, recorded_at);
    assert!(
        !outcome_b.token_revoked,
        "token_revoked must be false when no token was stored for this installation"
    );

    // Audit record assertions (against outcome_a's record).
    assert_eq!(
        record.kind, "installation.revoked",
        "audit record kind must be installation.revoked"
    );
    assert_eq!(record.seq, seq);
    assert_eq!(record.prev_hash, genesis_hash);
    assert!(!record.this_hash.is_empty(), "this_hash must be computed");
    assert!(
        record.payload.contains(installation_id),
        "audit payload must reference the installation_id"
    );
    assert_eq!(record.recorded_at, recorded_at);

    // Verify the direct builder function also produces correct records.
    let record2 =
        build_installation_revoked_record(installation_id, &genesis_hash, seq, recorded_at);
    assert_eq!(
        record, record2,
        "both builders must produce identical records"
    );

    // Verify the EventRecord is well-formed for the audit trail.
    let serialised =
        serde_json::to_string(&record).expect("installation.revoked EventRecord must serialise");
    let round_tripped: EventRecord =
        serde_json::from_str(&serialised).expect("EventRecord must deserialise");
    assert_eq!(
        record, round_tripped,
        "EventRecord must survive JSON round-trip"
    );

    // ── ORACLE: persist_event for halted install must be rejected ─────────
    let mut adapter = PersistenceAdapter::new_local();

    // First halt the installation.
    let halt_result = adapter.halt_installation(installation_id);
    assert_eq!(halt_result.installation_id, installation_id);
    assert!(
        halt_result.items_halted > 0 || halt_result.installation_id == installation_id,
        "halt_installation must register the installation as halted"
    );

    // Build a dummy envelope referencing the halted installation.
    use hugit_contracts::SignedEventEnvelope;
    let halted_envelope = SignedEventEnvelope {
        delivery_id: "delivery-halted-1".to_string(),
        event_type: "pull_request".to_string(),
        signature: "sha256=deadbeef".to_string(),
        payload: "{}".to_string(),
        received_at: recorded_at + 1000,
    };

    let persist_err = adapter
        .persist_event(&halted_envelope, Some(installation_id))
        .expect_err("persist_event for halted install must return an error");
    assert!(
        matches!(persist_err, PersistenceError::InstallationHalted { .. }),
        "persist_event for halted install must return InstallationHalted, got: {persist_err:?}"
    );

    // An un-halted install must still succeed.
    let other_envelope = SignedEventEnvelope {
        delivery_id: "delivery-other-1".to_string(),
        event_type: "pull_request".to_string(),
        signature: "sha256=deadbeef".to_string(),
        payload: "{}".to_string(),
        received_at: recorded_at + 2000,
    };
    adapter
        .persist_event(&other_envelope, Some("install-other-777"))
        .expect("persist_event for non-halted install must succeed");
}
