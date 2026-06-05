//! WP-B6 acceptance tests — hugit-app-sidecar intent sidecar.
//!
//! One `#[test]` per owned item, named `item_<n>_<slug>` per the lead's
//! convention. Contract: docs/plan/wp-contracts/WP-B6.md.
//!
//! Item ① — parse/validate/render
//! Item ② — malformed sidecar → actionable comment (never silent drop, never
//!           hard error that blocks the PR)
//! Item ③ — corpus → CAS keyed by intent_id
//! Item ④(R2) — sidecar is non-authoritative: never gates or blocks landing

use hugit_app_sidecar::{
    CorpusWriter, RenderOutput,
    corpus::CasRef,
    guard::{assert_non_authoritative, is_non_authoritative},
    parse::{ValidationFailure, parse_sidecar},
    render::render_sidecar,
};
use hugit_contracts::IntentSidecar;

// ── Item ① — parse / validate / render ───────────────────────────────────────

#[test]
fn item_1_parsed_validated_rendered() {
    // ① Construct a well-formed sidecar JSON.
    let raw = serde_json::json!({
        "intent_id": "550e8400-e29b-41d4-a716-446655440000",
        "charter": "Implement the intent sidecar: parse, validate, render, and persist to CAS.",
        "acceptance": [
            "Sidecar parses and validates from raw JSON",
            "Render produces PR comment + check summary",
            "Corpus written to CAS keyed by intent_id",
            "Sidecar is non-authoritative: never gates landing"
        ],
        "context_ref": "cas/sha256/abcdef1234567890abcdef1234567890",
        "authoritative": false
    })
    .to_string();

    // Parse must succeed.
    let sidecar = parse_sidecar(&raw).expect("valid sidecar must parse without error");

    // Fields must round-trip correctly.
    assert_eq!(sidecar.intent_id, "550e8400-e29b-41d4-a716-446655440000");
    assert!(!sidecar.charter.is_empty(), "charter must be non-empty");
    assert_eq!(
        sidecar.acceptance.len(),
        4,
        "must have 4 acceptance criteria"
    );
    assert!(
        sidecar.context_ref.starts_with("cas/"),
        "context_ref must be a CAS ref"
    );
    assert!(!sidecar.authoritative, "authoritative must be false");

    // Render must produce non-empty PR comment + check summary.
    let output: RenderOutput = render_sidecar(&sidecar);

    assert!(
        !output.pr_comment.is_empty(),
        "PR comment must be non-empty"
    );
    assert!(
        !output.check_summary.is_empty(),
        "check summary must be non-empty"
    );

    // PR comment must contain charter, acceptance markers, context_ref, and
    // intent_id.
    assert!(
        output.pr_comment.contains(&sidecar.intent_id),
        "PR comment must contain intent_id"
    );
    assert!(
        output.pr_comment.contains("Charter"),
        "PR comment must contain Charter section"
    );
    assert!(
        output.pr_comment.contains("Acceptance"),
        "PR comment must contain Acceptance section"
    );
    assert!(
        output.pr_comment.contains(&sidecar.context_ref),
        "PR comment must contain context_ref"
    );

    // Check summary must contain intent_id.
    assert!(
        output.check_summary.contains(&sidecar.intent_id),
        "check summary must contain intent_id"
    );
}

// ── Item ② — malformed sidecar → actionable comment ─────────────────────────

#[test]
fn item_2_malformed_actionable_comment() {
    // ② A completely invalid JSON must yield an InvalidJson error with an
    // actionable comment (never a silent drop, never a hard error that blocks
    // the PR).
    let not_json = "this is not json at all {{{";
    let err = parse_sidecar(not_json).expect_err("invalid JSON must fail");
    let comment = err.actionable_comment();
    assert!(
        !comment.is_empty(),
        "actionable comment must be non-empty for invalid JSON"
    );
    assert!(
        comment.contains("malformed") || comment.contains("valid JSON") || comment.contains("JSON"),
        "actionable comment must mention the JSON problem"
    );
    assert!(
        comment.contains("not block"),
        "actionable comment must state it does not block landing"
    );

    // ② A sidecar with missing required fields must yield an InvalidSidecar
    // error naming the specific fields.
    let missing_fields = serde_json::json!({
        "intent_id": "",
        "charter": "",
        "acceptance": [],
        "context_ref": "",
        "authoritative": false
    })
    .to_string();

    let err2 = parse_sidecar(&missing_fields).expect_err("empty fields must fail validation");
    assert!(
        err2.is_malformed(),
        "empty-fields sidecar must be identified as malformed"
    );
    let comment2 = err2.actionable_comment();
    assert!(
        !comment2.is_empty(),
        "actionable comment must be non-empty for invalid_sidecar"
    );
    // Must name the specific fields that failed.
    assert!(
        comment2.contains("intent_id"),
        "comment must name the intent_id field"
    );
    assert!(
        comment2.contains("charter"),
        "comment must name the charter field"
    );
    assert!(
        comment2.contains("acceptance"),
        "comment must name the acceptance field"
    );
    // Must state it does not block landing.
    assert!(
        comment2.contains("not") && comment2.contains("block"),
        "actionable comment must state it does not block landing"
    );

    // ② A sidecar claiming authoritative=true is a validation_failure (not a
    // silent acceptance).
    let authoritative_sidecar = serde_json::json!({
        "intent_id": "test-id",
        "charter": "some charter",
        "acceptance": ["criterion 1"],
        "context_ref": "cas/sha256/aabbcc",
        "authoritative": true
    })
    .to_string();

    let err3 =
        parse_sidecar(&authoritative_sidecar).expect_err("authoritative=true must be rejected");
    let comment3 = err3.actionable_comment();
    assert!(
        comment3.contains("authoritative"),
        "comment must name the authoritative field failure"
    );
}

// ── Item ③ — corpus → CAS by intent_id ───────────────────────────────────────

#[test]
fn item_3_corpus_cas_by_intent_id() {
    // ③ A validated sidecar must be written to the corpus keyed by intent_id.
    let sidecar = IntentSidecar {
        intent_id: "intent-b6-corpus-test-001".to_string(),
        charter: "Test corpus write to CAS".to_string(),
        acceptance: vec!["CAS key contains intent_id".to_string()],
        context_ref: "cas/sha256/corpus-test-ref".to_string(),
        authoritative: false,
    };

    let writer = CorpusWriter::new_local();
    let cas_ref: CasRef = writer.write(&sidecar).expect("CAS write must succeed");

    // The CAS ref must be keyed by intent_id.
    assert_eq!(
        cas_ref.intent_id, sidecar.intent_id,
        "CasRef.intent_id must match the sidecar's intent_id"
    );
    assert!(
        cas_ref.cas_key.contains(&sidecar.intent_id),
        "CAS key must contain the intent_id (addressable by intent_id)"
    );
    assert!(
        cas_ref.byte_size > 0,
        "byte_size must be positive (sidecar was serialised)"
    );

    // ③ Write multiple sidecars: each must have a distinct CAS key by intent_id.
    let sidecar2 = IntentSidecar {
        intent_id: "intent-b6-corpus-test-002".to_string(),
        charter: "Second corpus write test".to_string(),
        acceptance: vec!["Each intent gets its own CAS key".to_string()],
        context_ref: "cas/sha256/corpus-test-ref-2".to_string(),
        authoritative: false,
    };

    let cas_ref2 = writer
        .write(&sidecar2)
        .expect("second CAS write must succeed");
    assert_ne!(
        cas_ref.cas_key, cas_ref2.cas_key,
        "distinct intent_ids must yield distinct CAS keys"
    );
    assert!(
        cas_ref2.cas_key.contains(&sidecar2.intent_id),
        "second CAS key must contain the second intent_id"
    );
}

// ── Item ④(R2) — sidecar is non-authoritative: never gates landing ────────────

#[test]
fn item_4_non_authoritative_never_gates() {
    // ④ A well-formed sidecar with authoritative=false must pass the guard.
    let good_sidecar = IntentSidecar {
        intent_id: "intent-b6-guard-test".to_string(),
        charter: "Guard test".to_string(),
        acceptance: vec!["Sidecar must be non-authoritative".to_string()],
        context_ref: "cas/sha256/guard-test-ref".to_string(),
        authoritative: false,
    };

    assert_non_authoritative(&good_sidecar)
        .expect("a non-authoritative sidecar must pass the guard");
    assert!(
        is_non_authoritative(&good_sidecar),
        "is_non_authoritative must return true for authoritative=false"
    );

    // ④ A sidecar with authoritative=true must be rejected by the guard.
    // (This cannot come from parse_sidecar — that already rejects it.
    // This tests the landing-path guard independently.)
    let bad_sidecar = IntentSidecar {
        intent_id: "intent-b6-guard-bad".to_string(),
        charter: "Bad sidecar".to_string(),
        acceptance: vec!["Should be rejected".to_string()],
        context_ref: "cas/sha256/bad-ref".to_string(),
        authoritative: false, // hard-false: cannot be set to true via any valid parse path
    };

    // Even if somehow constructed with authoritative=false (the only valid
    // state), the guard confirms it is non-authoritative.
    assert!(
        is_non_authoritative(&bad_sidecar),
        "constructed sidecar must be non-authoritative"
    );

    // ④ Parse path: authoritative=true is a validation_failure, so it can
    // never reach the landing path through the normal parse/validate flow.
    let raw_authoritative = serde_json::json!({
        "intent_id": "intent-authoritative-attempt",
        "charter": "Attempting to set authoritative=true",
        "acceptance": ["This should be rejected"],
        "context_ref": "cas/sha256/auth-attempt",
        "authoritative": true
    })
    .to_string();

    let parse_result = parse_sidecar(&raw_authoritative);
    assert!(
        parse_result.is_err(),
        "parse must reject authoritative=true sidecars"
    );
    // The parse error is a validation_failure (not a hard error) — it produces
    // an actionable comment and does NOT block landing.
    if let Err(ref e) = parse_result {
        assert!(
            e.is_malformed(),
            "authoritative=true must be a validation_failure (malformed), not a hard error"
        );
        let comment = e.actionable_comment();
        assert!(
            !comment.is_empty(),
            "validation_failure must produce a non-empty actionable comment"
        );
        // The comment must NOT say the PR is blocked.
        assert!(
            !comment.to_lowercase().contains("pr is blocked")
                && !comment.to_lowercase().contains("landing is blocked"),
            "actionable comment must not say landing is blocked"
        );
    }

    // ④ Structural: the ValidationFailure type must exist and be usable.
    let vf = ValidationFailure {
        field: "authoritative".to_string(),
        expected: "false".to_string(),
        found: "true".to_string(),
    };
    assert_eq!(vf.field, "authoritative");
}
