//! WP-E2b acceptance oracle — PR/issue → proposed intents + fidelity contract.
//!
//! Items:
//!   ② `item_2_proposed_intents_with_provenance`
//!   ② `item_2_intents_flagged_proposed_non_authoritative`
//!   ② `item_2_bare_commit_produces_no_prissue_intent`
//!   ⑥ `item_6_fidelity_body_preserved`
//!   ⑥ `item_6_fidelity_comment_threads_preserved`
//!   ⑥ `item_6_fidelity_state_preserved`
//!   ⑥ `item_6_fidelity_labels_preserved`
//!   ⑥ `item_6_fidelity_crossrefs_preserved`
//!   ⑥ `item_6_non_imported_elements_enumerated`
//!
//! All items use fixture proofs (no live GitHub calls required).
//! Import boundary: bare commits must NOT produce PR/issue intents (E2a⑤ owns the
//! boundary law; E2b asserts only the proposed/non-authoritative flag on PR/issue intents).
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::IntentSidecar` (frozen; proposed-intent type)
//! - `hugit_mirror::import::prissue::{PrIssueImporter, ProposedIntent, ElementProvenance,
//!    FidelityReport, NON_IMPORTED_ELEMENTS, CrossRefResolution}`

use hugit_contracts::IntentSidecar;
use hugit_mirror::import::prissue::{
    CrossRefResolution, ElementProvenance, FidelityReport, NON_IMPORTED_ELEMENTS,
    PrIssueImporter, ProposedIntent,
};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a minimal fixture PR metadata payload.
fn fixture_pr_payload() -> serde_json::Value {
    serde_json::json!({
        "number": 42,
        "title": "feat: add graph resolver",
        "body": "Implements the graph resolver per spec §3.\n\nFixes #17.",
        "state": "open",
        "labels": [{"name": "enhancement"}, {"name": "graph"}],
        "comments": [
            {
                "id": 1,
                "body": "LGTM, left one nit.",
                "user": {"login": "reviewer-a"}
            }
        ],
        "review_threads": [
            {
                "id": 10,
                "comments": [
                    {"body": "nit: rename this var", "user": {"login": "reviewer-a"}}
                ]
            }
        ],
        "cross_refs": ["#17", "#22"],
        "url": "https://github.com/humangr-labs/hugit-fleet-syn-1/pull/42"
    })
}

/// Build a minimal fixture issue metadata payload.
fn fixture_issue_payload() -> serde_json::Value {
    serde_json::json!({
        "number": 17,
        "title": "bug: resolver panics on empty graph",
        "body": "Steps to reproduce:\n1. empty graph\n2. call resolve()\nPanic.",
        "state": "closed",
        "labels": [{"name": "bug"}],
        "comments": [
            {
                "id": 2,
                "body": "Fixed in PR #42.",
                "user": {"login": "author-b"}
            }
        ],
        "review_threads": [],
        "cross_refs": ["#42"],
        "url": "https://github.com/humangr-labs/hugit-fleet-syn-1/issues/17"
    })
}

// ── ② PRs/issues → proposed intents w/ provenance ────────────────────────────
#[test]
fn item_2_proposed_intents_with_provenance() {
    // PrIssueImporter must project a fixture PR/issue payload to a ProposedIntent
    // that carries per-element provenance (source URL + element-level origin).

    let importer = PrIssueImporter::new_fixture();

    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("PR projection must succeed for valid fixture");

    // Must carry a source provenance URL.
    assert!(
        !pr_intent.provenance.source_url.is_empty(),
        "ProposedIntent must carry a non-empty source_url provenance"
    );
    assert!(
        pr_intent.provenance.source_url.contains("pull/42"),
        "source_url must reference the PR number"
    );

    // Per-element provenance must be present for at least one preserved element.
    assert!(
        !pr_intent.element_provenances.is_empty(),
        "ProposedIntent must carry at least one ElementProvenance"
    );

    let issue_intent: ProposedIntent = importer
        .project_issue(fixture_issue_payload())
        .expect("issue projection must succeed for valid fixture");

    assert!(
        !issue_intent.provenance.source_url.is_empty(),
        "issue ProposedIntent must carry a non-empty source_url provenance"
    );
    assert!(
        issue_intent.provenance.source_url.contains("issues/17"),
        "source_url must reference the issue number"
    );
}

// ── ② proposed/non-authoritative flag on every imported PR/issue intent ───────
#[test]
fn item_2_intents_flagged_proposed_non_authoritative() {
    // Every ProposedIntent produced from a PR/issue must be flagged
    // proposed=true and non_authoritative=true. It must never gate/block/land.

    let importer = PrIssueImporter::new_fixture();

    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("PR projection must succeed");

    assert!(
        pr_intent.proposed,
        "PR intent must be flagged proposed=true"
    );
    assert!(
        pr_intent.non_authoritative,
        "PR intent must be flagged non_authoritative=true"
    );

    // The underlying IntentSidecar must also carry the proposed state.
    let sidecar: IntentSidecar = pr_intent
        .as_intent_sidecar()
        .expect("ProposedIntent must project to IntentSidecar");

    assert_eq!(
        sidecar.state, "proposed",
        "IntentSidecar.state must be 'proposed' for PR/issue imports"
    );
    assert!(
        !sidecar.authoritative,
        "IntentSidecar.authoritative must be false for PR/issue imports"
    );

    let issue_intent: ProposedIntent = importer
        .project_issue(fixture_issue_payload())
        .expect("issue projection must succeed");

    assert!(issue_intent.proposed, "issue intent must be flagged proposed=true");
    assert!(
        issue_intent.non_authoritative,
        "issue intent must be flagged non_authoritative=true"
    );
}

// ── ⑥ fidelity: body preserved with provenance ───────────────────────────────
#[test]
fn item_6_fidelity_body_preserved() {
    // The PR/issue body must be preserved verbatim in the ProposedIntent,
    // with per-element provenance recorded for the body field.

    let importer = PrIssueImporter::new_fixture();
    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("PR projection must succeed");

    let body_provenance: &ElementProvenance = pr_intent
        .element_provenances
        .iter()
        .find(|p| p.element == "body")
        .expect("element_provenances must contain a 'body' entry (fidelity contract)");

    assert!(
        !body_provenance.origin.is_empty(),
        "body ElementProvenance must carry a non-empty origin"
    );

    // Body content must be preserved in the intent payload.
    assert!(
        pr_intent.body.contains("graph resolver"),
        "PR body must be preserved verbatim in ProposedIntent"
    );
}

// ── ⑥ fidelity: comment/review threads preserved with provenance ──────────────
#[test]
fn item_6_fidelity_comment_threads_preserved() {
    // PR/issue comments and review threads must be preserved with per-element provenance.

    let importer = PrIssueImporter::new_fixture();
    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("PR projection must succeed");

    // At least one comment must be preserved.
    assert!(
        !pr_intent.comments.is_empty(),
        "PR comments must be preserved in ProposedIntent (fidelity: comment threads)"
    );
    assert!(
        pr_intent.comments[0].body.contains("LGTM"),
        "first comment body must be preserved verbatim"
    );

    // At least one review thread must be preserved.
    assert!(
        !pr_intent.review_threads.is_empty(),
        "PR review threads must be preserved in ProposedIntent (fidelity: review threads)"
    );

    // Per-element provenance for comments.
    let comment_provenance = pr_intent
        .element_provenances
        .iter()
        .find(|p| p.element == "comments" || p.element.starts_with("comment/"))
        .expect("element_provenances must contain a 'comments' entry");

    assert!(
        !comment_provenance.origin.is_empty(),
        "comment ElementProvenance must carry a non-empty origin"
    );
}

// ── ⑥ fidelity: state preserved with provenance ──────────────────────────────
#[test]
fn item_6_fidelity_state_preserved() {
    // PR/issue open/closed state must be preserved with per-element provenance.

    let importer = PrIssueImporter::new_fixture();

    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("PR projection must succeed");

    assert_eq!(
        pr_intent.state, "open",
        "PR state must be preserved as 'open' in ProposedIntent"
    );

    let state_provenance = pr_intent
        .element_provenances
        .iter()
        .find(|p| p.element == "state")
        .expect("element_provenances must contain a 'state' entry");

    assert!(
        !state_provenance.origin.is_empty(),
        "state ElementProvenance must carry a non-empty origin"
    );

    let issue_intent: ProposedIntent = importer
        .project_issue(fixture_issue_payload())
        .expect("issue projection must succeed");

    assert_eq!(
        issue_intent.state, "closed",
        "issue state must be preserved as 'closed' in ProposedIntent"
    );
}

// ── ⑥ fidelity: labels preserved with provenance ─────────────────────────────
#[test]
fn item_6_fidelity_labels_preserved() {
    // PR/issue labels must be preserved as a list with per-element provenance.

    let importer = PrIssueImporter::new_fixture();
    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("PR projection must succeed");

    assert!(
        pr_intent.labels.contains(&"enhancement".to_string()),
        "label 'enhancement' must be preserved in ProposedIntent"
    );
    assert!(
        pr_intent.labels.contains(&"graph".to_string()),
        "label 'graph' must be preserved in ProposedIntent"
    );

    let labels_provenance = pr_intent
        .element_provenances
        .iter()
        .find(|p| p.element == "labels")
        .expect("element_provenances must contain a 'labels' entry");

    assert!(
        !labels_provenance.origin.is_empty(),
        "labels ElementProvenance must carry a non-empty origin"
    );
}

// ── ⑥ fidelity: cross-refs preserved or recorded as residual ─────────────────
#[test]
fn item_6_fidelity_crossrefs_preserved() {
    // Cross-refs to imported objects must resolve to the imported objects.
    // Dangling cross-refs (referencing non-imported objects) must be recorded
    // as explicit residuals — never fabricated, never silently dropped.

    let importer = PrIssueImporter::new_fixture();
    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("PR projection must succeed");

    // At least one cross-ref must be present (fixture has "#17" and "#22").
    assert!(
        !pr_intent.cross_refs.is_empty(),
        "cross-refs must be preserved in ProposedIntent (fidelity contract)"
    );

    for xref in &pr_intent.cross_refs {
        match &xref.resolution {
            CrossRefResolution::Resolved { imported_id } => {
                assert!(
                    !imported_id.is_empty(),
                    "resolved cross-ref must carry the imported object id"
                );
            }
            CrossRefResolution::Residual { reason } => {
                assert!(
                    !reason.is_empty(),
                    "dangling cross-ref must carry a non-empty residual reason (not fabricated)"
                );
            }
        }
    }

    // Per-element provenance for cross-refs.
    let xref_provenance = pr_intent
        .element_provenances
        .iter()
        .find(|p| p.element == "cross_refs" || p.element.starts_with("cross_ref/"))
        .expect("element_provenances must contain a 'cross_refs' entry");

    assert!(
        !xref_provenance.origin.is_empty(),
        "cross_refs ElementProvenance must carry a non-empty origin"
    );
}

// ── ⑥ non-imported elements explicitly enumerated (no silent drop) ────────────
#[test]
fn item_6_non_imported_elements_enumerated() {
    // NON_IMPORTED_ELEMENTS must be a non-empty published constant enumerating
    // every element that is deliberately not imported (e.g. reactions,
    // social-graph signals). No silent drop: anything not imported must be named.

    assert!(
        !NON_IMPORTED_ELEMENTS.is_empty(),
        "NON_IMPORTED_ELEMENTS must be a non-empty list (no silent drop of excluded elements)"
    );

    // The published list must name at least one concrete element.
    // Per contract: reactions and social-graph signals are excluded.
    let has_reactions = NON_IMPORTED_ELEMENTS.iter().any(|e| e.contains("reaction"));
    assert!(
        has_reactions,
        "NON_IMPORTED_ELEMENTS must explicitly enumerate 'reactions' (per fidelity contract)"
    );

    // Every entry must be a non-empty string.
    for entry in NON_IMPORTED_ELEMENTS {
        assert!(
            !entry.is_empty(),
            "every NON_IMPORTED_ELEMENTS entry must be a non-empty string"
        );
    }
}

// ── ② import boundary: bare commit path produces no PR/issue intent ────────────
#[test]
fn item_2_bare_commit_produces_no_prissue_intent() {
    // The PrIssueImporter must not accept a bare-commit payload and must not
    // expose any API that mints a ProposedIntent from a bare commit OID.
    // Structural: PrIssueImporter has no `project_commit` method.
    //
    // Value-level: projecting a bare-commit-shaped payload must return an error,
    // not a ProposedIntent.

    let importer = PrIssueImporter::new_fixture();

    // A bare-commit payload (no PR/issue number, no body, no labels).
    let bare_commit = serde_json::json!({
        "oid": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        "type": "commit",
        "message": "fix: resolver panic"
    });

    // project_pr must reject a bare-commit payload.
    let result = importer.project_pr(bare_commit.clone());
    assert!(
        result.is_err(),
        "PrIssueImporter must reject a bare-commit payload for project_pr (import boundary)"
    );

    // project_issue must reject a bare-commit payload.
    let result2 = importer.project_issue(bare_commit);
    assert!(
        result2.is_err(),
        "PrIssueImporter must reject a bare-commit payload for project_issue (import boundary)"
    );

    // The FidelityReport for a valid PR must not reference any commit OID as a proposed intent.
    let pr_intent: ProposedIntent = importer
        .project_pr(fixture_pr_payload())
        .expect("valid PR projection must succeed");

    let report: FidelityReport = importer
        .fidelity_report(&pr_intent)
        .expect("fidelity report must succeed");

    assert!(
        !report.contains_bare_commit_intent,
        "FidelityReport must confirm no bare-commit intent was synthesized"
    );
}
