//! Acceptance — WP-F3: the three-altitude rollups (ADR-0001 §2.3) proven on a
//! multi-PR campaign.
//!
//! The fixture is built from REAL projection paths, never hand-faked sums
//! (the house pattern from `hugit-web`'s parity tests):
//!
//! 1. A world [`EventLog`] is built through the real append path
//!    ([`EventLog::append`] over [`canonical_json`] payloads): one
//!    `intent.landed` per landed intent, `verdict.recorded` for the proven
//!    subset.
//! 2. The landed-intent truth is PROJECTED out of that log
//!    ([`intents_from_log`] + [`Ledger::from_records`]) — landed ids, landing
//!    times and verdict panels all come from the projections, not constants.
//! 3. Frozen [`ContextEnvelope`]s (WP-F1) supply the per-unit metrics; the
//!    rollups under test compute everything else.
//!
//! Proven here, per the DoD:
//! - the decomposition adds up: `work + orchestration + verification + ci =
//!   total` (tokens and USD), with waste shown in its own line and EXCLUDED
//!   from total (ADR JSONC);
//! - `wall_span_ms ≤ agent_sum_ms` under parallelism (span vs sum);
//! - a subagent-authored envelope at `altitude:"pr"` is REJECTED by the
//!   rollup path (D14 — the type already forbids it; the projection refuses
//!   to construct it);
//! - null-ref tolerance: envelopes with all-`null` refs (sub-full capture)
//!   and a landed intent with NO envelope at all both roll up.

use hugit_contracts::context_envelope::{
    Altitude, Authorship, CiCost, ContextEnvelope, FileRead, IntentMetrics, PrAuthorKind, Snapshot,
    Spawn, TokenCounts, Trajectory,
};
use hugit_contracts::verdict_object::{Verdict, VerdictObject};
use hugit_ledger::Ledger;
use hugit_ledger::rollup::{PrPhase, PrQueueInput, RollupError, campaign_rollup, pr_record};
use hugit_refstore::intent::intents_from_log;
use hugit_refstore::{EventLog, canonical_json};

/// World epoch (unix ms). Every timestamp in the fixture is `T0 + offset` so
/// envelope lifespans and log `recorded_at`s live on one clock.
const T0: u64 = 1_760_000_000_000;

const CAMPAIGN: &str = "auth-hardening";

/// The landing-queue bundles: which landed intents belong to which PR (queue
/// state — the event payload deliberately does not carry PR membership).
const PR128_INTENTS: [&str; 3] = ["i-a1", "i-a2", "i-a3"];
const PR129_INTENTS: [&str; 2] = ["i-b1", "i-b2"];

// ─────────────────────────────────────────────────────────────────────────────
// World construction — the real append path
// ─────────────────────────────────────────────────────────────────────────────

fn principal_chain() -> Vec<String> {
    vec!["user:gustavo".to_string(), "orchestrator:opus".to_string()]
}

fn fake_oid_40(n: usize) -> String {
    format!("{n:040x}")
}

fn fake_tree_64(n: usize) -> String {
    format!("{n:064x}")
}

/// Build the world event log: 5 `intent.landed` (3 of PR-128, 2 of PR-129)
/// plus 3 `verdict.recorded` (i-a1, i-a3, i-b1), all through the real append
/// path over canonical-JSON payloads.
fn build_world_log() -> EventLog {
    let mut log = EventLog::new();
    let landed: [(&str, u64); 5] = [
        ("i-a1", T0 + 180_000),
        ("i-a2", T0 + 181_000),
        ("i-a3", T0 + 182_000),
        ("i-b1", T0 + 240_000),
        ("i-b2", T0 + 241_000),
    ];
    for (idx, (id, at)) in landed.iter().enumerate() {
        let payload_raw = serde_json::json!({
            "intent_id": id,
            "ref": "refs/heads/main",
            "target": fake_oid_40(idx),
            "charter": format!("endurecer a borda de autenticação — {id}"),
            "campaign": CAMPAIGN,
            "deep_link_target": id,
        })
        .to_string();
        let payload = canonical_json(&payload_raw).expect("valid JSON");
        log.append("intent.landed", principal_chain(), payload, *at);
    }
    for (idx, (id, at)) in [
        ("i-a1", T0 + 183_000),
        ("i-a3", T0 + 184_000),
        ("i-b1", T0 + 242_000),
    ]
    .iter()
    .enumerate()
    {
        let vo = VerdictObject {
            intent: id.to_string(),
            tree_hash: fake_tree_64(idx),
            lens: "adversarial-review".to_string(),
            model: "opus-4.8".to_string(),
            prompt_digest: format!("{:064x}", 0xC0FFEE + idx as u64),
            verdict: Verdict::Approve,
            claims_checked: vec!["charter cumprida".to_string()],
            evidence_refs: vec![format!("cas://verdict/{id}")],
        };
        let payload_raw = serde_json::to_string(&vo).expect("VerdictObject serializes");
        let payload = canonical_json(&payload_raw).expect("valid JSON");
        log.append("verdict.recorded", principal_chain(), payload, *at);
    }
    log
}

/// The landed intent ids of one PR bundle, PROJECTED from the world log
/// (log order ∩ bundle membership) — the queue's truth, not a constant list.
fn landed_ids_of(log: &EventLog, bundle: &[&str]) -> Vec<String> {
    intents_from_log(log)
        .expect("world log projects")
        .intents()
        .iter()
        .filter(|i| bundle.contains(&i.intent_id.as_str()))
        .map(|i| i.intent_id.clone())
        .collect()
}

/// The landing time of one PR bundle: the latest `recorded_at` among its
/// landed intents, off the Ledger projection.
fn landed_at_of(ledger: &Ledger, bundle: &[&str]) -> u64 {
    ledger
        .by_campaign(CAMPAIGN)
        .filter(|e| bundle.contains(&e.intent_id.as_str()))
        .map(|e| e.recorded_at)
        .max()
        .expect("bundle landed")
}

/// The verdict panels recorded for one PR bundle, parsed from the world log's
/// `verdict.recorded` records (the D5 seam the Ledger itself reads).
fn verdicts_of(log: &EventLog, bundle: &[&str]) -> Vec<VerdictObject> {
    log.records()
        .iter()
        .filter(|r| r.kind == "verdict.recorded")
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .filter(|vo| bundle.contains(&vo.intent.as_str()))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Envelope fixtures — the frozen WP-F1 shape
// ─────────────────────────────────────────────────────────────────────────────

/// All-`None` refs (sub-full capture) — the null-tolerance posture every
/// fixture envelope ships with; the rollup must never need them.
fn null_trajectory() -> Trajectory {
    Trajectory {
        raw_transcript_ref: None,
        task_transcript_ref: None,
        summary: None,
        journal_ref: None,
        redaction_policy: "default-v1".to_string(),
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        files_read: vec![FileRead {
            path: "auth/token.rs".to_string(),
            hash: format!("sha256:{:064x}", 0x9C2Fu64),
        }],
        prompt_ref: None,
        env_manifest: "rustc 1.96.0".to_string(),
    }
}

fn metrics(
    tokens_total: u64,
    active_ms: u64,
    tool_calls: u64,
    turns: u64,
    cost: f64,
) -> IntentMetrics {
    IntentMetrics {
        tokens: TokenCounts {
            input: tokens_total / 2,
            output: tokens_total - tokens_total / 2,
            cache_read: 0,
            cache_write: 0,
            total: tokens_total,
        },
        wall_ms: active_ms + 2_000,
        active_ms,
        tool_calls,
        tool_breakdown: vec![],
        model_turns: turns,
        cost_usd: cost,
    }
}

#[allow(clippy::too_many_arguments)]
fn envelope(
    altitude: Altitude,
    id: &str,
    model: &str,
    agent_type: &str,
    run_id: &str,
    parent: Option<&str>,
    born: u64,
    died: u64,
    m: IntentMetrics,
) -> ContextEnvelope {
    ContextEnvelope {
        schema_version: "1.0.0".to_string(),
        altitude,
        intent_id: id.to_string(),
        commit: fake_oid_40(7),
        tree_hash: fake_tree_64(7),
        authorship: Authorship {
            model: model.to_string(),
            model_digest: if model.is_empty() {
                String::new()
            } else {
                format!("{:064x}", 0xABCDu64)
            },
            agent_type: agent_type.to_string(),
            spawn: Spawn {
                run_id: run_id.to_string(),
                parent_run_id: parent.map(str::to_string),
                born_at: born,
                died_at: died,
            },
            operator: "gustavo@humangr.com".to_string(),
        },
        charter: format!("endurecer a borda de autenticação — {id}"),
        campaign: Some(CAMPAIGN.to_string()),
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        trajectory: null_trajectory(),
        snapshot: snapshot(),
        metrics: m,
        verdicts_ref: None,
    }
}

/// A subagent-authored intent envelope (spawned by the PR's orchestrator).
#[allow(clippy::too_many_arguments)]
fn intent_env(
    id: &str,
    model: &str,
    run_id: &str,
    parent: &str,
    born: u64,
    died: u64,
    active_ms: u64,
    tokens: u64,
    tool_calls: u64,
    cost: f64,
) -> ContextEnvelope {
    envelope(
        Altitude::Intent,
        id,
        model,
        "implementer",
        run_id,
        Some(parent),
        born,
        died,
        metrics(tokens, active_ms, tool_calls, 8, cost),
    )
}

// ── PR-128: orchestrator-authored; 3 landed intents in PARALLEL, one retry,
//    one discarded intent. ──────────────────────────────────────────────────

fn pr128_envelope() -> ContextEnvelope {
    envelope(
        Altitude::Pr,
        "PR-128",
        "opus-4.8",
        "main",
        "orq-014",
        None,
        T0,
        T0 + 182_000,
        metrics(20_000, 30_000, 40, 12, 0.50),
    )
}

fn pr128_intent_envelopes() -> Vec<ContextEnvelope> {
    vec![
        // Three landed intents overlapping in time → agent_sum > wall span.
        intent_env(
            "i-a1",
            "opus-4.8",
            "run-a1",
            "orq-014",
            T0 + 10_000,
            T0 + 80_000,
            60_000,
            50_000,
            30,
            1.00,
        ),
        // i-a2: a retried first attempt (waste) + the landed final attempt.
        intent_env(
            "i-a2",
            "opus-4.8",
            "run-a2x",
            "orq-014",
            T0 + 10_000,
            T0 + 40_000,
            25_000,
            20_000,
            12,
            0.40,
        ),
        intent_env(
            "i-a2",
            "opus-4.8",
            "run-a2",
            "orq-014",
            T0 + 50_000,
            T0 + 120_000,
            60_000,
            45_000,
            25,
            0.90,
        ),
        intent_env(
            "i-a3",
            "sonnet-4.6",
            "run-a3",
            "orq-014",
            T0 + 12_000,
            T0 + 90_000,
            60_000,
            40_000,
            20,
            0.80,
        ),
        // i-a9: authored but never landed — discarded spend (waste, shown).
        intent_env(
            "i-a9",
            "opus-4.8",
            "run-a9",
            "orq-014",
            T0 + 15_000,
            T0 + 30_000,
            12_000,
            9_000,
            6,
            0.18,
        ),
    ]
}

fn pr128_ci() -> CiCost {
    // From the checks seam: 5 memoized hits + 5 executions (measured counts).
    CiCost {
        cache_hit: 5,
        exec: 5,
        cost_usd: 0.10,
        saved_usd: 0.40,
    }
}

// ── PR-129: HUMAN-authored (no model session); one intent envelope with
//    all-null refs, one landed intent with NO envelope at all. ──────────────

fn pr129_envelope() -> ContextEnvelope {
    envelope(
        Altitude::Pr,
        "PR-129",
        "", // a human session records no model → PrAuthorKind::Human
        "main",
        "sess-h1",
        None,
        T0 + 200_000,
        T0 + 241_000,
        metrics(0, 0, 0, 0, 0.0),
    )
}

fn pr129_intent_envelopes() -> Vec<ContextEnvelope> {
    // i-b2 landed but has NO captured envelope (sub-full capture): tolerated.
    vec![intent_env(
        "i-b1",
        "opus-4.8",
        "run-b1",
        "sess-h1",
        T0 + 205_000,
        T0 + 230_000,
        20_000,
        30_000,
        14,
        0.60,
    )]
}

fn pr129_ci() -> CiCost {
    CiCost {
        cache_hit: 3,
        exec: 1,
        cost_usd: 0.05,
        saved_usd: 0.15,
    }
}

// ── PR-130 (in flight) and PR-131 (blocked): nothing landed yet — their
//    intent spend is honestly waste-so-far. ──────────────────────────────────

fn inflight_pr(
    pr_id: &str,
    run: &str,
    born: u64,
    intent: ContextEnvelope,
) -> (ContextEnvelope, Vec<ContextEnvelope>) {
    let pr = envelope(
        Altitude::Pr,
        pr_id,
        "opus-4.8",
        "main",
        run,
        None,
        born,
        born + 30_000,
        metrics(5_000, 8_000, 10, 4, 0.10),
    );
    (pr, vec![intent])
}

fn zero_ci() -> CiCost {
    CiCost {
        cache_hit: 0,
        exec: 0,
        cost_usd: 0.0,
        saved_usd: 0.0,
    }
}

fn campaign_envelope() -> ContextEnvelope {
    envelope(
        Altitude::Campaign,
        CAMPAIGN,
        "", // human-owned campaign session
        "main",
        "camp-01",
        None,
        T0,
        T0 + 500_000,
        metrics(0, 0, 0, 0, 0.0),
    )
}

const F64_EPS: f64 = 1e-9;

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < F64_EPS
}

// ─────────────────────────────────────────────────────────────────────────────
// PR record
// ─────────────────────────────────────────────────────────────────────────────

/// The decomposed PR record: work = Σ landed intents (final attempts),
/// orchestration = the PR author's own spend, verification = the real panel
/// count off the log, waste = the retried + discarded spend — and the total
/// identity `work + orchestration + verification + ci = total` holds, with
/// waste EXCLUDED from total.
#[test]
fn pr_record_decomposition_adds_up() {
    let log = build_world_log();
    let ledger = Ledger::from_records(log.records());
    let landed = landed_ids_of(&log, &PR128_INTENTS);
    let verdicts = verdicts_of(&log, &PR128_INTENTS);
    assert_eq!(
        landed,
        vec!["i-a1", "i-a2", "i-a3"],
        "projected, not assumed"
    );
    assert_eq!(verdicts.len(), 2, "i-a1 + i-a3 panels off the log");

    let rec = pr_record(
        &pr128_envelope(),
        "cas:pr-128-envelope",
        &pr128_intent_envelopes(),
        &landed,
        &verdicts,
        pr128_ci(),
        PrQueueInput {
            queue_wait_ms: 12_000,
            human_touches: 1,
            landed_at: landed_at_of(&ledger, &PR128_INTENTS),
        },
    )
    .expect("orchestrator-authored PR computes");

    // Identity facts.
    assert_eq!(rec.pr_id, "PR-128");
    assert_eq!(rec.author.kind, PrAuthorKind::Orchestrator);
    assert_eq!(rec.author.model.as_deref(), Some("opus-4.8"));
    assert_eq!(rec.author.run_id.as_deref(), Some("orq-014"));
    assert_eq!(rec.author.principal, None);
    assert_eq!(rec.intent_count, 3);
    assert_eq!(
        rec.agent_count, 5,
        "5 distinct agent runs incl. retry + discard"
    );
    assert_eq!(rec.models_used, vec!["opus-4.8", "sonnet-4.6"]);
    assert_eq!(rec.envelope_ref, "cas:pr-128-envelope");

    // work = Σ landed final attempts (i-a1 + i-a2 final + i-a3).
    assert_eq!(rec.cost.work.tokens, 50_000 + 45_000 + 40_000);
    assert_eq!(rec.cost.work.tool_calls, 30 + 25 + 20);
    assert!(approx(rec.cost.work.cost_usd, 1.00 + 0.90 + 0.80));

    // orchestration = the PR author's own session spend.
    assert_eq!(rec.cost.orchestration.tokens, 20_000);
    assert_eq!(rec.cost.orchestration.tool_calls, 40);
    assert_eq!(rec.cost.orchestration.turns, 12);
    assert!(approx(rec.cost.orchestration.cost_usd, 0.50));

    // verification: the REAL panel count; spend has no seam yet (honest 0).
    assert_eq!(rec.cost.verification.verdict_panels, 2);
    assert_eq!(rec.cost.verification.tokens, 0);

    // ci passes through from the checks seam.
    assert_eq!(rec.cost.ci.cache_hit, 5);
    assert_eq!(rec.cost.ci.exec, 5);

    // waste = retried attempt (i-a2x) + discarded intent (i-a9), shown.
    assert_eq!(rec.cost.waste.discarded_intents, 1);
    assert_eq!(rec.cost.waste.retried_agents, 1);
    assert_eq!(rec.cost.waste.tokens_not_landed, 20_000 + 9_000);
    assert!(approx(rec.cost.waste.cost_usd, 0.40 + 0.18));

    // THE identity: total = work + orchestration + verification + ci,
    // waste excluded (ADR JSONC).
    assert_eq!(
        rec.cost.total.tokens,
        rec.cost.work.tokens + rec.cost.orchestration.tokens + rec.cost.verification.tokens
    );
    assert!(approx(
        rec.cost.total.cost_usd,
        rec.cost.work.cost_usd
            + rec.cost.orchestration.cost_usd
            + rec.cost.verification.cost_usd
            + rec.cost.ci.cost_usd
    ));
    // Waste really is excluded: adding it would change the total.
    assert!(rec.cost.waste.cost_usd > 0.0);
    assert!(!approx(
        rec.cost.total.cost_usd,
        rec.cost.total.cost_usd + rec.cost.waste.cost_usd
    ));

    // Efficiency: overhead = orchestration ÷ total; first-pass yield = 2 of
    // 4 authored ids landed without rework (i-a1, i-a3).
    assert!(approx(rec.efficiency.overhead_pct, 0.50 / 3.30 * 100.0));
    assert!(approx(
        rec.efficiency.cache_savings_pct,
        0.40 / 0.50 * 100.0
    ));
    assert!(approx(rec.efficiency.first_pass_yield, 2.0 / 4.0));
    // Honest gap: no LOC seam → 0.0, never a fabricated figure.
    assert!(approx(rec.efficiency.cost_per_net_kloc, 0.0));
}

/// Span vs sum (the product asset): three intents ran in PARALLEL, so the
/// clock span is shorter than the summed agent-time.
#[test]
fn wall_span_le_agent_sum_under_parallelism() {
    let log = build_world_log();
    let ledger = Ledger::from_records(log.records());
    let landed = landed_ids_of(&log, &PR128_INTENTS);
    let landed_at = landed_at_of(&ledger, &PR128_INTENTS);
    let rec = pr_record(
        &pr128_envelope(),
        "cas:pr-128-envelope",
        &pr128_intent_envelopes(),
        &landed,
        &verdicts_of(&log, &PR128_INTENTS),
        pr128_ci(),
        PrQueueInput {
            queue_wait_ms: 12_000,
            human_touches: 1,
            landed_at,
        },
    )
    .expect("computes");

    // agent_sum is GROSS agent-time: every attempt, waste included.
    assert_eq!(
        rec.time.agent_sum_ms,
        60_000 + 25_000 + 60_000 + 60_000 + 12_000
    );
    // wall span: first activity (the orchestrator's birth at T0) → landed.
    assert_eq!(rec.time.landed_at, landed_at);
    assert_eq!(rec.time.wall_span_ms, landed_at - T0);
    assert!(
        rec.time.wall_span_ms <= rec.time.agent_sum_ms,
        "parallel fleet: span ({}) ≤ sum ({})",
        rec.time.wall_span_ms,
        rec.time.agent_sum_ms
    );
    assert_eq!(rec.time.queue_wait_ms, 12_000);
    assert_eq!(rec.time.human_touches, 1);
}

/// D14 at the projection level: a subagent-authored envelope at
/// `altitude:"pr"` is REJECTED by the rollup path — either subagent marker
/// (spawned session, or a subagent `agent_type`) refuses, fail-closed.
#[test]
fn subagent_authored_pr_envelope_rejected() {
    let log = build_world_log();
    let landed = landed_ids_of(&log, &PR128_INTENTS);

    // A subagent session blob mislabelled as the PR's envelope.
    let subagent_pr = envelope(
        Altitude::Pr,
        "PR-128",
        "opus-4.8",
        "implementer", // subagent type
        "run-a1",
        Some("orq-014"), // spawned — a subagent
        T0,
        T0 + 10_000,
        metrics(1_000, 1_000, 1, 1, 0.01),
    );
    let err = pr_record(
        &subagent_pr,
        "cas:x",
        &[],
        &landed,
        &[],
        zero_ci(),
        PrQueueInput::default(),
    )
    .expect_err("subagent PR author must be impossible");
    assert!(
        matches!(
            &err,
            RollupError::SubagentAuthor { altitude: Altitude::Pr, agent_type, run_id, .. }
                if agent_type == "implementer" && run_id == "run-a1"
        ),
        "got {err:?}"
    );

    // Fail-closed on EITHER marker: agent_type says "main" but the session
    // was spawned → still a subagent → still rejected.
    let spawned_main = envelope(
        Altitude::Pr,
        "PR-128",
        "opus-4.8",
        "main",
        "run-x",
        Some("orq-014"),
        T0,
        T0 + 10_000,
        metrics(1_000, 1_000, 1, 1, 0.01),
    );
    assert!(matches!(
        pr_record(
            &spawned_main,
            "cas:x",
            &[],
            &landed,
            &[],
            zero_ci(),
            PrQueueInput::default(),
        ),
        Err(RollupError::SubagentAuthor { .. })
    ));

    // A subagent-authored campaign envelope is rejected the same way.
    let mut subagent_campaign = campaign_envelope();
    subagent_campaign.authorship.agent_type = "implementer".to_string();
    assert!(matches!(
        campaign_rollup(&subagent_campaign, "cas:c", &[]),
        Err(RollupError::SubagentAuthor {
            altitude: Altitude::Campaign,
            ..
        })
    ));
}

/// Altitude discipline: an intent envelope where the PR's own envelope
/// belongs (and vice versa) is refused, attributably.
#[test]
fn wrong_altitude_rejected() {
    let intent = intent_env(
        "i-a1",
        "opus-4.8",
        "run-a1",
        "orq-014",
        T0,
        T0 + 1_000,
        500,
        100,
        1,
        0.01,
    );
    assert_eq!(
        pr_record(
            &intent,
            "cas:x",
            &[],
            &[],
            &[],
            zero_ci(),
            PrQueueInput::default()
        )
        .expect_err("intent envelope is not a PR envelope"),
        RollupError::WrongAltitude {
            expected: Altitude::Pr,
            found: Altitude::Intent,
            intent_id: "i-a1".to_string(),
        }
    );
    // A PR envelope smuggled into the intents slot.
    assert!(matches!(
        pr_record(
            &pr128_envelope(),
            "cas:x",
            &[pr128_envelope()],
            &[],
            &[],
            zero_ci(),
            PrQueueInput::default()
        ),
        Err(RollupError::WrongAltitude {
            expected: Altitude::Intent,
            found: Altitude::Pr,
            ..
        })
    ));
    // A PR envelope where the campaign's belongs.
    assert!(matches!(
        campaign_rollup(&pr128_envelope(), "cas:x", &[]),
        Err(RollupError::WrongAltitude {
            expected: Altitude::Campaign,
            found: Altitude::Pr,
            ..
        })
    ));
}

/// Null-ref tolerance (sub-full capture): every fixture envelope already
/// carries all-`null` refs; additionally a HUMAN-authored PR whose second
/// landed intent has NO captured envelope at all still rolls up — zero spend,
/// counted as a first-pass landing.
#[test]
fn null_refs_and_missing_envelope_tolerated() {
    let log = build_world_log();
    let ledger = Ledger::from_records(log.records());
    let landed = landed_ids_of(&log, &PR129_INTENTS);
    assert_eq!(landed, vec!["i-b1", "i-b2"]);

    let envs = pr129_intent_envelopes();
    // The captured envelope really is sub-full: every ref is null.
    assert!(envs[0].trajectory.raw_transcript_ref.is_none());
    assert!(envs[0].trajectory.task_transcript_ref.is_none());
    assert!(envs[0].trajectory.journal_ref.is_none());
    assert!(envs[0].snapshot.prompt_ref.is_none());
    assert!(envs[0].verdicts_ref.is_none());

    let rec = pr_record(
        &pr129_envelope(),
        "cas:pr-129-envelope",
        &envs,
        &landed,
        &verdicts_of(&log, &PR129_INTENTS),
        pr129_ci(),
        PrQueueInput {
            queue_wait_ms: 8_000,
            human_touches: 3,
            landed_at: landed_at_of(&ledger, &PR129_INTENTS),
        },
    )
    .expect("sub-full capture rolls up");

    // Human author (no model session) → principal, never a subagent.
    assert_eq!(rec.author.kind, PrAuthorKind::Human);
    assert_eq!(rec.author.principal.as_deref(), Some("gustavo@humangr.com"));
    assert_eq!(rec.author.model, None);
    assert_eq!(rec.author.run_id, None);

    // i-b2 (no envelope) contributes zero spend; only i-b1's is counted.
    assert_eq!(rec.intent_count, 2);
    assert_eq!(rec.cost.work.tokens, 30_000);
    assert!(approx(rec.cost.work.cost_usd, 0.60));
    assert_eq!(rec.cost.waste.tokens_not_landed, 0);
    // Both ids landed without rework → first-pass yield 1.0.
    assert!(approx(rec.efficiency.first_pass_yield, 1.0));
    // No orchestration spend (human session metrics are zero) → overhead 0.
    assert!(approx(rec.efficiency.overhead_pct, 0.0));
    // Identity holds here too.
    assert!(approx(
        rec.cost.total.cost_usd,
        rec.cost.work.cost_usd + rec.cost.ci.cost_usd
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// Campaign rollup — the third altitude, over a multi-PR campaign
// ─────────────────────────────────────────────────────────────────────────────

/// The campaign consolidates its PRs (the landing-queue bundle of PRs, NOT
/// commits) with the SAME decomposition, plus progress — proven over a
/// 4-PR campaign: 2 landed, 1 in flight, 1 blocked.
#[test]
fn campaign_rollup_over_multi_pr_campaign() {
    let log = build_world_log();
    let ledger = Ledger::from_records(log.records());

    // The two landed PRs, computed from the projections (as above).
    let pr128 = pr_record(
        &pr128_envelope(),
        "cas:pr-128-envelope",
        &pr128_intent_envelopes(),
        &landed_ids_of(&log, &PR128_INTENTS),
        &verdicts_of(&log, &PR128_INTENTS),
        pr128_ci(),
        PrQueueInput {
            queue_wait_ms: 12_000,
            human_touches: 1,
            landed_at: landed_at_of(&ledger, &PR128_INTENTS),
        },
    )
    .expect("PR-128 computes");
    let pr129 = pr_record(
        &pr129_envelope(),
        "cas:pr-129-envelope",
        &pr129_intent_envelopes(),
        &landed_ids_of(&log, &PR129_INTENTS),
        &verdicts_of(&log, &PR129_INTENTS),
        pr129_ci(),
        PrQueueInput {
            queue_wait_ms: 8_000,
            human_touches: 3,
            landed_at: landed_at_of(&ledger, &PR129_INTENTS),
        },
    )
    .expect("PR-129 computes");

    // An in-flight and a blocked PR: nothing landed — spend is waste-so-far.
    let (pr130_env, pr130_intents) = inflight_pr(
        "PR-130",
        "orq-015",
        T0 + 300_000,
        intent_env(
            "i-c1",
            "opus-4.8",
            "run-c1",
            "orq-015",
            T0 + 305_000,
            T0 + 325_000,
            15_000,
            12_000,
            7,
            0.24,
        ),
    );
    let pr130 = pr_record(
        &pr130_env,
        "cas:pr-130-envelope",
        &pr130_intents,
        &[],
        &[],
        zero_ci(),
        PrQueueInput {
            queue_wait_ms: 30_000,
            human_touches: 0,
            landed_at: 0,
        },
    )
    .expect("in-flight PR computes");
    assert_eq!(pr130.cost.waste.discarded_intents, 1);
    assert_eq!(pr130.cost.work.tokens, 0);

    let (pr131_env, pr131_intents) = inflight_pr(
        "PR-131",
        "orq-016",
        T0 + 400_000,
        intent_env(
            "i-d1",
            "opus-4.8",
            "run-d1",
            "orq-016",
            T0 + 405_000,
            T0 + 418_000,
            9_000,
            10_000,
            5,
            0.20,
        ),
    );
    let pr131 = pr_record(
        &pr131_env,
        "cas:pr-131-envelope",
        &pr131_intents,
        &[],
        &[],
        zero_ci(),
        PrQueueInput {
            queue_wait_ms: 60_000,
            human_touches: 0,
            landed_at: 0,
        },
    )
    .expect("blocked PR computes");

    let prs = vec![
        (pr128.clone(), PrPhase::Landed),
        (pr129.clone(), PrPhase::Landed),
        (pr130.clone(), PrPhase::InFlight),
        (pr131.clone(), PrPhase::Blocked),
    ];
    let camp = campaign_rollup(&campaign_envelope(), "cas:campaign-envelope", &prs)
        .expect("human-owned campaign computes");

    // Identity facts off the campaign's own envelope.
    assert_eq!(camp.campaign, CAMPAIGN);
    assert_eq!(camp.owner.principal, "gustavo@humangr.com");
    assert_eq!(camp.envelope_ref, "cas:campaign-envelope");
    assert_eq!(camp.pr_ids, vec!["PR-128", "PR-129", "PR-130", "PR-131"]);
    assert_eq!(camp.pr_count, 4);
    assert_eq!(camp.intent_count, 5, "landed intents across the campaign");
    assert_eq!(camp.agent_count, 8, "Σ distinct agent runs per PR");
    assert_eq!(camp.models_used, vec!["opus-4.8", "sonnet-4.6"]);

    // Progress: 2 landed · 1 in flight · 1 blocked.
    assert_eq!(camp.progress.landed, 2);
    assert_eq!(camp.progress.in_flight, 1);
    assert_eq!(camp.progress.blocked, 1);

    // Cost = component-wise Σ over the PRs (same vocabulary), checked against
    // the COMPUTED PR records — not hand-tallied constants.
    let all = [&pr128, &pr129, &pr130, &pr131];
    assert_eq!(
        camp.cost.work.tokens,
        all.iter().map(|p| p.cost.work.tokens).sum::<u64>()
    );
    assert_eq!(
        camp.cost.orchestration.tokens,
        all.iter().map(|p| p.cost.orchestration.tokens).sum::<u64>()
    );
    assert_eq!(
        camp.cost.verification.verdict_panels,
        all.iter()
            .map(|p| p.cost.verification.verdict_panels)
            .sum::<u64>()
    );
    assert_eq!(
        camp.cost.ci.cache_hit,
        all.iter().map(|p| p.cost.ci.cache_hit).sum::<u64>()
    );
    assert_eq!(
        camp.cost.waste.discarded_intents,
        all.iter()
            .map(|p| p.cost.waste.discarded_intents)
            .sum::<u64>()
    );
    assert_eq!(
        camp.cost.waste.tokens_not_landed,
        all.iter()
            .map(|p| p.cost.waste.tokens_not_landed)
            .sum::<u64>()
    );
    assert!(approx(
        camp.cost.work.cost_usd,
        all.iter().map(|p| p.cost.work.cost_usd).sum::<f64>()
    ));

    // THE identity at the third altitude too: total = work + orchestration +
    // verification + ci; waste shown, excluded.
    assert_eq!(
        camp.cost.total.tokens,
        camp.cost.work.tokens + camp.cost.orchestration.tokens + camp.cost.verification.tokens
    );
    assert!(approx(
        camp.cost.total.cost_usd,
        camp.cost.work.cost_usd
            + camp.cost.orchestration.cost_usd
            + camp.cost.verification.cost_usd
            + camp.cost.ci.cost_usd
    ));

    // Time: lead time = campaign opened (T0) → last PR landed; agent_sum is
    // the Σ over PRs — and the fleet parallelism shows at this altitude too.
    let last_landed = landed_at_of(&ledger, &PR129_INTENTS);
    assert_eq!(camp.time.wall_span_ms, last_landed - T0);
    assert_eq!(
        camp.time.agent_sum_ms,
        all.iter().map(|p| p.time.agent_sum_ms).sum::<u64>()
    );
    assert!(camp.time.wall_span_ms <= camp.time.agent_sum_ms);
    assert_eq!(
        camp.time.queue_wait_ms,
        all.iter().map(|p| p.time.queue_wait_ms).sum::<u64>()
    );

    // First-pass yield, weighted by authored intents per PR:
    // PR-128: 2 of 4 · PR-129: 2 of 2 · PR-130: 0 of 1 · PR-131: 0 of 1
    // → 4 of 8.
    assert!(approx(camp.efficiency.first_pass_yield, 4.0 / 8.0));
    // Overhead at the campaign altitude: orchestration ÷ total over the sums.
    assert!(approx(
        camp.efficiency.overhead_pct,
        camp.cost.orchestration.cost_usd / camp.cost.total.cost_usd * 100.0
    ));
}
