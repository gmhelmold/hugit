//! WP-F2 DoD proof — a metrics+trajectory bundle against REAL spawned
//! intents (the dogfood path extended, ADR-0001).
//!
//! The wave's intents do their real work (the memoized check through the
//! actual `hugit-checks` client) under a recorder; at intent close the
//! producer writes the frozen `ContextEnvelope` behind a `context_ref`
//! wired into the `IntentSidecar`. The orchestrator session then lands the
//! wave through the real Phase-B engine and emits the full FOUR-altitude
//! family — its own `altitude:"session"` home envelope, one `altitude:"pr"`
//! envelope per landed PR, and one `altitude:"campaign"` envelope (PR +
//! campaign referencing the deduped session-home blobs, WP-F2b) plus waste
//! — hermetic, measured.

use hugit_checks::client::ac::InMemoryAc;
use hugit_contracts::{Altitude, ContextEnvelope};
use hugit_dogfood::envelope::{DOGFOOD_CAMPAIGN, run_wave_with_envelope_capture};
use hugit_dogfood::wave::WaveConfig;
use hugit_ledger::envelope::{CaptureLevel, ColdBlobStore, InMemoryColdStore};

/// Resolve a ref and parse the blob through the FROZEN envelope type.
fn fetch_envelope<S: ColdBlobStore>(store: &S, blob_ref: &str) -> ContextEnvelope {
    let bytes = store
        .get(blob_ref)
        .expect("cold-store get")
        .expect("the ref must resolve");
    serde_json::from_slice(&bytes).expect("stored blob parses through the frozen ContextEnvelope")
}

#[test]
fn bundle_proven_against_real_spawned_intents() {
    let cfg = WaveConfig::five_pr_disjoint_green();
    let ac = InMemoryAc::new();
    let store = InMemoryColdStore::new();

    let captured = run_wave_with_envelope_capture(&cfg, &ac, &store, CaptureLevel::default());

    // The wave really landed through the real engine.
    assert_eq!(captured.report.landed.len(), 5);
    assert!(captured.report.excluded.is_empty());

    // ── per-intent: envelope behind the sidecar's context_ref ────────────────
    assert_eq!(captured.intents.len(), 5);
    for intent in &captured.intents {
        // Sidecar wiring: context_ref resolves to the stored envelope, which
        // round-trips through the frozen type and equals the closed one.
        assert!(!intent.sidecar.authoritative);
        assert_eq!(intent.sidecar.context_ref, intent.closed.context_ref);
        let env = fetch_envelope(&store, &intent.sidecar.context_ref);
        assert_eq!(env, intent.closed.envelope);
        assert_eq!(env.altitude, Altitude::Intent);
        assert_eq!(env.intent_id, format!("intent-{}", intent.pr_id));
        assert_eq!(env.campaign.as_deref(), Some(DOGFOOD_CAMPAIGN));

        // Metrics are MEASURED: the one real tool call (the memoized check)
        // is counted, with its breakdown; the lifespan is ordered.
        assert_eq!(env.metrics.tool_calls, 1);
        assert_eq!(env.metrics.tool_breakdown.len(), 1);
        assert_eq!(env.metrics.tool_breakdown[0].tool, "check.run_memoized");
        assert!(env.authorship.spawn.died_at >= env.authorship.spawn.born_at);
        // Honest zeros: no model was spawned in the hermetic loop.
        assert_eq!(env.metrics.tokens.total, 0);
        assert_eq!(env.metrics.model_turns, 0);

        // The trajectory blobs resolve and record what ACTUALLY happened:
        // a cold AC means the first run is a miss → executed once.
        let raw_ref = env
            .trajectory
            .raw_transcript_ref
            .as_ref()
            .expect("full capture (the ratified default) stores raw");
        let raw =
            String::from_utf8(store.get(raw_ref).expect("get").expect("present")).expect("utf-8");
        assert!(
            raw.contains("AC miss → executed once, stored"),
            "the raw track must record the real first-run provenance; got: {raw}"
        );
        assert!(raw.contains(&format!("brief: land {}", intent.pr_id)));
        // Task track is the mid-altitude subset.
        let task_ref = env.trajectory.task_transcript_ref.as_ref().expect("task");
        let task =
            String::from_utf8(store.get(task_ref).expect("get").expect("present")).expect("utf-8");
        assert!(task.contains("handoff:"));
        assert!(
            !task.contains("check.run_memoized("),
            "tool-call events are raw-track only"
        );
        // The inline summary digests the same reality.
        let summary = env.trajectory.summary.as_ref().expect("summary");
        assert!(summary.contains("exit 0"));
        // The snapshot pins the real scoped tree the check was keyed on.
        assert!(!env.tree_hash.is_empty());
        assert_eq!(env.snapshot.files_read.len(), 1);
        assert!(env.snapshot.files_read[0].hash.starts_with("sha256:"));
    }

    // ── orchestrator session per landed PR: dedup + waste ────────────────────
    assert_eq!(captured.pr_sessions.len(), 5, "one emission per landed PR");
    let first = &captured.pr_sessions[0];
    for (i, session) in captured.pr_sessions.iter().enumerate() {
        assert_eq!(session.envelope.altitude, Altitude::Pr);
        assert_eq!(session.envelope.intent_id, captured.report.landed[i]);
        // ONE session authored all five PRs: every emission refs the SAME
        // session transcript blobs (CAS dedupes), own envelope per PR.
        assert_eq!(
            session.envelope.trajectory.raw_transcript_ref,
            first.envelope.trajectory.raw_transcript_ref
        );
        assert_eq!(
            session.envelope.trajectory.task_transcript_ref,
            first.envelope.trajectory.task_transcript_ref
        );
        // Coordination metrics: the wave run was one recorded tool call.
        assert_eq!(session.orchestration.tool_calls, 1);
        // Green wave: nothing discarded, waste honestly zero.
        assert_eq!(session.waste.discarded_intents, 0);
        assert_eq!(session.waste.tokens_not_landed, 0);
        // The emitted ref resolves through the frozen type.
        let env = fetch_envelope(&store, &session.envelope_ref);
        assert_eq!(env, session.envelope);
    }
    let distinct_envelope_refs: std::collections::BTreeSet<&String> = captured
        .pr_sessions
        .iter()
        .map(|s| &s.envelope_ref)
        .collect();
    assert_eq!(
        distinct_envelope_refs.len(),
        5,
        "each PR carries its own envelope (own unit id) over the shared blobs"
    );

    // The session's raw track records the REAL landing outcome.
    let session_raw_ref = first
        .envelope
        .trajectory
        .raw_transcript_ref
        .as_ref()
        .expect("session raw");
    let session_raw = String::from_utf8(store.get(session_raw_ref).expect("get").expect("present"))
        .expect("utf-8");
    assert!(session_raw.contains("queue.run_wave"));
    assert!(session_raw.contains("landed 5 of 5"));

    // ── session altitude: the physical HOME (WP-F2b) ─────────────────────────
    let session = &captured.session;
    assert_eq!(session.envelope.altitude, Altitude::Session);
    assert_eq!(session.envelope.intent_id, "orq-dogfood-wave");
    // Under `full` the session home carries BOTH transcript refs (the
    // two-transcript imperative).
    let home_raw = session
        .envelope
        .trajectory
        .raw_transcript_ref
        .as_ref()
        .expect("session home raw");
    assert!(session.envelope.trajectory.task_transcript_ref.is_some());
    // Every PR envelope references the SAME blobs the session homes.
    for pr in &captured.pr_sessions {
        assert_eq!(
            pr.envelope.trajectory.raw_transcript_ref.as_ref(),
            Some(home_raw),
            "the PR envelope references the session-home raw blob"
        );
    }

    // ── campaign altitude: same path, same home blobs ────────────────────────
    let campaign = &captured.campaign_session;
    assert_eq!(campaign.envelope.altitude, Altitude::Campaign);
    assert_eq!(campaign.envelope.intent_id, DOGFOOD_CAMPAIGN);
    assert_eq!(
        campaign.envelope.trajectory.raw_transcript_ref.as_ref(),
        Some(home_raw),
        "the campaign envelope refs the same captured session-home blob"
    );
    let env = fetch_envelope(&store, &campaign.envelope_ref);
    assert_eq!(env, campaign.envelope);

    // ── the four-altitude family, one wave, behind real context_refs ─────────
    // (Altitude has no Ord; collect the debug labels into a sorted set.)
    let altitudes: std::collections::BTreeSet<String> =
        std::iter::once(captured.intents[0].closed.envelope.altitude)
            .chain(std::iter::once(captured.session.envelope.altitude))
            .chain(captured.pr_sessions.iter().map(|p| p.envelope.altitude))
            .chain(std::iter::once(captured.campaign_session.envelope.altitude))
            .map(|a| format!("{a:?}"))
            .collect();
    assert_eq!(
        altitudes,
        ["Campaign", "Intent", "Pr", "Session"]
            .into_iter()
            .map(String::from)
            .collect(),
        "the full four-altitude family is proven on one real wave"
    );
}

#[test]
fn repeat_wave_records_hit_provenance_in_the_trajectory() {
    // Shared AC across two captured waves: the second wave's intents must
    // RECORD the memoization wedge (hit, zero local execution) in their
    // captured trajectories — provenance measured, not asserted.
    let cfg = WaveConfig::five_pr_disjoint_green();
    let ac = InMemoryAc::new();
    let store = InMemoryColdStore::new();

    let _first = run_wave_with_envelope_capture(&cfg, &ac, &store, CaptureLevel::default());
    let second = run_wave_with_envelope_capture(&cfg, &ac, &store, CaptureLevel::default());

    assert_eq!(
        second.report.local_executions, 0,
        "warm AC: the second wave executes nothing locally"
    );
    for intent in &second.intents {
        let raw_ref = intent
            .closed
            .envelope
            .trajectory
            .raw_transcript_ref
            .as_ref()
            .expect("raw");
        let raw =
            String::from_utf8(store.get(raw_ref).expect("get").expect("present")).expect("utf-8");
        assert!(
            raw.contains("AC hit (zero local execution)"),
            "the warm-run trajectory must record the hit; got: {raw}"
        );
    }
}

#[test]
fn failing_pair_wave_shows_waste_not_hidden() {
    // A wave with a failing pair: the two excluded PRs surface as DISCARDED
    // intents in the orchestrator's waste emission — shown, not hidden.
    let cfg = WaveConfig::five_pr_with_failing_pair("pr-1", "pr-3");
    let ac = InMemoryAc::new();
    let store = InMemoryColdStore::new();

    let captured = run_wave_with_envelope_capture(&cfg, &ac, &store, CaptureLevel::default());

    assert_eq!(captured.report.excluded.len(), 2);
    assert_eq!(captured.pr_sessions.len(), captured.report.landed.len());
    for session in &captured.pr_sessions {
        assert_eq!(session.waste.discarded_intents, 2);
    }
    assert_eq!(captured.campaign_session.waste.discarded_intents, 2);
}

#[test]
fn capture_level_opt_down_gates_the_dogfood_bundle_too() {
    // The per-repo privacy opt-DOWN holds on the dogfood path: at `metrics`
    // no transcript blob is written, refs are null, metrics survive.
    let cfg = WaveConfig::five_pr_disjoint_green();
    let ac = InMemoryAc::new();
    let store = InMemoryColdStore::new();

    let captured = run_wave_with_envelope_capture(&cfg, &ac, &store, CaptureLevel::Metrics);

    for intent in &captured.intents {
        let env = &intent.closed.envelope;
        assert_eq!(env.trajectory.raw_transcript_ref, None);
        assert_eq!(env.trajectory.task_transcript_ref, None);
        assert_eq!(env.trajectory.summary, None);
        assert_eq!(env.snapshot.prompt_ref, None);
        assert_eq!(env.metrics.tool_calls, 1, "metrics still captured");
        // Null-refs round-trip through the frozen shape.
        let back = fetch_envelope(&store, &intent.closed.context_ref);
        assert_eq!(&back, env);
    }
    // Only envelope blobs at rest: 5 intents + 1 session + 5 PR sessions +
    // 1 campaign = 12. No transcript blobs below `task`.
    assert_eq!(store.len(), 12, "no transcript blobs below `task`");
}
