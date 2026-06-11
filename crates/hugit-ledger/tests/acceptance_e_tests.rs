//! WP-E-TESTS: the three P-TEST-GAPS closures (adversarial Round-1 audit).
//!
//! 1. **e2e_four_altitude_chain** — ONE test: capture → real `InMemoryColdStore`
//!    put → rollup projection → resolves a NON-NULL `context_ref` back to real
//!    bytes, for all four altitudes (intent / pr / campaign / session) in one
//!    wave.
//!
//! 2. **cold_store_failure_mid_sequence** — `FailingColdStore` impl that
//!    returns an error after N `put` calls; asserts `close_envelope` surfaces a
//!    structured `EnvelopeError::Store` (not a panic, not a partial-write).
//!
//! 3. **rollup_at_scale_decomposition_identity** — `campaign_rollup` over
//!    1 000+ intents across many PRs; asserts the decomposition identity holds
//!    exactly (integer micro-USD, no drift) and no overflow panic occurs.

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

use std::sync::Mutex;

use hugit_contracts::context_envelope::{
    Altitude, Authorship, CiCost, FileRead, IntentMetrics, PrAuthorKind, Snapshot, Spawn,
    TokenCounts, Trajectory, WasteCost,
};
use hugit_contracts::{CONTEXT_ENVELOPE_SCHEMA_VERSION, ContextEnvelope as CE};
use hugit_ledger::envelope::{
    CaptureLevel, ColdBlobStore, ColdStoreError, EnvelopeDraft, EnvelopeError, GetOutcome,
    InMemoryColdStore, Tombstone, TombstoneRecord, close_envelope, close_session_envelope,
};
use hugit_ledger::rollup::{PrPhase, PrQueueInput, campaign_rollup, pr_record};

const T0: u64 = 1_760_000_000_000;

fn fake_oid(n: usize) -> String {
    format!("{n:040x}")
}
fn fake_tree(n: usize) -> String {
    format!("{n:064x}")
}

fn null_trajectory() -> Trajectory {
    Trajectory {
        raw_transcript_ref: None,
        task_transcript_ref: None,
        summary: None,
        journal_ref: None,
        redaction_policy: "default-v1".to_string(),
    }
}

fn snapshot_stub() -> Snapshot {
    Snapshot {
        files_read: vec![],
        prompt_ref: None,
        env_manifest: "rustc 1.96.0".to_string(),
    }
}

fn metrics_stub(tokens: u64, cost_micros: u64) -> IntentMetrics {
    IntentMetrics {
        tokens: TokenCounts {
            input: tokens / 2,
            output: tokens - tokens / 2,
            cache_read: 0,
            cache_write: 0,
            total: tokens,
        },
        wall_ms: 1000,
        active_ms: 800,
        tool_calls: 1,
        tool_breakdown: vec![],
        model_turns: 1,
        cost_usd_micros: cost_micros,
    }
}

/// Build a minimal valid intent envelope.
fn intent_env(id: &str, run_id: &str, parent: &str, tokens: u64, cost_micros: u64) -> CE {
    CE {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.to_string(),
        altitude: Altitude::Intent,
        intent_id: id.to_string(),
        commit: fake_oid(1),
        tree_hash: fake_tree(1),
        authorship: Authorship {
            model: "test-model".to_string(),
            model_digest: "0".repeat(64),
            agent_type: "implementer".to_string(),
            spawn: Spawn {
                run_id: run_id.to_string(),
                parent_run_id: Some(parent.to_string()),
                born_at: T0 + 1_000,
                died_at: T0 + 5_000,
            },
            operator: "test@humangr.com".to_string(),
        },
        charter: format!("close {id}"),
        campaign: Some("test-campaign".to_string()),
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        trajectory: null_trajectory(),
        snapshot: snapshot_stub(),
        metrics: metrics_stub(tokens, cost_micros),
        verdicts_ref: None,
    }
}

/// Build a minimal valid PR envelope (orchestrator-authored, top-level session).
fn pr_env(pr_id: &str, run_id: &str, tokens: u64, cost_micros: u64) -> CE {
    CE {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.to_string(),
        altitude: Altitude::Pr,
        intent_id: pr_id.to_string(),
        commit: fake_oid(2),
        tree_hash: fake_tree(2),
        authorship: Authorship {
            model: "test-model".to_string(),
            model_digest: "0".repeat(64),
            agent_type: "main".to_string(),
            spawn: Spawn {
                run_id: run_id.to_string(),
                parent_run_id: None, // top-level session
                born_at: T0,
                died_at: T0 + 10_000,
            },
            operator: "test@humangr.com".to_string(),
        },
        charter: format!("orchestrate {pr_id}"),
        campaign: Some("test-campaign".to_string()),
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        trajectory: null_trajectory(),
        snapshot: snapshot_stub(),
        metrics: metrics_stub(tokens, cost_micros),
        verdicts_ref: None,
    }
}

/// Build a campaign envelope (human-owned, top-level session).
fn campaign_env(key: &str) -> CE {
    CE {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.to_string(),
        altitude: Altitude::Campaign,
        intent_id: key.to_string(),
        commit: fake_oid(3),
        tree_hash: fake_tree(3),
        authorship: Authorship {
            model: "".to_string(),
            model_digest: "".to_string(),
            agent_type: "main".to_string(),
            spawn: Spawn {
                run_id: "camp-session".to_string(),
                parent_run_id: None,
                born_at: T0,
                died_at: T0 + 50_000,
            },
            operator: "test@humangr.com".to_string(),
        },
        charter: format!("campaign {key}"),
        campaign: Some(key.to_string()),
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        trajectory: null_trajectory(),
        snapshot: snapshot_stub(),
        metrics: metrics_stub(0, 0),
        verdicts_ref: None,
    }
}

fn zero_ci() -> CiCost {
    CiCost {
        cache_hit: 0,
        exec: 0,
        cost_usd_micros: 0,
        saved_usd_micros: 0,
    }
}

fn default_queue(landed_at: u64) -> PrQueueInput {
    PrQueueInput {
        queue_wait_ms: 0,
        human_touches: 0,
        landed_at,
    }
}

fn authorship_for_draft(run_id: &str, parent: Option<&str>, agent_type: &str) -> Authorship {
    Authorship {
        model: "in-process-deterministic".to_string(),
        model_digest: "0".repeat(64),
        agent_type: agent_type.to_string(),
        spawn: Spawn {
            run_id: run_id.to_string(),
            parent_run_id: parent.map(str::to_string),
            born_at: T0,
            died_at: T0 + 1_000,
        },
        operator: "test@humangr.com".to_string(),
    }
}

/// Build a minimal [`EnvelopeDraft`] with non-empty transcripts so Full capture
/// is valid (two-transcript imperative).
fn draft(
    altitude: Altitude,
    unit_id: &str,
    agent_type: &str,
    parent: Option<&str>,
) -> EnvelopeDraft {
    EnvelopeDraft {
        altitude,
        unit_id: unit_id.to_string(),
        commit: fake_oid(0),
        tree_hash: fake_tree(0),
        authorship: authorship_for_draft(unit_id, parent, agent_type),
        charter: format!("close {unit_id}"),
        campaign: Some("e-tests-campaign".to_string()),
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        raw_transcript: vec!["raw event line 1".to_string()],
        task_transcript: vec!["task event line 1".to_string()],
        summary: "done".to_string(),
        journal_ref: None,
        files_read: vec![FileRead {
            path: "src/main.rs".to_string(),
            hash: format!("sha256:{}", "a".repeat(64)),
        }],
        prompt: Some("system: close the unit".to_string()),
        env_manifest: "rustc 1.96.0".to_string(),
        metrics: metrics_stub(100, 500_000),
        verdicts_ref: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1 — End-to-end four-altitude chain
// ─────────────────────────────────────────────────────────────────────────────

/// **P-TEST-GAPS / complete F4**: ONE test that drives capture → real
/// `InMemoryColdStore` put → rollup projection → resolves a NON-NULL
/// `context_ref` back to real bytes, asserting the full chain holds for all
/// four altitudes (intent / pr / session / campaign) in a single wave.
///
/// Path: `close_envelope` (intent, Full) + `close_session_envelope` (session,
/// pr, campaign) → `InMemoryColdStore` → `store.get(context_ref).present()`
/// → `serde_json::from_slice::<ContextEnvelope>` → `campaign_rollup`.
/// Every altitude's `context_ref` resolves to real bytes that re-parse through
/// the frozen shape and round-trip equal to the in-memory envelope.
#[test]
fn e2e_four_altitude_chain() {
    let store = InMemoryColdStore::new();
    let no_waste = WasteCost {
        discarded_intents: 0,
        retried_agents: 0,
        tokens_not_landed: 0,
        cost_usd_micros: 0,
    };

    // ── altitude: Intent ─────────────────────────────────────────────────────
    let intent_draft = draft(Altitude::Intent, "i-e1", "implementer", Some("orq-e1"));
    let closed_intent =
        close_envelope(&intent_draft, CaptureLevel::Full, &store).expect("intent close");

    // The context_ref is NON-NULL and resolves to real bytes.
    assert!(
        !closed_intent.context_ref.is_empty(),
        "intent context_ref must be non-null"
    );
    let intent_bytes = store
        .get(&closed_intent.context_ref)
        .expect("store get intent")
        .present()
        .expect("intent ref must be Present, not Absent or Erased");
    let intent_back: CE =
        serde_json::from_slice(&intent_bytes).expect("intent bytes parse as ContextEnvelope");
    assert_eq!(
        intent_back, closed_intent.envelope,
        "intent envelope round-trips through the store"
    );
    assert_eq!(intent_back.altitude, Altitude::Intent);
    // Both transcript refs are non-null (Full level; two-transcript imperative).
    assert!(
        intent_back.trajectory.raw_transcript_ref.is_some(),
        "intent raw_transcript_ref must be present under Full"
    );
    assert!(
        intent_back.trajectory.task_transcript_ref.is_some(),
        "intent task_transcript_ref must be present under Full"
    );
    // Every transcript ref itself resolves to real bytes in the store.
    let raw_ref = intent_back.trajectory.raw_transcript_ref.as_ref().unwrap();
    let _raw_bytes = store
        .get(raw_ref)
        .expect("store get raw transcript")
        .present()
        .expect("raw transcript ref must resolve");
    let task_ref = intent_back.trajectory.task_transcript_ref.as_ref().unwrap();
    let _task_bytes = store
        .get(task_ref)
        .expect("store get task transcript")
        .present()
        .expect("task transcript ref must resolve");

    // ── altitude: Session ────────────────────────────────────────────────────
    let session_draft = draft(Altitude::Session, "orq-e-session", "main", None);
    let session_emission =
        close_session_envelope(&session_draft, CaptureLevel::Full, &store, no_waste.clone())
            .expect("session close");

    assert!(
        !session_emission.envelope_ref.is_empty(),
        "session envelope_ref must be non-null"
    );
    let session_bytes = store
        .get(&session_emission.envelope_ref)
        .expect("store get session")
        .present()
        .expect("session ref must be Present");
    let session_back: CE =
        serde_json::from_slice(&session_bytes).expect("session bytes parse as ContextEnvelope");
    assert_eq!(session_back, session_emission.envelope);
    assert_eq!(session_back.altitude, Altitude::Session);
    assert!(session_back.trajectory.raw_transcript_ref.is_some());
    assert!(session_back.trajectory.task_transcript_ref.is_some());

    // ── altitude: Pr ─────────────────────────────────────────────────────────
    let pr_draft = draft(Altitude::Pr, "PR-E1", "main", None);
    let pr_emission =
        close_session_envelope(&pr_draft, CaptureLevel::Full, &store, no_waste.clone())
            .expect("pr close");

    assert!(
        !pr_emission.envelope_ref.is_empty(),
        "pr envelope_ref must be non-null"
    );
    let pr_bytes = store
        .get(&pr_emission.envelope_ref)
        .expect("store get pr")
        .present()
        .expect("pr ref must be Present");
    let pr_back: CE = serde_json::from_slice(&pr_bytes).expect("pr bytes parse as ContextEnvelope");
    assert_eq!(pr_back, pr_emission.envelope);
    assert_eq!(pr_back.altitude, Altitude::Pr);
    assert!(pr_back.trajectory.raw_transcript_ref.is_some());
    assert!(pr_back.trajectory.task_transcript_ref.is_some());

    // ── altitude: Campaign ───────────────────────────────────────────────────
    let campaign_draft = draft(Altitude::Campaign, "e-tests-campaign", "main", None);
    let campaign_emission =
        close_session_envelope(&campaign_draft, CaptureLevel::Full, &store, no_waste)
            .expect("campaign close");

    assert!(
        !campaign_emission.envelope_ref.is_empty(),
        "campaign envelope_ref must be non-null"
    );
    let campaign_bytes = store
        .get(&campaign_emission.envelope_ref)
        .expect("store get campaign")
        .present()
        .expect("campaign ref must be Present");
    let campaign_back: CE =
        serde_json::from_slice(&campaign_bytes).expect("campaign bytes parse as ContextEnvelope");
    assert_eq!(campaign_back, campaign_emission.envelope);
    assert_eq!(campaign_back.altitude, Altitude::Campaign);
    assert!(campaign_back.trajectory.raw_transcript_ref.is_some());
    assert!(campaign_back.trajectory.task_transcript_ref.is_some());

    // ── the four-altitude family is proven: all four altitudes are present ───
    let altitudes: std::collections::BTreeSet<String> = [
        closed_intent.envelope.altitude,
        session_emission.envelope.altitude,
        pr_emission.envelope.altitude,
        campaign_emission.envelope.altitude,
    ]
    .iter()
    .map(|a| format!("{a:?}"))
    .collect();
    assert_eq!(
        altitudes,
        ["Campaign", "Intent", "Pr", "Session"]
            .iter()
            .map(|s| s.to_string())
            .collect::<std::collections::BTreeSet<_>>(),
        "all four altitudes proven in one wave"
    );

    // ── rollup projection over the PR and campaign envelopes ─────────────────
    // Build minimal rollup inputs using the raw fixture envelopes (not the
    // closed ones, which carry the capture-level gated form).  The rollup reads
    // only authorship + metrics; the context_ref chain above already proved the
    // store round-trip.
    let i_env = intent_env("i-e1", "run-e1", "orq-e1", 100, 200_000);
    let p_env = pr_env("PR-E1", "orq-e1", 20, 50_000);
    let c_env = campaign_env("e-tests-campaign");

    let pr_rec = pr_record(
        &p_env,
        &pr_emission.envelope_ref,
        &[i_env],
        &["i-e1".to_string()],
        &[],
        zero_ci(),
        default_queue(T0 + 20_000),
    )
    .expect("pr_record must compute");

    assert_eq!(pr_rec.pr_id, "PR-E1");
    assert_eq!(pr_rec.intent_count, 1);
    assert_eq!(pr_rec.author.kind, PrAuthorKind::Orchestrator);

    // The decomposition identity: total = work + orchestration + ci (no
    // verification spend in this minimal fixture).
    assert_eq!(
        pr_rec.cost.total.cost_usd_micros,
        pr_rec.cost.work.cost_usd_micros
            + pr_rec.cost.orchestration.cost_usd_micros
            + pr_rec.cost.ci.cost_usd_micros
    );

    let camp_rec = campaign_rollup(
        &c_env,
        &campaign_emission.envelope_ref,
        &[(pr_rec, PrPhase::Landed)],
    )
    .expect("campaign_rollup must compute");

    assert_eq!(camp_rec.campaign, "e-tests-campaign");
    assert_eq!(camp_rec.pr_count, 1);
    // Identity holds at the campaign altitude too.
    assert_eq!(
        camp_rec.cost.total.cost_usd_micros,
        camp_rec.cost.work.cost_usd_micros
            + camp_rec.cost.orchestration.cost_usd_micros
            + camp_rec.cost.ci.cost_usd_micros
    );

    // The campaign envelope_ref resolves to real bytes in the store (the chain
    // closes: capture → store → rollup → ref back to bytes).
    let _camp_bytes = store
        .get(&camp_rec.envelope_ref)
        .expect("campaign_rollup envelope_ref must resolve in store")
        .present()
        .expect("campaign envelope_ref must be Present");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2 — Mid-write cold-store failure injection
// ─────────────────────────────────────────────────────────────────────────────

/// A `FailingColdStore` that returns `ColdStoreError::Io` after N successful
/// `put` calls — simulating a cold-store failure mid-sequence (e.g. the third
/// blob write fails after the raw and task transcripts have already been put).
///
/// `get` and `erase` are not exercised here and delegate to `InMemoryColdStore`
/// so any earlier successful puts remain accessible for assertions if needed.
struct FailingColdStore {
    /// The real backing store for successful puts.
    inner: InMemoryColdStore,
    /// How many puts to allow before failing.
    fail_after: u64,
    /// Counter of puts attempted so far.
    put_count: Mutex<u64>,
}

impl FailingColdStore {
    fn new(fail_after: u64) -> Self {
        Self {
            inner: InMemoryColdStore::new(),
            fail_after,
            put_count: Mutex::new(0),
        }
    }
}

impl ColdBlobStore for FailingColdStore {
    fn put(&self, bytes: &[u8]) -> Result<String, ColdStoreError> {
        let mut count = self.put_count.lock().expect("count lock poisoned");
        *count += 1;
        if *count > self.fail_after {
            return Err(ColdStoreError::Io(format!(
                "injected failure on put #{count} (fail_after={fail_after})",
                fail_after = self.fail_after,
            )));
        }
        drop(count); // release before delegating
        self.inner.put(bytes)
    }

    fn get(&self, blob_ref: &str) -> Result<GetOutcome, ColdStoreError> {
        self.inner.get(blob_ref)
    }

    fn erase(&self, blob_ref: &str, record: TombstoneRecord) -> Result<Tombstone, ColdStoreError> {
        self.inner.erase(blob_ref, record)
    }
}

/// **P-TEST-GAPS / complete F5**: a `FailingColdStore` whose `put` returns an
/// error after N calls; asserts `close_envelope` surfaces a structured
/// `EnvelopeError::Store(ColdStoreError::Io(_))` — NOT a panic and NOT a
/// partial-write that corrupts. The test sweeps every failure point (fail at
/// put #1, #2, …, #N) so every blob in the write sequence is covered.
///
/// WA3 added the `ColdBlobStore` trait; WC2 made the serialize path fail-
/// closed. This test proves the STORE-FAULT path (an Io error from the
/// backing tier) surfaces correctly at every position in the blob sequence.
#[test]
fn cold_store_failure_mid_sequence() {
    let draft_full = draft(Altitude::Intent, "i-fail", "implementer", Some("orq-fail"));

    // Under CaptureLevel::Full, close_envelope writes these blobs in sequence:
    //   1. raw_transcript (full level)
    //   2. task_transcript (task level ⊆ full)
    //   3. prompt (full level, draft.prompt = Some(...))
    //   4. envelope blob (the context_ref put)
    // Failing at any of the four positions must yield EnvelopeError::Store.
    for fail_after in 0..4 {
        let store = FailingColdStore::new(fail_after);
        let result = close_envelope(&draft_full, CaptureLevel::Full, &store);
        match result {
            Err(EnvelopeError::Store(ColdStoreError::Io(ref msg))) => {
                // Correct: structured error surfaced, not a panic.
                assert!(
                    msg.contains("injected failure"),
                    "Store error must carry the injected message; got: {msg}"
                );
            }
            Err(other) => {
                panic!("fail_after={fail_after}: expected EnvelopeError::Store(Io), got {other:?}")
            }
            Ok(_) => panic!("fail_after={fail_after}: expected a store failure error, got Ok"),
        }
        // No panic: the process is still alive here — fail_after={fail_after} passed.
    }

    // Sanity: fail_after=4 (no failure in the 4-put sequence) succeeds cleanly.
    let store_ok = FailingColdStore::new(4);
    let closed = close_envelope(&draft_full, CaptureLevel::Full, &store_ok)
        .expect("fail_after=4 means no failure, close must succeed");
    assert!(closed.envelope.trajectory.raw_transcript_ref.is_some());
    assert!(closed.envelope.trajectory.task_transcript_ref.is_some());
    // The context_ref resolves in the real inner store (no corruption).
    let bytes = store_ok
        .inner
        .get(&closed.context_ref)
        .expect("get context_ref")
        .present()
        .expect("context_ref must be Present");
    let back: CE = serde_json::from_slice(&bytes).expect("parses as ContextEnvelope");
    assert_eq!(back, closed.envelope, "round-trip after clean write");

    // The Metrics level uses fewer puts (no raw/prompt), fail at put #1.
    let draft_metrics = draft(
        Altitude::Intent,
        "i-fail-m",
        "implementer",
        Some("orq-fail"),
    );
    // At Metrics level: only the envelope blob is put (1 put).
    let store_metrics = FailingColdStore::new(0); // fail on put #1
    let result_metrics = close_envelope(&draft_metrics, CaptureLevel::Metrics, &store_metrics);
    assert!(
        matches!(
            result_metrics,
            Err(EnvelopeError::Store(ColdStoreError::Io(_)))
        ),
        "Metrics-level put failure must surface as Store error: {result_metrics:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3 — Rollup at scale
// ─────────────────────────────────────────────────────────────────────────────

/// **P-TEST-GAPS / complete F6**: `campaign_rollup` over 1 000+ intents across
/// many PRs — asserts:
/// - the decomposition identity holds EXACTLY (integer micro-USD, no drift);
/// - no overflow panic (saturating/checked arithmetic holds at scale);
/// - `campaign_rollup` completes (no `RollupError::Overflow` on realistic
///   per-intent token/cost values).
///
/// Scale: 50 PRs × 20 intents each = 1 000 intents.
/// Each intent: 10 000 tokens, 200 micro-USD cost.
/// Each PR: 5 000 tokens orchestration, 100 micro-USD orchestration.
/// No verdicts, no CI costs (honest zeros).
#[test]
fn rollup_at_scale_decomposition_identity() {
    const PR_COUNT: usize = 50;
    const INTENTS_PER_PR: usize = 20;
    const INTENT_TOKENS: u64 = 10_000;
    const INTENT_COST_MICROS: u64 = 200;
    const PR_TOKENS: u64 = 5_000;
    const PR_COST_MICROS: u64 = 100;

    let camp_key = "scale-campaign";
    let c_env = campaign_env(camp_key);

    let mut pr_records_with_phase = Vec::with_capacity(PR_COUNT);

    for pr_idx in 0..PR_COUNT {
        let pr_id = format!("PR-{pr_idx:04}");
        let pr_run_id = format!("orq-{pr_idx:04}");
        let p_env = pr_env(&pr_id, &pr_run_id, PR_TOKENS, PR_COST_MICROS);

        // Build N intent envelopes for this PR, all landed.
        let mut intent_envs = Vec::with_capacity(INTENTS_PER_PR);
        let mut landed_ids = Vec::with_capacity(INTENTS_PER_PR);
        for i_idx in 0..INTENTS_PER_PR {
            let i_id = format!("i-{pr_idx:04}-{i_idx:04}");
            let run_id = format!("run-{pr_idx:04}-{i_idx:04}");
            let env = intent_env(
                &i_id,
                &run_id,
                &pr_run_id,
                INTENT_TOKENS,
                INTENT_COST_MICROS,
            );
            landed_ids.push(i_id);
            intent_envs.push(env);
        }

        let envelope_ref = format!("cas:{:064x}", pr_idx);
        let rec = pr_record(
            &p_env,
            &envelope_ref,
            &intent_envs,
            &landed_ids,
            &[], // no verdicts
            zero_ci(),
            default_queue(T0 + 50_000 + pr_idx as u64 * 1_000),
        )
        .expect("pr_record must succeed at scale");

        // Per-PR identity: total = work + orchestration + ci (ci = 0 here).
        assert_eq!(
            rec.cost.total.cost_usd_micros,
            rec.cost.work.cost_usd_micros
                + rec.cost.orchestration.cost_usd_micros
                + rec.cost.ci.cost_usd_micros,
            "PR-{pr_idx} decomposition identity violated"
        );
        assert_eq!(
            rec.cost.work.cost_usd_micros,
            INTENTS_PER_PR as u64 * INTENT_COST_MICROS,
            "PR-{pr_idx} work cost mismatch"
        );
        assert_eq!(
            rec.cost.orchestration.cost_usd_micros, PR_COST_MICROS,
            "PR-{pr_idx} orchestration cost mismatch"
        );
        assert_eq!(rec.intent_count, INTENTS_PER_PR as u64);
        assert_eq!(rec.cost.waste.discarded_intents, 0, "no waste at scale");

        pr_records_with_phase.push((rec, PrPhase::Landed));
    }

    let camp_ref = "cas:scale-campaign-ref";
    let camp = campaign_rollup(&c_env, camp_ref, &pr_records_with_phase)
        .expect("campaign_rollup must succeed over 1000+ intents");

    assert_eq!(camp.pr_count, PR_COUNT as u64);
    assert_eq!(
        camp.intent_count,
        (PR_COUNT * INTENTS_PER_PR) as u64,
        "total intent count must be PR_COUNT * INTENTS_PER_PR"
    );

    // Expected totals (exact integer arithmetic).
    let expected_work_cost = PR_COUNT as u64 * INTENTS_PER_PR as u64 * INTENT_COST_MICROS;
    let expected_orch_cost = PR_COUNT as u64 * PR_COST_MICROS;
    let expected_total_cost = expected_work_cost + expected_orch_cost;

    assert_eq!(
        camp.cost.work.cost_usd_micros, expected_work_cost,
        "campaign work cost must equal exact sum of all PR work costs (no drift)"
    );
    assert_eq!(
        camp.cost.orchestration.cost_usd_micros, expected_orch_cost,
        "campaign orchestration cost must equal exact sum (no drift)"
    );
    assert_eq!(
        camp.cost.total.cost_usd_micros, expected_total_cost,
        "campaign total must be work + orchestration (no drift, no overflow)"
    );

    // The decomposition identity at campaign altitude (exact, no epsilon).
    assert_eq!(
        camp.cost.total.cost_usd_micros,
        camp.cost.work.cost_usd_micros
            + camp.cost.orchestration.cost_usd_micros
            + camp.cost.verification.cost_usd_micros
            + camp.cost.ci.cost_usd_micros,
        "campaign decomposition identity violated at scale"
    );

    // Token counts (same exact arithmetic).
    let expected_work_tokens = PR_COUNT as u64 * INTENTS_PER_PR as u64 * INTENT_TOKENS;
    let expected_orch_tokens = PR_COUNT as u64 * PR_TOKENS;
    assert_eq!(camp.cost.work.tokens, expected_work_tokens);
    assert_eq!(camp.cost.orchestration.tokens, expected_orch_tokens);
    assert_eq!(
        camp.cost.total.tokens,
        camp.cost.work.tokens + camp.cost.orchestration.tokens + camp.cost.verification.tokens,
        "campaign token decomposition identity violated at scale"
    );

    // All PRs landed → no waste, no in-flight, no blocked.
    assert_eq!(camp.progress.landed, PR_COUNT as u64);
    assert_eq!(camp.progress.in_flight, 0);
    assert_eq!(camp.progress.blocked, 0);
    assert_eq!(camp.cost.waste.discarded_intents, 0);
    assert_eq!(camp.cost.waste.tokens_not_landed, 0);
    assert_eq!(camp.cost.waste.cost_usd_micros, 0);
}
