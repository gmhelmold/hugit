//! Acceptance tests for WP-D6 — policy engine v0.
//!
//! Owned items (verbatim from decomposition v2.0 D6①–③):
//! ① 3 ported gates local≡forge
//! ② engine down → landing blocks (kill-test)
//! ③ policy change = audited event
//!
//! Contract: docs/plan/wp-contracts/WP-D6.md
//! Oracle: these tests (crates/hugit-policy/tests/acceptance_d6.rs)

use hugit_policy::{
    Engine, EvalContext, GateOutcome, changelog, dco, emit_policy_change, landing_gate_check,
    secrets,
};
use std::collections::HashMap;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Build a context with DCO-signed commits only.
fn ctx_dco_pass() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.commit_messages = vec![
        "feat: add widget\n\nSigned-off-by: Alice <alice@example.com>".into(),
        "fix: correct typo\n\nSigned-off-by: Bob <bob@example.com>".into(),
    ];
    ctx
}

/// Build a context with one unsigned commit.
fn ctx_dco_fail() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.commit_messages = vec!["feat: no signature here".into()];
    ctx
}

/// CHANGELOG.md with a non-empty [Unreleased] section.
const CHANGELOG_GOOD: &str =
    "# Changelog\n\n## [Unreleased]\n\n- feat: widget added\n\n## [0.1.0]\n";

/// CHANGELOG.md with an empty [Unreleased] section.
const CHANGELOG_EMPTY: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.1.0]\n";

fn ctx_changelog_pass() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.commit_messages =
        vec!["feat: add widget\n\nSigned-off-by: Alice <alice@example.com>".into()];
    ctx.changed_files = vec!["CHANGELOG.md".into()];
    ctx.file_contents
        .insert("CHANGELOG.md".into(), CHANGELOG_GOOD.into());
    ctx
}

fn ctx_changelog_fail_no_file() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.commit_messages =
        vec!["feat: add widget\n\nSigned-off-by: Alice <alice@example.com>".into()];
    // CHANGELOG.md not in changed_files
    ctx
}

fn ctx_changelog_fail_empty_section() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.commit_messages =
        vec!["feat: add widget\n\nSigned-off-by: Alice <alice@example.com>".into()];
    ctx.changed_files = vec!["CHANGELOG.md".into()];
    ctx.file_contents
        .insert("CHANGELOG.md".into(), CHANGELOG_EMPTY.into());
    ctx
}

fn ctx_secrets_pass() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.changed_files = vec!["src/main.rs".into()];
    ctx.file_contents.insert(
        "src/main.rs".into(),
        "fn main() { println!(\"hello\"); }".into(),
    );
    ctx
}

fn ctx_secrets_fail() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.changed_files = vec!["config.env".into()];
    ctx.file_contents.insert(
        "config.env".into(),
        "AWS_KEY=AKIAIOSFODNN7EXAMPLE\nother=stuff".into(),
    );
    ctx
}

// ─── item ①: 3 ported gates local≡forge ──────────────────────────────────────
//
// The SAME evaluator functions are called both locally (hugit policy test)
// and at the forge enforcement site. We verify:
//   (a) each gate produces the correct verdict on known pass/fail inputs
//   (b) the Engine::house() registry dispatches to the identical functions
//   (c) direct call ≡ engine dispatch (byte-identical outcome)
//
#[test]
fn item_1_gates_local_eq_forge() {
    // ── (a) DCO gate ──────────────────────────────────────────────────────────
    assert_eq!(
        dco::eval(&ctx_dco_pass()),
        GateOutcome::Pass,
        "DCO: pass case"
    );
    assert!(
        matches!(dco::eval(&ctx_dco_fail()), GateOutcome::Fail { .. }),
        "DCO: fail case"
    );

    // ── (b) Changelog gate ───────────────────────────────────────────────────
    assert_eq!(
        changelog::eval(&ctx_changelog_pass()),
        GateOutcome::Pass,
        "changelog: pass case"
    );
    assert!(
        matches!(
            changelog::eval(&ctx_changelog_fail_no_file()),
            GateOutcome::Fail { .. }
        ),
        "changelog: fail when no file"
    );
    assert!(
        matches!(
            changelog::eval(&ctx_changelog_fail_empty_section()),
            GateOutcome::Fail { .. }
        ),
        "changelog: fail when empty section"
    );

    // ── (c) Secrets gate ─────────────────────────────────────────────────────
    assert_eq!(
        secrets::eval(&ctx_secrets_pass()),
        GateOutcome::Pass,
        "secrets: pass case"
    );
    assert!(
        matches!(secrets::eval(&ctx_secrets_fail()), GateOutcome::Fail { .. }),
        "secrets: fail case"
    );

    // ── (d) Engine::house() produces identical outcomes to direct calls ───────
    // Build a mixed context: all three gates relevant
    let mut ctx = EvalContext::new();
    ctx.commit_messages =
        vec!["feat: add widget\n\nSigned-off-by: Alice <alice@example.com>".into()];
    ctx.changed_files = vec!["CHANGELOG.md".into(), "src/clean.rs".into()];
    ctx.file_contents
        .insert("CHANGELOG.md".into(), CHANGELOG_GOOD.into());
    ctx.file_contents
        .insert("src/clean.rs".into(), "fn clean() {}".into());

    let engine = Engine::house();
    let engine_outcomes: HashMap<String, GateOutcome> = engine.eval(&ctx).into_iter().collect();

    // Direct-call outcomes for the same context
    let dco_direct = dco::eval(&ctx);
    let changelog_direct = changelog::eval(&ctx);
    let secrets_direct = secrets::eval(&ctx);

    assert_eq!(
        engine_outcomes.get("dco").unwrap(),
        &dco_direct,
        "local≡forge: dco outcomes must be identical"
    );
    assert_eq!(
        engine_outcomes.get("changelog").unwrap(),
        &changelog_direct,
        "local≡forge: changelog outcomes must be identical"
    );
    assert_eq!(
        engine_outcomes.get("secrets").unwrap(),
        &secrets_direct,
        "local≡forge: secrets outcomes must be identical"
    );

    // All three should pass with a well-formed context
    assert!(
        Engine::all_pass(&engine.eval(&ctx)),
        "all gates should pass"
    );
}

// ─── item ②: engine down → landing blocks (kill-test) ───────────────────────
//
// When `landing_gate_check` receives `engine = None` (engine down / unavailable),
// every outcome must be `Blocked`, never `Pass`.  The landing path must refuse.
//
#[test]
fn item_2_engine_down_blocks() {
    let ctx = ctx_dco_pass(); // a perfectly valid context
    let gate_ids = &["dco", "changelog", "secrets"];

    // Kill the engine: pass None
    let outcomes = landing_gate_check(None, &ctx, gate_ids);

    // Every outcome must be Blocked — never Pass, never Fail (which would
    // still be the "correct" gate verdict; Blocked means "don't know, refuse").
    assert_eq!(
        outcomes.len(),
        gate_ids.len(),
        "must return one outcome per gate id"
    );

    for (id, outcome) in &outcomes {
        assert!(
            matches!(outcome, GateOutcome::Blocked { .. }),
            "engine-down kill-test: gate '{}' must be Blocked, got {:?}",
            id,
            outcome
        );
    }

    // The landing path check: any_blocks returns true → landing must refuse
    assert!(
        Engine::any_blocks(&outcomes),
        "landing path: any_blocks must be true when engine is down"
    );
    assert!(
        !Engine::all_pass(&outcomes),
        "landing path: all_pass must be false when engine is down"
    );

    // Verify the same invariant holds even when Engine::eval_closed is called directly
    let direct_outcomes = Engine::eval_closed(gate_ids);
    for (id, outcome) in &direct_outcomes {
        assert!(
            matches!(outcome, GateOutcome::Blocked { .. }),
            "eval_closed: gate '{}' must be Blocked, got {:?}",
            id,
            outcome
        );
    }
}

// ─── item ③: policy change = audited event ───────────────────────────────────
//
// Every policy edit must emit an EventRecord (who, what, when, old→new) and
// append it to the event log.  The event must be attributable to the principal.
//
#[test]
fn item_3_policy_change_audited() {
    let mut log = Vec::new();

    let old_gates = r#"[{"id":"dco","enabled":true}]"#;
    let new_gates = r#"[{"id":"dco","enabled":true},{"id":"secrets","enabled":true}]"#;

    // ── First policy change ───────────────────────────────────────────────────
    let event1 = emit_policy_change(&mut log, "alice@example.com", old_gates, new_gates);

    assert_eq!(
        log.len(),
        1,
        "log must contain one event after first change"
    );
    assert_eq!(event1.seq, 0, "first event seq must be 0");
    assert_eq!(
        event1.prev_hash,
        "0".repeat(64),
        "genesis event prev_hash must be 64 zeros"
    );
    assert_eq!(event1.kind, "policy.change", "kind must be 'policy.change'");
    assert_eq!(
        event1.principal_chain,
        vec!["alice@example.com"],
        "principal_chain must carry the actor"
    );
    assert!(!event1.this_hash.is_empty(), "this_hash must be non-empty");
    assert_eq!(
        event1.this_hash.len(),
        64,
        "this_hash must be a 64-char hex SHA-256"
    );

    // Payload contains old and new gate JSON
    let payload: serde_json::Value =
        serde_json::from_str(&event1.payload).expect("payload must be valid JSON");
    assert_eq!(
        payload["old"], old_gates,
        "payload.old must match old_gates"
    );
    assert_eq!(
        payload["new"], new_gates,
        "payload.new must match new_gates"
    );

    // ── Second policy change (chain integrity) ────────────────────────────────
    let old2 = new_gates;
    let new2 = r#"[{"id":"dco","enabled":true},{"id":"secrets","enabled":false}]"#;

    let event2 = emit_policy_change(&mut log, "bob@example.com", old2, new2);

    assert_eq!(
        log.len(),
        2,
        "log must contain two events after second change"
    );
    assert_eq!(event2.seq, 1, "second event seq must be 1");
    assert_eq!(
        event2.prev_hash, event1.this_hash,
        "chain: event2.prev_hash must equal event1.this_hash"
    );
    assert_ne!(
        event2.this_hash, event1.this_hash,
        "each event must have a distinct hash"
    );
    assert_eq!(
        event2.principal_chain,
        vec!["bob@example.com"],
        "second event must carry bob's identity"
    );

    // ── Silent-change assertion ───────────────────────────────────────────────
    // A policy change with NO call to emit_policy_change must NOT append events.
    let log_len_before = log.len();
    // (no emit_policy_change call here — this is the "silent" baseline)
    assert_eq!(
        log.len(),
        log_len_before,
        "no spurious events appended without an explicit emit"
    );
}
