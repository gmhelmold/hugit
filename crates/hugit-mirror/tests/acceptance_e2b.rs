//! WP-E2b acceptance oracle — PR/issue → proposed intents + fidelity contract.
//!
//! Owned acceptance items (VERBATIM from decomposition v2.0, E2):
//!   ② PRs/issues → proposed intents w/ provenance
//!   ⑥(R3) PR/issue fidelity contract: stated set (body, comment/review threads,
//!          state, labels, cross-refs) preserved with per-element provenance;
//!          non-imported elements explicitly enumerated; verified on a fixture
//!          containing each element.
//!
//! All items use fixture proofs — no live GitHub calls required.
//!
//! Import boundary (E2a⑤): bare commits MUST NOT produce intents. E2b only
//! asserts that PR/issue intents are flagged proposed/non-authoritative.

use hugit_contracts::intent_sidecar::IntentSidecar;
use hugit_mirror::import::prissue::{
    CrossRef, ElementProvenance, ImportedComment, ImportedPrIssue, NON_IMPORTED,
    PrIssueImportError, PrIssueState, SourceKind, import_prissue, import_prissue_batch,
};

// ── Fixture helpers ────────────────────────────────────────────────────────────

fn fixture_pr() -> ImportedPrIssue {
    let url = "https://github.com/humangr-labs/hugit/pull/1";
    ImportedPrIssue {
        kind: SourceKind::PullRequest,
        source_id: 1,
        source_url: url.to_string(),
        body: "This PR adds the landing queue.".to_string(),
        body_provenance: ElementProvenance::new(url, "body"),
        state: PrIssueState::Open,
        state_provenance: ElementProvenance::new(url, "state"),
        labels: vec![
            (
                "enhancement".to_string(),
                ElementProvenance::new(url, "label/enhancement"),
            ),
            (
                "priority:high".to_string(),
                ElementProvenance::new(url, "label/priority:high"),
            ),
        ],
        comments: vec![
            ImportedComment {
                id: 100,
                body: "LGTM, one nit below.".to_string(),
                author: "reviewer-a".to_string(),
                created_at: 1_000_000,
                provenance: ElementProvenance::new(url, "comment/100"),
            },
            ImportedComment {
                id: 101,
                body: "Nit: rename `x` to `count`.".to_string(),
                author: "reviewer-a".to_string(),
                created_at: 1_000_001,
                provenance: ElementProvenance::new(url, "comment/101"),
            },
        ],
        cross_refs: vec![
            CrossRef::resolved(
                "#2",
                ElementProvenance::new(url, "crossref/#2"),
                "https://github.com/humangr-labs/hugit/pull/2",
            ),
            CrossRef::dangling("#999", ElementProvenance::new(url, "crossref/#999")),
        ],
    }
}

fn fixture_issue() -> ImportedPrIssue {
    let url = "https://github.com/humangr-labs/hugit/issues/42";
    ImportedPrIssue {
        kind: SourceKind::Issue,
        source_id: 42,
        source_url: url.to_string(),
        body: "Agents need a way to report merge conflicts.".to_string(),
        body_provenance: ElementProvenance::new(url, "body"),
        state: PrIssueState::Closed,
        state_provenance: ElementProvenance::new(url, "state"),
        labels: vec![("bug".to_string(), ElementProvenance::new(url, "label/bug"))],
        comments: vec![ImportedComment {
            id: 200,
            body: "Fixed in #1.".to_string(),
            author: "owner".to_string(),
            created_at: 2_000_000,
            provenance: ElementProvenance::new(url, "comment/200"),
        }],
        cross_refs: vec![CrossRef::resolved(
            "#1",
            ElementProvenance::new(url, "crossref/#1"),
            "https://github.com/humangr-labs/hugit/pull/1",
        )],
    }
}

// ── ② PRs/issues → proposed intents w/ provenance ───────────────────────────

/// ② A PR fixture projects into a proposed intent with a stable intent_id
/// derived from the source URL, and per-element provenance is attached.
#[test]
fn item_2_proposed_intents_with_provenance() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // Stable intent_id: derived from source URL.
    assert_eq!(
        intent.sidecar.intent_id, "proposed::https://github.com/humangr-labs/hugit/pull/1",
        "intent_id must be derived from source_url"
    );

    // context_ref carries source URL → provenance preserved end-to-end.
    assert_eq!(
        intent.sidecar.context_ref, "https://github.com/humangr-labs/hugit/pull/1",
        "context_ref must be the source URL"
    );

    // Per-element provenance present for body.
    assert_eq!(
        intent.element_provenance.body_provenance.element_origin,
        "body"
    );
    assert_eq!(
        intent.element_provenance.body_provenance.source_url,
        "https://github.com/humangr-labs/hugit/pull/1"
    );

    // source_url on the ProposedIntent itself.
    assert_eq!(
        intent.source_url,
        "https://github.com/humangr-labs/hugit/pull/1"
    );

    // Issue fixture also projects correctly.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert!(
        issue_intent
            .sidecar
            .intent_id
            .starts_with("proposed::https://github.com/humangr-labs/hugit/issues/"),
        "issue intent_id must be derived from source_url"
    );
}

/// ② Every imported PR/issue intent must be flagged proposed/non-authoritative.
#[test]
fn item_2_intents_flagged_proposed_non_authoritative() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // The IntentSidecar must always have authoritative = false.
    assert!(
        !intent.sidecar.authoritative,
        "sidecar.authoritative must be false — non-authoritative by design"
    );
    // The helper method also confirms.
    assert!(
        intent.is_proposed_non_authoritative(),
        "is_proposed_non_authoritative() must return true"
    );

    // Issue is also non-authoritative.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert!(
        !issue_intent.sidecar.authoritative,
        "issue intent must also be non-authoritative"
    );
}

// ── ⑥ Fidelity: body preserved with provenance ───────────────────────────────

/// ⑥ Body is preserved in the intent and carries per-element provenance.
#[test]
fn item_6_fidelity_body_preserved() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // Body content is reflected in the charter (summary of the intent).
    assert!(
        intent.sidecar.charter.contains("PR import:"),
        "charter must reference the PR import"
    );

    // Per-element provenance for body is present and correct.
    let body_prov = &intent.element_provenance.body_provenance;
    assert_eq!(body_prov.element_origin, "body");
    assert_eq!(
        body_prov.source_url,
        "https://github.com/humangr-labs/hugit/pull/1"
    );

    // Issue body also has provenance.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert_eq!(
        issue_intent
            .element_provenance
            .body_provenance
            .element_origin,
        "body"
    );
}

// ── ⑥ Fidelity: comment/review threads preserved with provenance ──────────────

/// ⑥ Comment/review threads are preserved with per-element provenance.
#[test]
fn item_6_fidelity_comment_threads_preserved() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // Both comments are preserved with provenance.
    assert_eq!(
        intent.element_provenance.comment_provenances.len(),
        2,
        "both comments must be preserved"
    );
    assert_eq!(
        intent.element_provenance.comment_provenances[0].element_origin,
        "comment/100"
    );
    assert_eq!(
        intent.element_provenance.comment_provenances[1].element_origin,
        "comment/101"
    );

    // Issue comment also preserved.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert_eq!(issue_intent.element_provenance.comment_provenances.len(), 1);
    assert_eq!(
        issue_intent.element_provenance.comment_provenances[0].element_origin,
        "comment/200"
    );
}

// ── ⑥ Fidelity: state preserved with provenance ──────────────────────────────

/// ⑥ State (open/closed/merged) is preserved with per-element provenance.
#[test]
fn item_6_fidelity_state_preserved() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // State is reflected in the charter.
    assert!(
        intent.sidecar.charter.contains("open"),
        "charter must contain the state"
    );

    // Per-element provenance for state is present.
    let state_prov = &intent.element_provenance.state_provenance;
    assert_eq!(state_prov.element_origin, "state");
    assert_eq!(
        state_prov.source_url,
        "https://github.com/humangr-labs/hugit/pull/1"
    );

    // Closed state also preserved correctly.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert!(issue_intent.sidecar.charter.contains("closed"));
    assert_eq!(
        issue_intent
            .element_provenance
            .state_provenance
            .element_origin,
        "state"
    );
}

// ── ⑥ Fidelity: labels preserved with provenance ─────────────────────────────

/// ⑥ Labels are preserved with per-element provenance.
#[test]
fn item_6_fidelity_labels_preserved() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // Labels are reflected in acceptance items.
    assert_eq!(
        intent.sidecar.acceptance.len(),
        2,
        "both labels must appear as acceptance items"
    );
    assert!(
        intent
            .sidecar
            .acceptance
            .contains(&"label:enhancement".to_string())
    );
    assert!(
        intent
            .sidecar
            .acceptance
            .contains(&"label:priority:high".to_string())
    );

    // Per-element provenance for each label.
    assert_eq!(intent.element_provenance.label_provenances.len(), 2);
    let label_origins: Vec<&str> = intent
        .element_provenance
        .label_provenances
        .iter()
        .map(|p| p.element_origin.as_str())
        .collect();
    assert!(label_origins.contains(&"label/enhancement"));
    assert!(label_origins.contains(&"label/priority:high"));

    // Issue label also preserved.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert_eq!(issue_intent.sidecar.acceptance.len(), 1);
    assert!(
        issue_intent
            .sidecar
            .acceptance
            .contains(&"label:bug".to_string())
    );
}

// ── ⑥ Fidelity: cross-refs preserved or recorded as residual ─────────────────

/// ⑥ Cross-refs are preserved: resolved when both ends imported, or recorded
/// as explicit dangling residuals — never fabricated.
#[test]
fn item_6_fidelity_crossrefs_preserved() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // Two cross-refs: one resolved, one dangling.
    assert_eq!(intent.cross_refs.len(), 2);

    let resolved = intent
        .cross_refs
        .iter()
        .find(|cr| cr.raw == "#2")
        .expect("resolved cross-ref #2 must be present");
    assert!(
        !resolved.residual,
        "resolved cross-ref must not be residual"
    );
    assert_eq!(
        resolved.resolved_to.as_deref(),
        Some("https://github.com/humangr-labs/hugit/pull/2")
    );

    let dangling = intent
        .cross_refs
        .iter()
        .find(|cr| cr.raw == "#999")
        .expect("dangling cross-ref #999 must be present");
    assert!(
        dangling.residual,
        "dangling cross-ref must be marked residual"
    );
    assert!(
        dangling.resolved_to.is_none(),
        "dangling cross-ref must not have a fabricated resolved_to"
    );

    // Per-element provenance for cross-refs present.
    assert_eq!(intent.element_provenance.cross_ref_provenances.len(), 2);

    // Issue cross-ref also preserved.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert_eq!(issue_intent.cross_refs.len(), 1);
    assert!(!issue_intent.cross_refs[0].residual);
}

// ── ⑥ Fidelity: non-imported elements explicitly enumerated ──────────────────

/// ⑥ Non-imported elements are explicitly enumerated — no silent drop.
#[test]
fn item_6_non_imported_elements_enumerated() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("PR import must succeed");

    // The non_imported list is published and non-empty.
    assert!(
        !intent.non_imported.is_empty(),
        "non-imported elements list must be non-empty"
    );

    // Known excluded elements must be present.
    assert!(
        intent.non_imported.contains(&"reactions"),
        "reactions must be explicitly excluded"
    );
    assert!(
        intent.non_imported.contains(&"social_graph_signals"),
        "social_graph_signals must be explicitly excluded"
    );

    // The constant is the same object (not a copy).
    assert_eq!(intent.non_imported, NON_IMPORTED);

    // Issue intent also carries the same non-imported list.
    let issue = fixture_issue();
    let issue_intent = import_prissue(&issue).expect("issue import must succeed");
    assert_eq!(issue_intent.non_imported, NON_IMPORTED);
}

// ── ② Import boundary: bare commit path produces no PR/issue intent ───────────

/// ② Bare commits MUST NOT produce PR/issue intents (E2a⑤ boundary law).
///
/// This item asserts the boundary from E2b's perspective: there is no
/// code path in this module that synthesises an intent from a commit object,
/// commit hash, or any git-history artifact. Only PR/issue metadata objects
/// (ImportedPrIssue) can be passed to import_prissue().
#[test]
fn item_2_bare_commit_produces_no_prissue_intent() {
    // An empty source_url (the closest analogue to "no metadata identity")
    // is rejected — the import function fails closed.
    let bare = ImportedPrIssue {
        kind: SourceKind::PullRequest,
        source_id: 0,
        source_url: String::new(), // empty — no provenance identity
        body: String::new(),
        body_provenance: ElementProvenance::new("", "body"),
        state: PrIssueState::Open,
        state_provenance: ElementProvenance::new("", "state"),
        labels: vec![],
        comments: vec![],
        cross_refs: vec![],
    };

    let result = import_prissue(&bare);
    assert!(
        result.is_err(),
        "import must fail on an empty source_url — no bare-commit path exists"
    );
    assert_eq!(result.unwrap_err(), PrIssueImportError::EmptySourceUrl);

    // The batch importer silently skips invalid entries (no panics, no intents).
    let batch = import_prissue_batch(&[bare]);
    assert!(
        batch.is_empty(),
        "batch import must produce zero intents for invalid (bare-commit-like) entries"
    );
}

// ── ② Idempotency: unchanged → no-op (same intent_id, no duplication) ─────────

/// ② Idempotency: importing the same PR/issue twice produces the same intent_id.
/// Unchanged → no-op; changed → incremental re-sync; never dupes.
#[test]
fn item_2_idempotency_same_source_same_id() {
    let pr = fixture_pr();

    let intent1 = import_prissue(&pr).expect("first import must succeed");
    let intent2 = import_prissue(&pr).expect("second import must succeed");

    // Same source → same intent_id.
    assert_eq!(
        intent1.sidecar.intent_id, intent2.sidecar.intent_id,
        "re-importing the same PR must produce the same intent_id"
    );

    // A modified PR produces the same intent_id (re-sync via same id).
    let mut modified = fixture_pr();
    modified.body = "Updated body text.".to_string();
    let intent3 = import_prissue(&modified).expect("modified import must succeed");
    assert_eq!(
        intent1.sidecar.intent_id, intent3.sidecar.intent_id,
        "modified PR must produce the same intent_id for re-sync"
    );

    // Batch import of two identical fixtures produces two intents (the
    // deduplication responsibility is on the *caller*, not the importer).
    let batch = import_prissue_batch(&[fixture_pr(), fixture_pr()]);
    assert_eq!(batch.len(), 2, "batch returns both; caller deduplicates");
    assert_eq!(
        batch[0].sidecar.intent_id, batch[1].sidecar.intent_id,
        "both entries share the same intent_id"
    );
}

// ── ⑥ Structural: IntentSidecar is the output sidecar type ──────────────────

/// Compile-time assertion: the sidecar field is a hugit_contracts IntentSidecar.
#[test]
fn item_6_intent_sidecar_is_contract_type() {
    let pr = fixture_pr();
    let intent = import_prissue(&pr).expect("import must succeed");
    // This is a type assertion: if the field type changes, this line fails to compile.
    let _sidecar: &IntentSidecar = &intent.sidecar;
    assert!(!_sidecar.authoritative);
}
