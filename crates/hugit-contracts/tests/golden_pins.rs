//! Hand-authored byte-level pins for frozen security-critical contracts — the
//! **de-laundered oracle** (SOTA-audit Tier-1 S6, WA4; extended WP-E-PINS).
//!
//! The problem this file fixes: the rest of the golden suite is generated AND
//! verified by the same code path (`gen_fixtures` writes the goldens;
//! `integration_tests` round-trips them through the *same* serde derive).
//! `UPDATE_SCHEMAS=1` is a one-command laundry — if the serializer drifts, the
//! generator drifts with it and the round-trip stays green. The generator is
//! its own oracle.
//!
//! This file breaks that loop. Everything here is **HAND-AUTHORED**: the JSON
//! string literals below were typed by a human author, NOT emitted by
//! `gen_fixtures`. The pins assert:
//!
//! 1. the exact serialized field-name SET and ORDER of a minimal instance
//!    (one `pin_*_field_order` test per type),
//! 2. any security-critical literal values (schema version, sentinel names),
//! 3. **the tripwire**: the value a producer constructs for that same minimal
//!    instance, serialized with the same `to_string_pretty` the generator uses,
//!    byte-equals the hand-written literal (`tripwire_*` per type).
//!    The generator is now checked AGAINST an external author, not itself: any
//!    serde/field-order/type drift fails HERE before the laundered goldens can
//!    absorb it.
//!
//! ## Types pinned (WA4 + WP-E-PINS)
//!
//! - `ContextEnvelope` (WA4) — ADR-0001 context envelope with money micro-USD
//! - `VerdictObject` (WP-E-PINS) — review lens verdict; security-critical
//! - `RunnerLease` (WP-E-PINS) — scoped filesystem access lease; security-critical
//! - `FenceManifest` (WP-E-PINS) — sparse materialization fence; security-critical
//! - `AttestationChain` (WP-E-PINS) — provenance chain with Ed25519 sig; security-critical
//! - `ExportSchema` (WP-E-PINS) — versioned export envelope; security-critical
//!
//! ## Maintenance law (do not violate)
//!
//! `UPDATE_SCHEMAS=1 cargo run -p hugit-contracts --bin gen_fixtures`
//! regenerates the *generated* fixtures under `tests/golden/`. **THIS file is
//! updated only by hand, deliberately.** Never paste generator output in here:
//! that would re-launder the oracle. When a contract shape changes, a human
//! re-derives the literal below by reading the contract, and the tripwire
//! confirms the serializer agrees.

use hugit_contracts::attestation_chain::AttestationChain;
use hugit_contracts::context_envelope::{
    Altitude, Authorship, CONTEXT_ENVELOPE_SCHEMA_VERSION, ContextEnvelope, IntentMetrics,
    Snapshot, Spawn, TokenCounts, Trajectory,
};
use hugit_contracts::export_schema::ExportSchema;
use hugit_contracts::fence_manifest::{FenceManifest, MaterializedEntry};
use hugit_contracts::runner_lease::{RunnerLease, RunnerState};
use hugit_contracts::verdict_object::{Verdict, VerdictObject};

/// A minimal `ContextEnvelope`, serialized exactly. **Hand-authored** — typed
/// against the ADR-0001 §2.2 contract + WA4 money amendment (schema 1.2.0,
/// money as integer micro-USD), NOT generated. The values are deliberately
/// distinct from any `gen_fixtures` fixture (id `pin-min`, round numbers) so a
/// copy-paste from the generator is obvious. The trailing newline matches the
/// generator's `to_string_pretty(..) + "\n"` convention.
const HAND_PIN: &str = r#"{
  "schema_version": "1.2.0",
  "altitude": "intent",
  "intent_id": "pin-min",
  "commit": "0000000000000000000000000000000000000000",
  "tree_hash": "1111111111111111111111111111111111111111111111111111111111111111",
  "authorship": {
    "model": "claude-opus-4-8",
    "model_digest": "2222222222222222222222222222222222222222222222222222222222222222",
    "agent_type": "implementer",
    "spawn": {
      "run_id": "run-pin",
      "parent_run_id": null,
      "born_at": 1000,
      "died_at": 2000
    },
    "operator": "owner@example.com"
  },
  "charter": "pin: minimal hand-authored envelope",
  "campaign": null,
  "constraints": [],
  "acceptance": [],
  "parent_intents": [],
  "trajectory": {
    "raw_transcript_ref": null,
    "task_transcript_ref": null,
    "summary": null,
    "journal_ref": null,
    "redaction_policy": "default-v1"
  },
  "snapshot": {
    "files_read": [],
    "prompt_ref": null,
    "env_manifest": "rustc 1.96.0"
  },
  "metrics": {
    "tokens": {
      "input": 100,
      "output": 50,
      "cache_read": 0,
      "cache_write": 0,
      "total": 150
    },
    "wall_ms": 1000,
    "active_ms": 800,
    "tool_calls": 0,
    "tool_breakdown": [],
    "model_turns": 1,
    "cost_usd_micros": 250000
  },
  "verdicts_ref": null
}
"#;

/// Build the SAME minimal envelope as a Rust value (the data a producer would
/// construct). Kept in lock-step with [`HAND_PIN`] BY HAND — the tripwire
/// proves the serializer turns this into exactly those bytes.
fn minimal_envelope() -> ContextEnvelope {
    ContextEnvelope {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.into(),
        altitude: Altitude::Intent,
        intent_id: "pin-min".into(),
        commit: "0000000000000000000000000000000000000000".into(),
        tree_hash: "1111111111111111111111111111111111111111111111111111111111111111".into(),
        authorship: Authorship {
            model: "claude-opus-4-8".into(),
            model_digest: "2222222222222222222222222222222222222222222222222222222222222222".into(),
            agent_type: "implementer".into(),
            spawn: Spawn {
                run_id: "run-pin".into(),
                parent_run_id: None,
                born_at: 1000,
                died_at: 2000,
            },
            operator: "owner@example.com".into(),
        },
        charter: "pin: minimal hand-authored envelope".into(),
        campaign: None,
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        trajectory: Trajectory {
            raw_transcript_ref: None,
            task_transcript_ref: None,
            summary: None,
            journal_ref: None,
            redaction_policy: "default-v1".into(),
        },
        snapshot: Snapshot {
            files_read: vec![],
            prompt_ref: None,
            env_manifest: "rustc 1.96.0".into(),
        },
        metrics: IntentMetrics {
            tokens: TokenCounts {
                input: 100,
                output: 50,
                cache_read: 0,
                cache_write: 0,
                total: 150,
            },
            wall_ms: 1000,
            active_ms: 800,
            tool_calls: 0,
            tool_breakdown: vec![],
            model_turns: 1,
            cost_usd_micros: 250_000,
        },
        verdicts_ref: None,
    }
}

/// The exact ordered field-name set of a serialized `ContextEnvelope@1.2.0`,
/// hand-listed (top level + the `metrics` block where the money rename lives).
/// Asserting the literal NAMES — not just a round-trip — catches a rename or a
/// reorder that a value-level round-trip would silently tolerate.
#[test]
fn pin_context_envelope_field_order() {
    // Top-level keys, in serialization order.
    let top: Vec<&str> = HAND_PIN
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            // top-level keys are indented exactly two spaces.
            if l.starts_with("  ") && !l.starts_with("   ") && t.contains(':') {
                t.split(':').next().map(|k| k.trim_matches('"'))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        top,
        vec![
            "schema_version",
            "altitude",
            "intent_id",
            "commit",
            "tree_hash",
            "authorship",
            "charter",
            "campaign",
            "constraints",
            "acceptance",
            "parent_intents",
            "trajectory",
            "snapshot",
            "metrics",
            "verdicts_ref",
        ],
        "ContextEnvelope top-level field set/order drifted from the hand pin"
    );
    // The money field MUST be the integer micro-USD name (WA4 / 1.2.0), and the
    // old f64 name MUST be gone — a one-line guard against a silent revert.
    assert!(
        HAND_PIN.contains("\"cost_usd_micros\": 250000"),
        "money must serialize as integer micro-USD `cost_usd_micros`"
    );
    assert!(
        !HAND_PIN.contains("\"cost_usd\":"),
        "the f64 `cost_usd` field must not reappear (WA4 break)"
    );
}

/// Pin the schema-version literal independently of any generated artifact.
#[test]
fn pin_schema_version_literal() {
    assert_eq!(
        CONTEXT_ENVELOPE_SCHEMA_VERSION, "1.2.0",
        "CONTEXT_ENVELOPE_SCHEMA_VERSION drifted from the hand pin (WA4 = 1.2.0)"
    );
}

/// THE TRIPWIRE: the producer-constructed value, serialized with the SAME
/// `to_string_pretty(..) + "\n"` the generator uses, must byte-equal the
/// hand-written literal. This checks the serde serializer against an EXTERNAL
/// author — not against itself — so any field rename / reorder / money-type
/// drift fails here before the (laundered) generated goldens can absorb it.
#[test]
fn tripwire_serializer_matches_hand_pin() {
    let serialized = serde_json::to_string_pretty(&minimal_envelope()).unwrap() + "\n";
    assert_eq!(
        serialized, HAND_PIN,
        "serializer output diverged from the HAND-AUTHORED pin — the generator \
         is no longer its own oracle; update the contract or the hand pin BY HAND"
    );
}

// ============================================================================
// VerdictObject — WP-E-PINS
// ============================================================================

/// Minimal `VerdictObject`, serialized exactly. **Hand-authored** against the
/// type definition in `crates/hugit-contracts/src/verdict_object.rs` (frozen
/// WP-00). Field order: `intent`, `tree_hash`, `lens`, `model`,
/// `prompt_digest`, `verdict`, `claims_checked`, `evidence_refs`.
/// `verdict` enum uses `snake_case` rename: `Approve` → `"approve"`.
/// Values are deliberately distinct from any generated fixture.
const VERDICT_OBJECT_PIN: &str = r#"{
  "intent": "pin-verdict-intent",
  "tree_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "lens": "security-review-v1",
  "model": "claude-opus-4-8",
  "prompt_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "verdict": "approve",
  "claims_checked": [
    "no-unsafe-paths",
    "no-secret-leak"
  ],
  "evidence_refs": [
    "cas:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  ]
}
"#;

/// Build the SAME minimal `VerdictObject` as a Rust value. Kept in lock-step
/// with [`VERDICT_OBJECT_PIN`] BY HAND.
fn minimal_verdict_object() -> VerdictObject {
    VerdictObject {
        intent: "pin-verdict-intent".into(),
        tree_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        lens: "security-review-v1".into(),
        model: "claude-opus-4-8".into(),
        prompt_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        verdict: Verdict::Approve,
        claims_checked: vec!["no-unsafe-paths".into(), "no-secret-leak".into()],
        evidence_refs: vec![
            "cas:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
        ],
    }
}

/// Assert the exact top-level field-name set and order of `VerdictObject`.
#[test]
fn pin_verdict_object_field_order() {
    let top: Vec<&str> = VERDICT_OBJECT_PIN
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if l.starts_with("  ") && !l.starts_with("   ") && t.contains(':') {
                t.split(':').next().map(|k| k.trim_matches('"'))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        top,
        vec![
            "intent",
            "tree_hash",
            "lens",
            "model",
            "prompt_digest",
            "verdict",
            "claims_checked",
            "evidence_refs",
        ],
        "VerdictObject top-level field set/order drifted from the hand pin"
    );
    // The verdict enum MUST use snake_case serialization.
    assert!(
        VERDICT_OBJECT_PIN.contains("\"verdict\": \"approve\""),
        "VerdictObject.verdict must serialize as snake_case `approve`"
    );
    assert!(
        !VERDICT_OBJECT_PIN.contains("\"Approve\""),
        "VerdictObject.verdict must NOT serialize as PascalCase"
    );
}

/// THE TRIPWIRE for `VerdictObject`: producer value byte-equals the
/// hand-written literal.
#[test]
fn tripwire_verdict_object_matches_hand_pin() {
    let serialized = serde_json::to_string_pretty(&minimal_verdict_object()).unwrap() + "\n";
    assert_eq!(
        serialized, VERDICT_OBJECT_PIN,
        "VerdictObject serializer output diverged from the HAND-AUTHORED pin \
         — update the contract or the hand pin BY HAND"
    );
}

// ============================================================================
// RunnerLease — WP-E-PINS
// ============================================================================

/// Minimal `RunnerLease`, serialized exactly. **Hand-authored** against
/// `crates/hugit-contracts/src/runner_lease.rs` (frozen WP-00). Field order:
/// `lease_id`, `principal_chain`, `path_set`, `expiry`, `net_policy`,
/// `tmp_root`, `state`. `RunnerState` enum `snake_case`: `Held` → `"held"`.
const RUNNER_LEASE_PIN: &str = r#"{
  "lease_id": "pin-lease-0001",
  "principal_chain": [
    "agent:pin-agent-0001"
  ],
  "path_set": [
    "/workspace/pin"
  ],
  "expiry": 9000000000,
  "net_policy": "deny-all-v1",
  "tmp_root": "/tmp/pin-runner-root",
  "state": "held"
}
"#;

/// Build the SAME minimal `RunnerLease` as a Rust value. Kept in lock-step
/// with [`RUNNER_LEASE_PIN`] BY HAND.
fn minimal_runner_lease() -> RunnerLease {
    RunnerLease {
        lease_id: "pin-lease-0001".into(),
        principal_chain: vec!["agent:pin-agent-0001".into()],
        path_set: vec!["/workspace/pin".into()],
        expiry: 9_000_000_000,
        net_policy: "deny-all-v1".into(),
        tmp_root: "/tmp/pin-runner-root".into(),
        state: RunnerState::Held,
    }
}

/// Assert the exact top-level field-name set and order of `RunnerLease`.
#[test]
fn pin_runner_lease_field_order() {
    let top: Vec<&str> = RUNNER_LEASE_PIN
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if l.starts_with("  ") && !l.starts_with("   ") && t.contains(':') {
                t.split(':').next().map(|k| k.trim_matches('"'))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        top,
        vec![
            "lease_id",
            "principal_chain",
            "path_set",
            "expiry",
            "net_policy",
            "tmp_root",
            "state",
        ],
        "RunnerLease top-level field set/order drifted from the hand pin"
    );
    // The state enum MUST use snake_case serialization.
    assert!(
        RUNNER_LEASE_PIN.contains("\"state\": \"held\""),
        "RunnerLease.state must serialize as snake_case `held`"
    );
    assert!(
        !RUNNER_LEASE_PIN.contains("\"Held\""),
        "RunnerLease.state must NOT serialize as PascalCase"
    );
}

/// THE TRIPWIRE for `RunnerLease`: producer value byte-equals the
/// hand-written literal.
#[test]
fn tripwire_runner_lease_matches_hand_pin() {
    let serialized = serde_json::to_string_pretty(&minimal_runner_lease()).unwrap() + "\n";
    assert_eq!(
        serialized, RUNNER_LEASE_PIN,
        "RunnerLease serializer output diverged from the HAND-AUTHORED pin \
         — update the contract or the hand pin BY HAND"
    );
}

// ============================================================================
// FenceManifest — WP-E-PINS
// ============================================================================

/// Minimal `FenceManifest`, serialized exactly. **Hand-authored** against
/// `crates/hugit-contracts/src/fence_manifest.rs` (frozen WP-00). Field order:
/// `path_set`, `deny_default`, `materialized`. `MaterializedEntry` fields:
/// `path`, `digest`. `deny_default` MUST be `true` in all production manifests.
const FENCE_MANIFEST_PIN: &str = r#"{
  "path_set": [
    "/workspace/pin/src/lib.rs"
  ],
  "deny_default": true,
  "materialized": [
    {
      "path": "/workspace/pin/src/lib.rs",
      "digest": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
    }
  ]
}
"#;

/// Build the SAME minimal `FenceManifest` as a Rust value. Kept in lock-step
/// with [`FENCE_MANIFEST_PIN`] BY HAND.
fn minimal_fence_manifest() -> FenceManifest {
    FenceManifest {
        path_set: vec!["/workspace/pin/src/lib.rs".into()],
        deny_default: true,
        materialized: vec![MaterializedEntry {
            path: "/workspace/pin/src/lib.rs".into(),
            digest: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into(),
        }],
    }
}

/// Assert the exact top-level field-name set and order of `FenceManifest`,
/// and that `deny_default` serializes as the boolean `true` (not a string).
#[test]
fn pin_fence_manifest_field_order() {
    let top: Vec<&str> = FENCE_MANIFEST_PIN
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if l.starts_with("  ") && !l.starts_with("   ") && t.contains(':') {
                t.split(':').next().map(|k| k.trim_matches('"'))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        top,
        vec!["path_set", "deny_default", "materialized"],
        "FenceManifest top-level field set/order drifted from the hand pin"
    );
    // The security-critical `deny_default` flag MUST serialize as JSON boolean
    // true, never as the string `"true"`.
    assert!(
        FENCE_MANIFEST_PIN.contains("\"deny_default\": true"),
        "FenceManifest.deny_default must serialize as JSON boolean `true`"
    );
    assert!(
        !FENCE_MANIFEST_PIN.contains("\"deny_default\": \"true\""),
        "FenceManifest.deny_default must NOT serialize as a string"
    );
}

/// THE TRIPWIRE for `FenceManifest`: producer value byte-equals the
/// hand-written literal.
#[test]
fn tripwire_fence_manifest_matches_hand_pin() {
    let serialized = serde_json::to_string_pretty(&minimal_fence_manifest()).unwrap() + "\n";
    assert_eq!(
        serialized, FENCE_MANIFEST_PIN,
        "FenceManifest serializer output diverged from the HAND-AUTHORED pin \
         — update the contract or the hand pin BY HAND"
    );
}

// ============================================================================
// AttestationChain — WP-E-PINS
// ============================================================================

/// Minimal `AttestationChain`, serialized exactly. **Hand-authored** against
/// `crates/hugit-contracts/src/attestation_chain.rs` (frozen WP-00). Field
/// order (struct order): `tree`, `def`, `runner`, `model`, `principal`, `sig`.
/// The `sig` field is a base64-encoded Ed25519 signature placeholder —
/// deliberately a well-formed base64 string, not real output.
const ATTESTATION_CHAIN_PIN: &str = r#"{
  "tree": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  "def": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
  "runner": "runner:pin-runner-0001",
  "model": "claude-opus-4-8",
  "principal": [
    "agent:pin-agent-0001",
    "user:pin-owner@example.com"
  ],
  "sig": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
}
"#;

/// Build the SAME minimal `AttestationChain` as a Rust value. Kept in
/// lock-step with [`ATTESTATION_CHAIN_PIN`] BY HAND.
fn minimal_attestation_chain() -> AttestationChain {
    AttestationChain {
        tree: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
        def: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
        runner: "runner:pin-runner-0001".into(),
        model: "claude-opus-4-8".into(),
        principal: vec![
            "agent:pin-agent-0001".into(),
            "user:pin-owner@example.com".into(),
        ],
        // 66 'A' chars = 66 bytes = valid base64 placeholder (88 chars when base64-encoded)
        sig: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
    }
}

/// Assert the exact top-level field-name set and order of `AttestationChain`
/// (struct order is the provenance pre-image order: tree, def, runner, model,
/// principal, sig — any reorder would break the Ed25519 verifier).
#[test]
fn pin_attestation_chain_field_order() {
    let top: Vec<&str> = ATTESTATION_CHAIN_PIN
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if l.starts_with("  ") && !l.starts_with("   ") && t.contains(':') {
                t.split(':').next().map(|k| k.trim_matches('"'))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        top,
        vec!["tree", "def", "runner", "model", "principal", "sig"],
        "AttestationChain top-level field set/order drifted from the hand pin \
         — this breaks the Ed25519 pre-image ordering"
    );
    // `sig` MUST be present — absence would silently drop the integrity check.
    assert!(
        ATTESTATION_CHAIN_PIN.contains("\"sig\":"),
        "AttestationChain.sig field must be present in serialized output"
    );
}

/// THE TRIPWIRE for `AttestationChain`: producer value byte-equals the
/// hand-written literal.
#[test]
fn tripwire_attestation_chain_matches_hand_pin() {
    let serialized = serde_json::to_string_pretty(&minimal_attestation_chain()).unwrap() + "\n";
    assert_eq!(
        serialized, ATTESTATION_CHAIN_PIN,
        "AttestationChain serializer output diverged from the HAND-AUTHORED pin \
         — update the contract or the hand pin BY HAND"
    );
}

// ============================================================================
// ExportSchema — WP-E-PINS
// ============================================================================

/// Minimal `ExportSchema`, serialized exactly. **Hand-authored** against
/// `crates/hugit-contracts/src/export_schema.rs` (frozen WP-00). Field order:
/// `version`, `object_classes`, `redaction_manifest`.
const EXPORT_SCHEMA_PIN: &str = r#"{
  "version": "1.0.0",
  "object_classes": [
    "ContextEnvelope",
    "VerdictObject"
  ],
  "redaction_manifest": "cas:0000000000000000000000000000000000000000000000000000000000000000"
}
"#;

/// Build the SAME minimal `ExportSchema` as a Rust value. Kept in lock-step
/// with [`EXPORT_SCHEMA_PIN`] BY HAND.
fn minimal_export_schema() -> ExportSchema {
    ExportSchema {
        version: "1.0.0".into(),
        object_classes: vec!["ContextEnvelope".into(), "VerdictObject".into()],
        redaction_manifest: "cas:0000000000000000000000000000000000000000000000000000000000000000"
            .into(),
    }
}

/// Assert the exact top-level field-name set and order of `ExportSchema`.
#[test]
fn pin_export_schema_field_order() {
    let top: Vec<&str> = EXPORT_SCHEMA_PIN
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if l.starts_with("  ") && !l.starts_with("   ") && t.contains(':') {
                t.split(':').next().map(|k| k.trim_matches('"'))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        top,
        vec!["version", "object_classes", "redaction_manifest"],
        "ExportSchema top-level field set/order drifted from the hand pin"
    );
    // `redaction_manifest` MUST be present — it is the audit trail for field
    // redaction; absence would silently drop accountability.
    assert!(
        EXPORT_SCHEMA_PIN.contains("\"redaction_manifest\":"),
        "ExportSchema.redaction_manifest must be present in serialized output"
    );
}

/// THE TRIPWIRE for `ExportSchema`: producer value byte-equals the
/// hand-written literal.
#[test]
fn tripwire_export_schema_matches_hand_pin() {
    let serialized = serde_json::to_string_pretty(&minimal_export_schema()).unwrap() + "\n";
    assert_eq!(
        serialized, EXPORT_SCHEMA_PIN,
        "ExportSchema serializer output diverged from the HAND-AUTHORED pin \
         — update the contract or the hand pin BY HAND"
    );
}
