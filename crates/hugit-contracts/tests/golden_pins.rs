//! Hand-authored byte-level pins for `ContextEnvelope` — the **de-laundered
//! oracle** (SOTA-audit Tier-1 S6, WA4).
//!
//! The problem this file fixes: the rest of the golden suite is generated AND
//! verified by the same code path (`gen_fixtures` writes the goldens;
//! `integration_tests` round-trips them through the *same* serde derive).
//! `UPDATE_SCHEMAS=1` is a one-command laundry — if the serializer drifts, the
//! generator drifts with it and the round-trip stays green. The generator is
//! its own oracle.
//!
//! This file breaks that loop. Everything here is **HAND-AUTHORED**: the JSON
//! string literal below was typed by a human author, NOT emitted by
//! `gen_fixtures`. The pins assert:
//!
//! 1. the exact serialized field-name SET and ORDER of a minimal envelope
//!    (`pin_context_envelope_field_order`),
//! 2. the [`CONTEXT_ENVELOPE_SCHEMA_VERSION`] literal
//!    (`pin_schema_version_literal`),
//! 3. **the tripwire**: the value a producer constructs for that same minimal
//!    envelope, serialized with the same `to_string_pretty` the generator uses,
//!    byte-equals the hand-written literal (`tripwire_serializer_matches_hand_pin`).
//!    The generator is now checked AGAINST an external author, not itself: any
//!    serde/field-order/money-type drift fails HERE before the laundered
//!    goldens can absorb it.
//!
//! ## Maintenance law (do not violate)
//!
//! `UPDATE_SCHEMAS=1 cargo run -p hugit-contracts --bin gen_fixtures`
//! regenerates the *generated* fixtures under `tests/golden/`. **THIS file is
//! updated only by hand, deliberately.** Never paste generator output in here:
//! that would re-launder the oracle. When the envelope shape changes, a human
//! re-derives the literal below by reading the contract, and the tripwire
//! confirms the serializer agrees.

use hugit_contracts::context_envelope::{
    Altitude, Authorship, CONTEXT_ENVELOPE_SCHEMA_VERSION, ContextEnvelope, IntentMetrics,
    Snapshot, Spawn, TokenCounts, Trajectory,
};

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
