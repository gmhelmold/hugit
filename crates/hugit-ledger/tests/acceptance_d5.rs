//! D5 acceptance suite — hugit ledger + watch TUI v0 + fleet schema (WP-D5).
//!
//! Items owned (verbatim from decomposition v2.0 D5①–⑥):
//! ① asked→done→proven per campaign
//! ② watch: EventRecord-to-display p95 <2s (measured per event class)
//! ③ deep-links resolve to golden expected targets (not just non-error)
//! ④ planted secret renders REDACTED in ledger/verdict views
//! ⑤ two-zoom toggle: intent view ⇄ raw-commit view mutually consistent over
//!    the same fixture (one store)
//! ⑥ hugit fleet emits documented machine-readable schema reflecting true
//!    ws/agent state vs fixture
//!
//! Each test MEASURES and ASSERTS real behaviour over deterministic fixtures.

use std::time::{Duration, Instant};

use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::{Verdict, VerdictObject};
use hugit_ledger::deeplink::resolve;
use hugit_ledger::fleet::FleetState;
use hugit_ledger::ledger::Ledger;
use hugit_ledger::redact;
use hugit_ledger::watch::{EventClass, WatchDisplay};

// ── fixture helpers ──────────────────────────────────────────────────────────

/// Build a well-formed EventLog (via hugit-refstore) from a list of
/// (kind, principal_chain, payload) tuples, with sequential recorded_at.
fn build_log(events: &[(&str, Vec<&str>, String)]) -> Vec<EventRecord> {
    use hugit_refstore::log::EventLog;
    let mut log = EventLog::new();
    let mut records = Vec::new();
    for (i, (kind, principals, payload)) in events.iter().enumerate() {
        let chain: Vec<String> = principals.iter().map(|s| s.to_string()).collect();
        let r = log.append_for_test(*kind, chain, payload.clone(), 1_000_000 + i as u64);
        records.push(r);
    }
    records
}

/// Build an `intent.landed` payload.
fn intent_payload(
    intent_id: &str,
    campaign: &str,
    charter: &str,
    target: &str,
    deep_link: &str,
) -> String {
    serde_json::json!({
        "intent_id": intent_id,
        "campaign": campaign,
        "charter": charter,
        "ref": "refs/heads/main",
        "target": target,
        "deep_link_target": deep_link,
    })
    .to_string()
}

/// Build a `verdict.recorded` payload.
fn verdict_payload(intent_id: &str, verdict: Verdict) -> String {
    let vo = VerdictObject {
        intent: intent_id.to_string(),
        tree_hash: "abc123".to_string(),
        lens: "security".to_string(),
        model: "claude-sonnet-4-6".to_string(),
        prompt_digest: "deadbeef".to_string(),
        verdict,
        claims_checked: vec!["no-secrets".to_string(), "no-sql-injection".to_string()],
        evidence_refs: vec![],
    };
    serde_json::to_string(&vo).unwrap()
}

// ── ① asked→done→proven per campaign ────────────────────────────────────────

#[test]
fn item_1_asked_done_proven_per_campaign() {
    // Fixture: campaign "wave-D2" with 3 intents, 2 proven.
    let events: &[(&str, Vec<&str>, String)] = &[
        (
            "intent.landed",
            vec!["agent-a"],
            intent_payload(
                "id-001",
                "wave-D2",
                "Add ledger module",
                "sha:aaa111",
                "sha:aaa111",
            ),
        ),
        (
            "intent.landed",
            vec!["agent-b"],
            intent_payload(
                "id-002",
                "wave-D2",
                "Add watch module",
                "sha:bbb222",
                "sha:bbb222",
            ),
        ),
        (
            "intent.landed",
            vec!["agent-c"],
            intent_payload(
                "id-003",
                "wave-D2",
                "Add fleet module",
                "sha:ccc333",
                "sha:ccc333",
            ),
        ),
        (
            "verdict.recorded",
            vec!["reviewer"],
            verdict_payload("id-001", Verdict::Approve),
        ),
        (
            "verdict.recorded",
            vec!["reviewer"],
            verdict_payload("id-002", Verdict::Approve),
        ),
        // id-003 NOT proven yet.
    ];

    let records = build_log(events);
    let ledger = Ledger::from_records(&records);

    // ① asked: all three are in the ledger
    assert_eq!(ledger.asked("wave-D2"), 3, "asked count must be 3");

    // ① done: all three are done (landed = done)
    assert_eq!(ledger.done("wave-D2"), 3, "done count must be 3");

    // ① proven: two have verdicts
    assert_eq!(ledger.proven("wave-D2"), 2, "proven count must be 2");

    // Verify the proven flags individually.
    let entries: Vec<_> = ledger.by_campaign("wave-D2").collect();
    assert_eq!(entries.len(), 3);
    assert!(entries[0].proven, "id-001 must be proven");
    assert!(entries[1].proven, "id-002 must be proven");
    assert!(!entries[2].proven, "id-003 must NOT be proven");

    // Verdicts are present for the proven entries.
    assert!(
        entries[0].verdict.is_some(),
        "id-001 must have verdict view"
    );
    assert!(
        entries[1].verdict.is_some(),
        "id-002 must have verdict view"
    );
    assert!(
        entries[2].verdict.is_none(),
        "id-003 must not have verdict view"
    );
}

// ── ② watch latency p95 <2s (measured, not instant) ─────────────────────────

#[test]
fn item_2_watch_latency_p95_under_2s() {
    // Build a fixture with 40 events per class (160 total).
    // Each render should complete well under 2 seconds; we measure the real
    // wall-clock latency and assert p95 < 2000 ms per class.
    let mut events: Vec<(&str, Vec<&str>, String)> = Vec::new();

    // 40 landing events.
    for i in 0..40usize {
        events.push((
            "intent.landed",
            vec!["agent-a"],
            intent_payload(
                &format!("id-{i}"),
                "wave-D2",
                "charter",
                "sha:000",
                "sha:000",
            ),
        ));
    }
    // 40 verdict events.
    for i in 0..40usize {
        events.push((
            "verdict.recorded",
            vec!["reviewer"],
            verdict_payload(&format!("id-{i}"), Verdict::Approve),
        ));
    }
    // 40 policy-change events.
    for i in 0..40usize {
        events.push((
            "policy.changed",
            vec!["system"],
            format!(r#"{{"rule":"rule-{i}","action":"enabled"}}"#),
        ));
    }
    // 40 ws-state events.
    for i in 0..40usize {
        events.push((
            "ws.state.active",
            vec!["system"],
            format!(r#"{{"workspace_id":"ws-{i}"}}"#),
        ));
    }

    let records = build_log(&events);

    // Measure: process all records through WatchDisplay, capturing real timing.
    let overall_start = Instant::now();
    let mut display = WatchDisplay::new();
    display.process_batch(&records);
    let overall_elapsed = overall_start.elapsed();

    // Sanity: total wall time for 160 renders must be < 10s (very loose bound).
    assert!(
        overall_elapsed < Duration::from_secs(10),
        "total render time for 160 events exceeded 10s: {:?}",
        overall_elapsed
    );

    // Assert p95 < 2000 ms per class.
    let classes = [
        EventClass::Landing,
        EventClass::Verdict,
        EventClass::PolicyChange,
        EventClass::WsState,
    ];
    for class in &classes {
        let p95 = display.p95_ms(*class);
        assert!(
            p95 < 2000,
            "p95 render latency for {:?} is {}ms — must be < 2000ms",
            class,
            p95
        );
    }

    // Print the latency table as evidence (visible with -- --nocapture).
    println!("--- D5② latency table ---");
    for class in &classes {
        let m = display.latency_for(*class);
        let count = m.map(|m| m.count()).unwrap_or(0);
        let p95 = display.p95_ms(*class);
        println!("  {:?}: n={} p95={}ms", class, count, p95);
    }
}

// ── ③ deep-links resolve to golden expected targets ──────────────────────────

#[test]
fn item_3_deep_links_resolve_to_golden_targets() {
    // Fixture: three intents with distinct golden targets.
    let golden = [
        ("id-alpha", "sha:golden-aaa111aaa111"),
        ("id-beta", "sha:golden-bbb222bbb222"),
        ("id-gamma", "sha:golden-ccc333ccc333"),
    ];
    let events: Vec<(&str, Vec<&str>, String)> = golden
        .iter()
        .map(|(id, target)| {
            (
                "intent.landed",
                vec!["agent-x"],
                intent_payload(id, "wave-D2", "charter", target, target),
            )
        })
        .collect();

    let records = build_log(&events);

    // Each deep-link must resolve to the EXACT golden target (not merely non-error).
    for (id, expected_target) in &golden {
        match resolve(id, &records) {
            hugit_ledger::deeplink::ResolveResult::Found { target, .. } => {
                assert_eq!(
                    &target, expected_target,
                    "deep-link for {id} resolved to wrong target: got {target}, expected {expected_target}"
                );
            }
            hugit_ledger::deeplink::ResolveResult::NotFound { .. } => {
                panic!("deep-link for {id} was not found in the event log");
            }
        }
    }

    // A missing id must return NotFound.
    let missing = resolve("id-does-not-exist", &records);
    assert!(
        matches!(
            missing,
            hugit_ledger::deeplink::ResolveResult::NotFound { .. }
        ),
        "expected NotFound for missing id"
    );
}

// ── ④ planted secret renders REDACTED ────────────────────────────────────────

#[test]
fn item_4_planted_secret_renders_redacted() {
    // Plant a secret in the charter field of an intent.
    let secret_charter = "SECRET:my-api-key-12345";
    let events: &[(&str, Vec<&str>, String)] = &[
        (
            "intent.landed",
            vec!["agent-a"],
            intent_payload("id-secret", "wave-D2", secret_charter, "sha:000", "sha:000"),
        ),
        // Verdict with secret in claims.
        ("verdict.recorded", vec!["reviewer"], {
            let vo = VerdictObject {
                intent: "id-secret".to_string(),
                tree_hash: "abc123".to_string(),
                lens: "security".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                prompt_digest: "deadbeef".to_string(),
                verdict: Verdict::Approve,
                claims_checked: vec![
                    "normal-claim".to_string(),
                    "SECRET:another-secret-claim".to_string(),
                ],
                evidence_refs: vec![],
            };
            serde_json::to_string(&vo).unwrap()
        }),
    ];

    let records = build_log(events);
    let ledger = Ledger::from_records(&records);

    let entries: Vec<_> = ledger.by_campaign("wave-D2").collect();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];

    // ④: charter must be REDACTED — the raw secret must NOT appear in the output bytes.
    assert_eq!(
        entry.charter,
        hugit_ledger::REDACTED,
        "charter must be [REDACTED]; got {:?}",
        entry.charter
    );
    assert!(
        !entry.charter.contains("SECRET:"),
        "SECRET: must not appear in ledger charter output bytes"
    );

    // ④: verdict view claims must be redacted where secret.
    let verdict = entry.verdict.as_ref().expect("verdict must be present");
    assert!(
        !verdict.claims_checked.iter().any(|c| c.contains("SECRET:")),
        "SECRET: must not appear in any claim in the verdict view"
    );
    // The secret claim must be [REDACTED].
    assert!(
        verdict
            .claims_checked
            .iter()
            .any(|c| c == hugit_ledger::REDACTED),
        "At least one claim must be [REDACTED]"
    );
    // Non-secret claims must pass through.
    assert!(
        verdict.claims_checked.iter().any(|c| c == "normal-claim"),
        "normal-claim must pass through unredacted"
    );

    // ④: direct redact::apply check — verify the raw secret is not visible.
    let raw = secret_charter;
    let rendered = redact::apply(raw);
    assert_eq!(rendered, hugit_ledger::REDACTED);
    assert!(
        !rendered.contains("SECRET:"),
        "rendered output must not contain SECRET:"
    );
}

/// ④-extended: secret planted in intent_id, campaign, AND deep_link_target —
/// ALL must render [REDACTED], not raw.
///
/// This is the gamed-by-omission defect: the original oracle only planted in
/// `charter` and verdict `claims_checked`. These three fields were surfaced raw.
#[test]
fn item_4b_secret_in_intent_id_campaign_deep_link_redacted() {
    let secret_intent_id = "SECRET:intent-token-abc";
    let secret_campaign = "SECRET:campaign-key-xyz";
    let secret_deep_link = "SECRET:deep-link-target-secret";

    // Build the payload manually so we can control all three fields.
    let payload = serde_json::json!({
        "intent_id": secret_intent_id,
        "campaign": secret_campaign,
        "charter": "normal-charter",
        "ref": "refs/heads/main",
        "target": "sha:abc",
        "deep_link_target": secret_deep_link,
    })
    .to_string();

    let events: &[(&str, Vec<&str>, String)] = &[("intent.landed", vec!["agent-a"], payload)];

    let records = build_log(events);
    let ledger = Ledger::from_records(&records);

    // The entry will be found under the (redacted) campaign name — but the
    // ledger stores whatever came from the payload, so we get all entries.
    let entries = ledger.entries();
    assert_eq!(entries.len(), 1, "one entry must be present");
    let entry = &entries[0];

    // intent_id must be REDACTED.
    assert_eq!(
        entry.intent_id,
        hugit_ledger::REDACTED,
        "intent_id must be [REDACTED] when it contains a secret; got {:?}",
        entry.intent_id
    );
    assert!(
        !entry.intent_id.contains("SECRET:"),
        "SECRET: must not appear in surfaced intent_id"
    );

    // campaign must be REDACTED.
    assert_eq!(
        entry.campaign,
        hugit_ledger::REDACTED,
        "campaign must be [REDACTED] when it contains a secret; got {:?}",
        entry.campaign
    );
    assert!(
        !entry.campaign.contains("SECRET:"),
        "SECRET: must not appear in surfaced campaign"
    );

    // deep_link_target must be REDACTED.
    assert_eq!(
        entry.deep_link_target,
        hugit_ledger::REDACTED,
        "deep_link_target must be [REDACTED] when it contains a secret; got {:?}",
        entry.deep_link_target
    );
    assert!(
        !entry.deep_link_target.contains("SECRET:"),
        "SECRET: must not appear in surfaced deep_link_target"
    );

    // charter (non-secret) must pass through unchanged.
    assert_eq!(
        entry.charter, "normal-charter",
        "non-secret charter must pass through unredacted"
    );
}

/// ④-fleet: secret planted in workspace_id and agent_id — must render [REDACTED].
///
/// fleet/mod.rs applied zero redaction on surfaced workspace_id / agent_id.
#[test]
fn item_4c_fleet_redacts_workspace_and_agent_ids() {
    use hugit_ledger::fleet::FleetState;

    let secret_ws = "SECRET:ws-key-abc123";
    let secret_agent = "SECRET:agent-token-xyz";

    let events: &[(&str, Vec<&str>, String)] = &[
        (
            "ws.state.active",
            vec!["system"],
            serde_json::json!({"workspace_id": secret_ws}).to_string(),
        ),
        (
            "agent.assigned",
            vec!["dispatch"],
            serde_json::json!({"agent_id": secret_agent, "workspace_id": secret_ws}).to_string(),
        ),
    ];

    let records = build_log(events);
    let fleet = FleetState::from_records(&records);

    // workspace_id must not be raw secret.
    for ws in &fleet.workspaces {
        assert!(
            !ws.workspace_id.contains("SECRET:"),
            "SECRET: must not appear in surfaced workspace_id; got {:?}",
            ws.workspace_id
        );
        assert_eq!(
            ws.workspace_id,
            hugit_ledger::REDACTED,
            "workspace_id containing a secret must be [REDACTED]"
        );
    }

    // agent_id and workspace_id in agent entries must not be raw secret.
    for agent in &fleet.agents {
        assert!(
            !agent.agent_id.contains("SECRET:"),
            "SECRET: must not appear in surfaced agent_id; got {:?}",
            agent.agent_id
        );
        assert_eq!(
            agent.agent_id,
            hugit_ledger::REDACTED,
            "agent_id containing a secret must be [REDACTED]"
        );
        assert!(
            !agent.workspace_id.contains("SECRET:"),
            "SECRET: must not appear in surfaced workspace_id in agent entry; got {:?}",
            agent.workspace_id
        );
        assert_eq!(
            agent.workspace_id,
            hugit_ledger::REDACTED,
            "workspace_id containing a secret in agent entry must be [REDACTED]"
        );
    }
}

/// ④-deeplink: secret planted in deep_link target — must render [REDACTED] in
/// ResolveResult::Found::target.
///
/// deeplink.rs applied zero redaction on the returned target.
#[test]
fn item_4d_deeplink_redacts_secret_target() {
    use hugit_ledger::deeplink::{ResolveResult, resolve};

    let secret_target = "SECRET:deep-link-content-addr";

    let payload = serde_json::json!({
        "intent_id": "id-dl-secret",
        "campaign": "wave-test",
        "charter": "some charter",
        "ref": "refs/heads/main",
        "target": secret_target,
        "deep_link_target": secret_target,
    })
    .to_string();

    let events: &[(&str, Vec<&str>, String)] = &[("intent.landed", vec!["agent-a"], payload)];

    let records = build_log(events);

    match resolve("id-dl-secret", &records) {
        ResolveResult::Found { target, .. } => {
            assert!(
                !target.contains("SECRET:"),
                "SECRET: must not appear in resolved deep-link target; got {:?}",
                target
            );
            assert_eq!(
                target,
                hugit_ledger::REDACTED,
                "resolved target containing a secret must be [REDACTED]"
            );
        }
        ResolveResult::NotFound { .. } => panic!("deep-link must resolve"),
    }
}

/// Defect 3: malformed payload must increment a `malformed` counter rather
/// than silently coalescing to "unknown"/dropping the record.
#[test]
fn item_4e_malformed_fleet_payload_counted_not_swallowed() {
    use hugit_ledger::fleet::FleetState;

    // Valid event first, then a malformed ws.state.active (payload is not JSON).
    let valid_ws_payload = serde_json::json!({"workspace_id": "ws-ok"}).to_string();
    let malformed_payload = "not-valid-json{{{{".to_string();

    let events: &[(&str, Vec<&str>, String)] = &[
        ("ws.state.active", vec!["system"], valid_ws_payload),
        // malformed: payload is not JSON — should NOT produce a "unknown" workspace.
        ("ws.state.active", vec!["system"], malformed_payload),
    ];

    let records = build_log(events);
    let fleet = FleetState::from_records(&records);

    // The malformed record must be COUNTED, not silently normalized to "unknown".
    assert_eq!(
        fleet.malformed, 1,
        "one malformed record must be counted; got {}",
        fleet.malformed
    );

    // The valid workspace must still be present.
    assert_eq!(
        fleet.workspaces.len(),
        1,
        "only the well-formed workspace must appear"
    );
    assert_eq!(
        fleet.workspaces[0].workspace_id, "ws-ok",
        "valid workspace id must be surfaced"
    );

    // No "unknown" workspace must be fabricated from the malformed payload.
    assert!(
        !fleet.workspaces.iter().any(|w| w.workspace_id == "unknown"),
        "malformed payload must not create an 'unknown' workspace"
    );
}

/// LOW defect: unknown event kinds must be classified as Other, not Landing.
#[test]
fn item_4f_unknown_event_class_is_other_not_landing() {
    use hugit_ledger::watch::{EventClass, WatchDisplay};

    let events: &[(&str, Vec<&str>, String)] = &[
        // Known kind.
        (
            "intent.landed",
            vec!["agent"],
            r#"{"intent_id":"i1"}"#.to_string(),
        ),
        // Unknown kind — must NOT be classified as Landing.
        (
            "some.unknown.event.kind",
            vec!["system"],
            r#"{"data":"x"}"#.to_string(),
        ),
    ];

    let records = build_log(events);
    let mut display = WatchDisplay::new();
    let lines = display.process_batch(&records);

    // The known landing event is Landing.
    assert_eq!(
        lines[0].class,
        EventClass::Landing,
        "intent.landed must be Landing"
    );

    // The unknown event must be Other, not Landing.
    assert_ne!(
        lines[1].class,
        EventClass::Landing,
        "unknown event kind must NOT be classified as Landing; got {:?}",
        lines[1].class
    );
    assert_eq!(
        lines[1].class,
        EventClass::Other,
        "unknown event kind must be classified as Other"
    );
}

// ── ⑤ two-zoom toggle mutually consistent (one store) ────────────────────────

#[test]
fn item_5_two_zoom_toggle_mutually_consistent() {
    // Fixture: 10 intent.landed + 5 raw pushes on the same event log.
    use hugit_refstore::intent::model::intents_from_log;
    use hugit_refstore::intent::projection::project_machine;
    use hugit_refstore::log::EventLog;

    let mut log = EventLog::new();

    // 10 intent.landed events.
    for i in 0..10usize {
        let payload = serde_json::json!({
            "intent_id": format!("intent-{i:03}"),
            "ref": "refs/heads/main",
            "target": format!("sha:target{i:03}"),
            "charter": format!("charter for intent {i}"),
        })
        .to_string();
        log.append_for_test(
            "intent.landed",
            vec!["agent".to_string()],
            payload,
            1_000_000 + i as u64,
        );
    }

    // 5 raw pushes (external changes — NOT intents).
    for i in 0..5usize {
        let payload = serde_json::json!({
            "ref": "refs/heads/feature",
            "target": format!("sha:raw{i:03}"),
        })
        .to_string();
        log.append_for_test(
            "ref.update",
            vec!["human".to_string()],
            payload,
            2_000_000 + i as u64,
        );
    }

    // Project both altitudes from the SAME store.
    let intent_altitude = intents_from_log(&log).expect("intent altitude must succeed");
    let machine_altitude = project_machine(&log).expect("machine altitude must succeed");

    // ⑤: mutual consistency — both altitudes derive from the same log.
    assert!(
        machine_altitude.is_consistent_with(&intent_altitude),
        "two-zoom toggle: intent altitude and machine altitude must be mutually consistent"
    );

    // ⑤: intent altitude has exactly 10 intents (raw pushes are not intents).
    assert_eq!(
        intent_altitude.len(),
        10,
        "intent altitude must have 10 entries"
    );

    // ⑤: machine altitude has 15 rows total (10 intents + 5 external-change).
    assert_eq!(
        machine_altitude.len(),
        15,
        "machine altitude must have 15 rows"
    );

    // ⑤: every intent in the intent altitude matches its peer in the machine altitude.
    for intent in intent_altitude.intents() {
        // The intent must appear in the machine altitude as a generated commit.
        let commit = machine_altitude
            .commits()
            .find(|c| c.intent_id == intent.intent_id)
            .unwrap_or_else(|| panic!("intent {} not found in machine altitude", intent.intent_id));
        assert_eq!(commit.seq, intent.seq, "seq must match");
        assert_eq!(commit.ref_name, intent.ref_name, "ref_name must match");
        assert_eq!(commit.target, intent.target, "target must match");

        // The generated message must embed the intent_id.
        use hugit_refstore::intent::projection::GitCommit;
        let recovered = GitCommit::intent_id_from_message(&commit.message);
        assert_eq!(
            recovered,
            Some(intent.intent_id.as_str()),
            "generated commit message must embed intent_id"
        );
    }

    // ⑤: raw pushes project as external-change, never as fabricated intents.
    for row in machine_altitude.external_changes() {
        assert!(
            !row.is_intent(),
            "raw push must not become an intent in the machine altitude"
        );
    }
}

// ── ⑥ fleet emits valid schema vs fixture ────────────────────────────────────

#[test]
fn item_6_fleet_emits_valid_schema_vs_fixture() {
    // Fixture: 3 workspaces (2 active, 1 closed), 4 agents (2 assigned, 1 completed, 1 failed).
    let events: &[(&str, Vec<&str>, String)] = &[
        // Workspace state events.
        (
            "ws.state.active",
            vec!["system"],
            r#"{"workspace_id":"ws-001"}"#.to_string(),
        ),
        (
            "ws.state.active",
            vec!["system"],
            r#"{"workspace_id":"ws-002"}"#.to_string(),
        ),
        (
            "ws.state.active",
            vec!["system"],
            r#"{"workspace_id":"ws-003"}"#.to_string(),
        ),
        (
            "ws.state.closed",
            vec!["system"],
            r#"{"workspace_id":"ws-003"}"#.to_string(),
        ),
        // Agent lifecycle events.
        (
            "agent.assigned",
            vec!["dispatch"],
            r#"{"agent_id":"agent-A","workspace_id":"ws-001"}"#.to_string(),
        ),
        (
            "agent.assigned",
            vec!["dispatch"],
            r#"{"agent_id":"agent-B","workspace_id":"ws-001"}"#.to_string(),
        ),
        (
            "agent.assigned",
            vec!["dispatch"],
            r#"{"agent_id":"agent-C","workspace_id":"ws-002"}"#.to_string(),
        ),
        (
            "agent.assigned",
            vec!["dispatch"],
            r#"{"agent_id":"agent-D","workspace_id":"ws-002"}"#.to_string(),
        ),
        (
            "agent.completed",
            vec!["agent-C"],
            r#"{"agent_id":"agent-C"}"#.to_string(),
        ),
        (
            "agent.failed",
            vec!["agent-D"],
            r#"{"agent_id":"agent-D"}"#.to_string(),
        ),
    ];

    let records = build_log(events);
    let fleet = FleetState::from_records(&records);

    // ⑥: schema is valid.
    fleet
        .validate()
        .expect("FleetState must pass schema validation");

    // ⑥: schema_version is documented.
    assert_eq!(
        fleet.schema_version,
        hugit_ledger::fleet::FLEET_SCHEMA_VERSION,
        "schema_version must match the documented constant"
    );

    // ⑥: workspaces reflect TRUE state vs fixture.
    let ws_map: std::collections::HashMap<_, _> = fleet
        .workspaces
        .iter()
        .map(|w| (w.workspace_id.as_str(), &w.state))
        .collect();

    assert!(ws_map.contains_key("ws-001"), "ws-001 must be present");
    assert!(ws_map.contains_key("ws-002"), "ws-002 must be present");
    assert!(ws_map.contains_key("ws-003"), "ws-003 must be present");

    assert!(
        matches!(
            ws_map["ws-001"],
            hugit_ledger::fleet::WorkspaceState::Active
        ),
        "ws-001 must be Active"
    );
    assert!(
        matches!(
            ws_map["ws-002"],
            hugit_ledger::fleet::WorkspaceState::Active
        ),
        "ws-002 must be Active"
    );
    assert!(
        matches!(
            ws_map["ws-003"],
            hugit_ledger::fleet::WorkspaceState::Closed
        ),
        "ws-003 must be Closed (transitioned after active)"
    );

    // ⑥: agents reflect TRUE state vs fixture.
    let agent_map: std::collections::HashMap<_, _> = fleet
        .agents
        .iter()
        .map(|a| (a.agent_id.as_str(), &a.state))
        .collect();

    assert_eq!(agent_map.len(), 4, "4 agents must be present");
    assert!(
        matches!(
            agent_map["agent-A"],
            hugit_ledger::fleet::AgentState::Assigned
        ),
        "agent-A must be Assigned"
    );
    assert!(
        matches!(
            agent_map["agent-B"],
            hugit_ledger::fleet::AgentState::Assigned
        ),
        "agent-B must be Assigned"
    );
    assert!(
        matches!(
            agent_map["agent-C"],
            hugit_ledger::fleet::AgentState::Completed
        ),
        "agent-C must be Completed"
    );
    assert!(
        matches!(
            agent_map["agent-D"],
            hugit_ledger::fleet::AgentState::Failed
        ),
        "agent-D must be Failed"
    );

    // ⑥: event_count and last_seq are accurate.
    assert_eq!(
        fleet.event_count,
        records.len() as u64,
        "event_count must equal records.len()"
    );
    assert_eq!(
        fleet.last_seq,
        records.last().map(|r| r.seq).unwrap_or(0),
        "last_seq must be the seq of the last record"
    );

    // ⑥: emit + round-trip: JSON output is schema-valid and round-trips cleanly.
    let json = fleet.to_json();
    let reparsed: FleetState =
        serde_json::from_str(&json).expect("fleet JSON must round-trip through serde");
    assert_eq!(
        reparsed, fleet,
        "fleet state must survive a JSON round-trip"
    );

    // Print the emitted schema as evidence.
    println!("--- D5⑥ fleet schema emission ---");
    println!("{}", json);
}
