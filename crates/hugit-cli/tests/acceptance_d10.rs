//! Acceptance suite for WP-D10 — `hugit why` + `hugit impact`.
//!
//! Items (verbatim from decomposition v2.0 D10①–④):
//! ① hugit why <line|symbol> → originating intent + charter/author/model/cost, matching event log
//! ② hugit impact <path|change> → golden affected-set on known build graph
//! ③ impact feeds verdict-panel ground truth (cross-check)
//! ④ (R6) why on regenerated/derived bytes resolves honestly to the regen/derivation event
//!    — never fabricates or mis-attributes a human author

use std::collections::BTreeSet;

use hugit_checks::affected::{BuildGraph, Ecosystem, PackageNode};
use hugit_cli::impact::{ImpactQuery, compute_impact, export_ground_truth};
use hugit_cli::why::resolver::{INTENT_LANDED_KIND, LogEntry, REGEN_DERIVED_KIND, fixture_event};
use hugit_cli::why::{AuthorKind, WhyQuery, resolve_why};
use hugit_contracts::AttestationChain;

// ---------------------------------------------------------------------------
// Shared fixture builders
// ---------------------------------------------------------------------------

/// Build the known build graph used in ②③.
fn known_build_graph() -> BuildGraph {
    BuildGraph {
        ecosystem: Ecosystem::Cargo,
        root_manifests: vec!["Cargo.toml".to_string()],
        packages: vec![
            PackageNode {
                name: "hugit-contracts".to_string(),
                path: "crates/hugit-contracts".to_string(),
                direct_deps: vec![],
            },
            PackageNode {
                name: "hugit-checks".to_string(),
                path: "crates/hugit-checks".to_string(),
                direct_deps: vec!["hugit-contracts".to_string()],
            },
            PackageNode {
                name: "hugit-refstore".to_string(),
                path: "crates/hugit-refstore".to_string(),
                direct_deps: vec!["hugit-contracts".to_string()],
            },
            PackageNode {
                name: "hugit-cli".to_string(),
                path: "crates/hugit-cli".to_string(),
                direct_deps: vec!["hugit-contracts".to_string(), "hugit-checks".to_string()],
            },
            PackageNode {
                name: "hugit-app".to_string(),
                path: "crates/hugit-app".to_string(),
                direct_deps: vec!["hugit-cli".to_string(), "hugit-checks".to_string()],
            },
        ],
    }
}

/// Build the event-log fixture (three events).
///
/// - seq=1: `intent.landed` for `src/auth/mod.rs` — human author alice.
/// - seq=2: `intent.landed` for `src/flags/mod.rs` — human author bob.
/// - seq=3: `regen.derived` for `Cargo.lock` — machine author / regen runner.
fn event_log_fixture() -> Vec<LogEntry> {
    let e1 = fixture_event(
        1,
        INTENT_LANDED_KIND,
        vec!["alice@example.com".to_string()],
        serde_json::json!({
            "intent_id": "intent-abc-001",
            "ref": "refs/heads/main",
            "target": "deadbeef0001",
            "charter": "Add authentication module",
            "path": "src/auth/mod.rs"
        }),
    );
    let attest1 = AttestationChain {
        tree: "tree-sha-001".to_string(),
        def: "0.00 USD".to_string(),
        runner: "runner-001".to_string(),
        model: "".to_string(),
        principal: vec!["alice@example.com".to_string()],
        sig: "sig-001".to_string(),
    };

    let e2 = fixture_event(
        2,
        INTENT_LANDED_KIND,
        vec!["bob@example.com".to_string()],
        serde_json::json!({
            "intent_id": "intent-abc-002",
            "ref": "refs/heads/main",
            "target": "deadbeef0002",
            "charter": "Add feature flag support",
            "path": "src/flags/mod.rs"
        }),
    );
    let attest2 = AttestationChain {
        tree: "tree-sha-002".to_string(),
        def: "0.00 USD".to_string(),
        runner: "runner-002".to_string(),
        model: "".to_string(),
        principal: vec!["bob@example.com".to_string()],
        sig: "sig-002".to_string(),
    };

    // seq=3 is the REGEN event — machine-produced, NOT a human author.
    let e3 = fixture_event(
        3,
        REGEN_DERIVED_KIND,
        vec!["runner-regen-03".to_string()],
        serde_json::json!({
            "charter": "Regenerated lockfile from Cargo.toml update",
            "path": "Cargo.lock",
            "source_intent": "intent-abc-002"
        }),
    );
    let attest3 = AttestationChain {
        tree: "tree-sha-003".to_string(),
        def: "0.00 USD".to_string(),
        runner: "runner-regen-03".to_string(),
        model: "claude-sonnet-4-6".to_string(),
        principal: vec!["runner-regen-03".to_string()],
        sig: "sig-003".to_string(),
    };

    vec![
        LogEntry {
            record: e1,
            attestation: Some(attest1),
            sidecar: None,
        },
        LogEntry {
            record: e2,
            attestation: Some(attest2),
            sidecar: None,
        },
        LogEntry {
            record: e3,
            attestation: Some(attest3),
            sidecar: None,
        },
    ]
}

// ---------------------------------------------------------------------------
// ① why → originating intent + charter/author/model/cost, matching event log
// ---------------------------------------------------------------------------

#[test]
fn item_1_why_matches_event_log() {
    let entries = event_log_fixture();

    // Query for a human-authored file.
    let q = WhyQuery {
        path: "src/auth/mod.rs".to_string(),
        line: Some(1),
        symbol: None,
    };
    let answer = resolve_why(&q, &entries).expect("why must resolve src/auth/mod.rs");

    // Must be an intent kind, NOT derived.
    assert_eq!(
        answer.author_kind,
        AuthorKind::Intent,
        "src/auth/mod.rs is authored by a human intent"
    );

    // Intent id must match the event-log entry.
    assert_eq!(
        answer.intent_id.as_deref(),
        Some("intent-abc-001"),
        "intent_id must match event log seq=1"
    );

    // Charter must match the log.
    assert_eq!(
        answer.charter, "Add authentication module",
        "charter must match event log seq=1"
    );

    // Author must match principal_chain in the log.
    assert_eq!(
        answer.author,
        vec!["alice@example.com".to_string()],
        "author must match principal_chain in event log seq=1"
    );

    // Event kind must be intent.landed.
    assert_eq!(answer.event_kind, INTENT_LANDED_KIND);

    // Event seq must match.
    assert_eq!(answer.event_seq, 1, "event_seq must match log seq=1");

    // model is blank for a human-only intent.
    assert_eq!(answer.model, "", "model must be empty for human intent");

    // Query a second file (different intent).
    let q2 = WhyQuery {
        path: "src/flags/mod.rs".to_string(),
        line: None,
        symbol: None,
    };
    let answer2 = resolve_why(&q2, &entries).expect("why must resolve src/flags/mod.rs");
    assert_eq!(answer2.intent_id.as_deref(), Some("intent-abc-002"));
    assert_eq!(answer2.author, vec!["bob@example.com".to_string()]);
    assert_eq!(answer2.event_seq, 2);
}

// ---------------------------------------------------------------------------
// ② impact → golden affected-set on known build graph
// ---------------------------------------------------------------------------

#[test]
fn item_2_impact_golden_affected_set() {
    let graph = known_build_graph();

    // Case A: change to hugit-contracts (leaf) — must affect everything downstream.
    let q_leaf = ImpactQuery {
        changed_paths: vec!["crates/hugit-contracts/src/event_record.rs".to_string()],
    };
    let result_leaf = compute_impact(&q_leaf, &graph).expect("impact must succeed");
    let golden_leaf: BTreeSet<String> = [
        "hugit-contracts",
        "hugit-checks",
        "hugit-refstore",
        "hugit-cli",
        "hugit-app",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        result_leaf.affected, golden_leaf,
        "leaf change must propagate to all dependents (set-equality)"
    );
    assert!(
        !result_leaf.is_full_set,
        "targeted change must NOT be a full set"
    );

    // Case B: change to hugit-checks (mid-graph).
    let q_mid = ImpactQuery {
        changed_paths: vec!["crates/hugit-checks/src/lib.rs".to_string()],
    };
    let result_mid = compute_impact(&q_mid, &graph).expect("impact must succeed");
    let golden_mid: BTreeSet<String> = ["hugit-checks", "hugit-cli", "hugit-app"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        result_mid.affected, golden_mid,
        "mid-graph change golden set-equality"
    );

    // Case C: change to hugit-cli (tip).
    let q_tip = ImpactQuery {
        changed_paths: vec!["crates/hugit-cli/src/lib.rs".to_string()],
    };
    let result_tip = compute_impact(&q_tip, &graph).expect("impact must succeed");
    let golden_tip: BTreeSet<String> = ["hugit-cli", "hugit-app"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        result_tip.affected, golden_tip,
        "tip change golden set-equality"
    );
}

// ---------------------------------------------------------------------------
// ③ impact feeds verdict-panel ground truth (cross-check)
// ---------------------------------------------------------------------------

#[test]
fn item_3_impact_feeds_verdict_ground_truth() {
    let graph = known_build_graph();

    // Simulate a PR-42 changing hugit-checks.
    let q = ImpactQuery {
        changed_paths: vec!["crates/hugit-checks/src/lib.rs".to_string()],
    };
    let result = compute_impact(&q, &graph).expect("impact must succeed");

    // Export as ground truth for the verdict panel.
    let ground_truth = export_ground_truth("pr-42", &result);

    // The exported record must carry the same affected set.
    let golden: BTreeSet<String> = ["hugit-checks", "hugit-cli", "hugit-app"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        ground_truth.affected, golden,
        "exported ground truth must match impact result"
    );
    assert_eq!(ground_truth.change_id, "pr-42");
    assert!(!ground_truth.is_full_set);

    // The exported record is structurally equivalent to the ImpactResult
    // (the seam D10 → D7: D10 owns export, D7 consumes).
    assert_eq!(ground_truth.affected, result.affected);
    assert_eq!(ground_truth.is_full_set, result.is_full_set);
    assert_eq!(ground_truth.full_set_reason, result.full_set_reason);
}

// ---------------------------------------------------------------------------
// ④ (R6) why on regenerated/derived bytes resolves honestly to regen event
// ---------------------------------------------------------------------------

#[test]
fn item_4_why_derived_bytes_honest() {
    let entries = event_log_fixture();

    // Query for the derived file (Cargo.lock — seq=3 is a regen.derived event).
    let q = WhyQuery {
        path: "Cargo.lock".to_string(),
        line: None,
        symbol: None,
    };
    let answer = resolve_why(&q, &entries).expect("why must resolve Cargo.lock");

    // MUST resolve to the regen event, NOT a human intent.
    match &answer.author_kind {
        AuthorKind::Derived { event_kind } => {
            assert_eq!(
                event_kind, REGEN_DERIVED_KIND,
                "event_kind must be regen.derived for machine-produced file"
            );
        }
        AuthorKind::Intent => {
            panic!(
                "R6 violation: why on derived bytes MUST NOT return AuthorKind::Intent \
                 (would fabricate a human author for machine-produced bytes)"
            );
        }
    }

    // Must NOT have a human intent_id.
    assert!(
        answer.intent_id.is_none(),
        "derived bytes must not carry a human intent_id"
    );

    // The principal must be the regen runner, not a human.
    assert!(
        answer
            .author
            .iter()
            .all(|a| a.contains("runner") || a.contains("regen")),
        "derived-bytes author must be a regen runner, not a human: {:?}",
        answer.author
    );

    // Event seq must be 3 (the regen event), never 1 or 2 (the human intents).
    assert_eq!(
        answer.event_seq, 3,
        "derived bytes must resolve to seq=3 (regen event), not a human-authored event"
    );

    // The model field must be populated (regen events use a model).
    assert_eq!(
        answer.model, "claude-sonnet-4-6",
        "regen event must carry the model that produced the derivation"
    );
}
