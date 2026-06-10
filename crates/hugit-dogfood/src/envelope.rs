//! WP-F2 dogfood leg — envelope capture around REAL spawned intents.
//!
//! Extends the item-① wave (`wave.rs`) with the ADR-0001 producer
//! ([`hugit_runner::envelope`]): each PR's intent does its REAL work (the
//! memoized check through the actual `hugit-checks` client against the
//! shared `InMemoryAc`) under a [`TrajectoryRecorder`] — so the captured
//! metrics are **measured** (wall/active time, tool-call counts, hit/miss
//! provenance), never fabricated — and is closed into a frozen
//! `ContextEnvelope` behind a `context_ref`, wired into its
//! [`IntentSidecar`]. The orchestrator (the wave run itself) then emits the
//! full FOUR-altitude family through the identical path: its OWN
//! `altitude:"session"` envelope (the physical HOME of the session's raw +
//! task blobs), one `altitude:"pr"` envelope per landed PR, and one
//! `altitude:"campaign"` envelope — the PR/campaign envelopes referencing the
//! SAME deduped blobs the session homes (WP-F2b) — plus **waste**.
//!
//! Token figures are zero by honesty: the deterministic in-process harness
//! spawns no model, so zero is the measured spend (the token/cache-split
//! capture itself is proven in `hugit-runner`'s WP-F2 acceptance with
//! explicit recorder feeds).
//!
//! Note the realistic warm-up: the intents run their checks BEFORE the
//! landing wave evaluates unions over the same memo keys, so the wave's
//! union legs are AC hits — the same shape the production path has (agents
//! check first, the queue re-proves).

use std::collections::HashMap;
use std::time::Instant;

use hugit_checks::client::{ac::InMemoryAc, executor::run_memoized, memo_key::FileContent};
use hugit_contracts::context_envelope::{Authorship, Spawn, WasteCost};
use hugit_contracts::{Altitude, IntentSidecar};
use hugit_runner::envelope::{
    CaptureLevel, ClosedEnvelope, ColdBlobStore, EnvelopeDraft, SessionEmission,
    TrajectoryRecorder, close_envelope, close_session_envelope, cold_ref_for,
};

use crate::wave::{DeterministicRunner, WaveConfig, WaveReport, run_wave_with_ac};

/// The campaign key the dogfood wave runs under (the landing-queue bundle
/// key all its PRs share).
pub const DOGFOOD_CAMPAIGN: &str = "dogfood-wave";

/// One captured intent: the frozen envelope behind its `context_ref`, wired
/// into the (non-authoritative) [`IntentSidecar`].
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedIntent {
    /// The PR this intent lands through (dogfood: one intent per PR).
    pub pr_id: String,
    /// The PR-attached sidecar with `context_ref` → the stored envelope.
    pub sidecar: IntentSidecar,
    /// The closed envelope + its content-addressed ref.
    pub closed: ClosedEnvelope,
}

/// A wave run with full WP-F2/F2b capture across the FOUR altitudes
/// (`session` · `pr` · `campaign` · `intent`).
/// (No `Debug` derive: [`WaveReport`] carries the non-`Debug` `EventLog`.)
pub struct CapturedWave {
    /// The item-① wave report (real Phase-B engine).
    pub report: WaveReport,
    /// Per-intent emissions (one per wave entry, landed or not — waste is
    /// shown, not hidden).
    pub intents: Vec<CapturedIntent>,
    /// The orchestrator-session's OWN envelope (`altitude:"session"`) — the
    /// physical HOME of the session's raw + task transcript blobs. The PR and
    /// campaign emissions reference the same deduped blobs (WP-F2b).
    pub session: SessionEmission,
    /// The orchestrator-session emission per LANDED PR (`altitude:"pr"`).
    /// One session authored them all: the transcript blobs are CAS-deduped
    /// against the session home above.
    pub pr_sessions: Vec<SessionEmission>,
    /// The campaign-altitude emission — same path, same session home blobs.
    pub campaign_session: SessionEmission,
}

/// Authorship for a dogfood run: the deterministic in-process harness (no
/// model is spawned — the honest provenance of this hermetic agent).
fn dogfood_authorship(
    run_id: &str,
    parent_run_id: Option<&str>,
    agent_type: &str,
    born_at: u64,
    died_at: u64,
) -> Authorship {
    Authorship {
        model: "in-process-deterministic".to_string(),
        model_digest: "dogfood-harness-v0".to_string(),
        agent_type: agent_type.to_string(),
        spawn: Spawn {
            run_id: run_id.to_string(),
            parent_run_id: parent_run_id.map(str::to_string),
            born_at,
            died_at,
        },
        operator: "dogfood-harness@hugit".to_string(),
    }
}

/// Run the wave with envelope capture: spawn each intent's real work under
/// a recorder, close its envelope at `level` into `store`, then land the
/// wave and emit the orchestrator-session envelopes (per landed PR +
/// campaign) with waste.
///
/// # Panics
/// Panics if a cold-store write fails or a check refuses to run — the
/// dogfood harness treats infrastructure failure as a test failure (same
/// posture as `run_wave`).
pub fn run_wave_with_envelope_capture<S: ColdBlobStore>(
    cfg: &WaveConfig,
    ac: &InMemoryAc,
    store: &S,
    level: CaptureLevel,
) -> CapturedWave {
    // ── per-intent: real work under a recorder, closed at intent close ───────
    let mut intents = Vec::with_capacity(cfg.entries.len());
    for (pr_id, _) in &cfg.entries {
        // The same per-PR file map the wave uses (one file, content = pr id).
        let path = format!("src/{pr_id}.rs");
        let content: FileContent = pr_id.as_bytes().to_vec();
        let mut files = HashMap::new();
        files.insert(path.clone(), content.clone());

        let mut rec = TrajectoryRecorder::start();
        let born_at = unix_ms_now();
        rec.record_task_event(format!(
            "brief: land {pr_id} — run `{}` memoized and hand off green",
            cfg.check_def.command
        ));

        // The REAL work: the memoized check through the actual checks client.
        let t0 = Instant::now();
        let runner = DeterministicRunner {
            pr_id: pr_id.clone(),
        };
        let file_iter: Vec<(&str, &FileContent)> =
            files.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let outcome = run_memoized(
            ac,
            &runner,
            &cfg.check_def,
            file_iter,
            &cfg.toolchain_digest,
        )
        .expect("dogfood check must run");
        let busy = t0.elapsed();
        let provenance = if outcome.from_cache {
            "AC hit (zero local execution)"
        } else {
            "AC miss → executed once, stored"
        };
        rec.record_tool_call(
            "check.run_memoized",
            format!(
                "check.run_memoized({}) -> exit {} via {provenance}; memo_key={}",
                cfg.check_def.command, outcome.result.exit, outcome.result.memo_key
            ),
            busy,
        );
        rec.record_task_event(format!(
            "handoff: {pr_id} check {} ({provenance})",
            if outcome.result.exit == 0 {
                "green"
            } else {
                "red"
            }
        ));

        let t = rec.finish(0.0);
        let summary = format!(
            "Ran the memoized check for {pr_id}: exit {}, {provenance}; \
             1 tool call, {} ms wall.",
            outcome.result.exit, t.metrics.wall_ms
        );

        let draft = EnvelopeDraft {
            altitude: Altitude::Intent,
            unit_id: format!("intent-{pr_id}"),
            // Dogfood drives PRs without a git repo; the synthetic commit id
            // is labelled as such (fixture world — honest shape, no fake sha).
            commit: format!("dogfood:{pr_id}"),
            // The REAL scoped tree root the check was keyed on.
            tree_hash: outcome.result.tree_hash.clone(),
            authorship: dogfood_authorship(
                &format!("run-{pr_id}"),
                Some("orq-dogfood-wave"),
                "implementer",
                born_at,
                t.died_at,
            ),
            charter: format!("land {pr_id} with green checks"),
            campaign: Some(DOGFOOD_CAMPAIGN.to_string()),
            constraints: vec![],
            acceptance: vec!["union verdict green".to_string()],
            parent_intents: vec![],
            raw_transcript: t.raw_transcript,
            task_transcript: t.task_transcript,
            summary,
            journal_ref: None,
            files_read: vec![hugit_contracts::context_envelope::FileRead {
                path,
                // Content-pin of the file as read (sha256 of the real bytes).
                hash: cold_ref_for(&content).replace("cas:", "sha256:"),
            }],
            prompt: Some(format!("charter: land {pr_id} with green checks")),
            env_manifest: cfg.toolchain_digest.clone(),
            metrics: t.metrics,
            verdicts_ref: None,
        };
        let closed = close_envelope(&draft, level, store).expect("intent envelope close");
        let sidecar = IntentSidecar {
            intent_id: draft.unit_id.clone(),
            charter: draft.charter.clone(),
            acceptance: draft.acceptance.clone(),
            context_ref: closed.context_ref.clone(),
            authoritative: false,
        };
        intents.push(CapturedIntent {
            pr_id: pr_id.clone(),
            sidecar,
            closed,
        });
    }

    // ── the orchestrator session: plan → land the wave, recorded ─────────────
    let mut rec = TrajectoryRecorder::start();
    let session_born = unix_ms_now();
    rec.record_task_event(format!(
        "plan: wave of {} PRs, campaign {DOGFOOD_CAMPAIGN}",
        cfg.entries.len()
    ));
    let t0 = Instant::now();
    let report = run_wave_with_ac(cfg, ac);
    let busy = t0.elapsed();
    rec.record_tool_call(
        "queue.run_wave",
        format!(
            "queue.run_wave -> landed {:?}, excluded {:?}, {} local executions",
            report.landed, report.excluded, report.local_executions
        ),
        busy,
    );
    rec.record_task_event(format!(
        "landed {} of {} PRs in queue order",
        report.landed.len(),
        cfg.entries.len()
    ));
    let t = rec.finish(0.0);
    let session_summary = format!(
        "Orchestrated a {}-PR wave: landed {}, excluded {}; {} local check executions.",
        cfg.entries.len(),
        report.landed.len(),
        report.excluded.len(),
        report.local_executions
    );

    // Waste — shown, not hidden: intents that did not land, and their spend.
    let tokens_not_landed: u64 = intents
        .iter()
        .filter(|i| report.excluded.contains(&i.pr_id))
        .map(|i| i.closed.envelope.metrics.tokens.total)
        .sum();
    let waste = WasteCost {
        discarded_intents: report.excluded.len() as u64,
        retried_agents: 0,
        tokens_not_landed,
        cost_usd: 0.0,
    };

    // One session authored every landed PR: close the SAME session draft once
    // per PR (each carries its own unit id; the transcript blobs CAS-dedupe).
    let session_draft = |unit_id: String, altitude: Altitude| EnvelopeDraft {
        altitude,
        unit_id,
        commit: "dogfood:wave-head".to_string(),
        tree_hash: "dogfood:wave-union".to_string(),
        authorship: dogfood_authorship("orq-dogfood-wave", None, "main", session_born, t.died_at),
        charter: "land the dogfood wave".to_string(),
        campaign: Some(DOGFOOD_CAMPAIGN.to_string()),
        constraints: vec![],
        acceptance: vec!["0 wrong merges, 0 lost PRs".to_string()],
        parent_intents: vec![],
        raw_transcript: t.raw_transcript.clone(),
        task_transcript: t.task_transcript.clone(),
        summary: session_summary.clone(),
        journal_ref: None,
        files_read: vec![],
        prompt: Some("orchestrate: land the dogfood wave".to_string()),
        env_manifest: cfg.toolchain_digest.clone(),
        metrics: t.metrics.clone(),
        verdicts_ref: None,
    };

    // The session's OWN envelope (`altitude:"session"`) — the physical HOME
    // of the raw + task blobs. Emitted FIRST so the home is unambiguous; the
    // PR and campaign envelopes below reference the SAME content (CAS dedupes
    // identical bytes to one stored blob, one ref).
    let session = close_session_envelope(
        &session_draft("orq-dogfood-wave".to_string(), Altitude::Session),
        level,
        store,
        waste.clone(),
    )
    .expect("session envelope close");

    let pr_sessions: Vec<SessionEmission> = report
        .landed
        .iter()
        .map(|pr_id| {
            close_session_envelope(
                &session_draft(pr_id.clone(), Altitude::Pr),
                level,
                store,
                waste.clone(),
            )
            .expect("pr session envelope close")
        })
        .collect();

    // Campaign altitude: the same path, the campaign's own envelope.
    let campaign_session = close_session_envelope(
        &session_draft(DOGFOOD_CAMPAIGN.to_string(), Altitude::Campaign),
        level,
        store,
        waste,
    )
    .expect("campaign session envelope close");

    CapturedWave {
        report,
        intents,
        session,
        pr_sessions,
        campaign_session,
    }
}

/// Current unix time in ms (the recorder owns the authoritative lifespan;
/// this anchors `Spawn.born_at` to the same clock).
fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
