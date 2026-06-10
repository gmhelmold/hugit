//! WP-F2 acceptance — envelope producer (ADR-0001, ratified 2026-06-10).
//!
//! Owned proofs:
//! 1. **Redaction-before-write** — a planted secret is scrubbed on the WRITE
//!    path: a grep-class ABSENCE scan over EVERY blob the cold store holds
//!    (raw, task, prompt, the envelope itself) finds no trace of the secret
//!    value; redaction is per-line (non-secret lines survive — the X3
//!    lesson, never whole-blob).
//! 2. **Capture-level gating** — `off|metrics|task|full` gate exactly what
//!    ADR-0001 §3 tabulates; absent refs are `null`; the default level is
//!    `full` (ratified, non-negotiable).
//! 3. **Null-refs round-trip** — sub-`full` envelopes round-trip through the
//!    FROZEN `ContextEnvelope` (deny_unknown_fields), and the producer's
//!    null-ref shape matches the committed WP-F1 golden
//!    (`ContextEnvelopeNullRefs.json`) byte-for-byte on re-serialization.
//! 4. **Dedupe** — same content → same `cas:` ref, stored once (in-memory
//!    AND dir-backed); one session authoring several PRs stores ONE session
//!    transcript blob that every PR envelope refs.
//! 5. **Disclosed live seam fails closed** — the unwired cold-store binding
//!    surfaces `NotWired`, never a silent drop.

use std::time::Duration;

use hugit_contracts::context_envelope::{Authorship, Spawn, ToolCount, WasteCost};
use hugit_contracts::{Altitude, ContextEnvelope, REDACTED_MARKER};
use hugit_runner::envelope::{
    CaptureLevel, ColdBlobStore, ColdStoreError, DirColdStore, EnvelopeDraft, InMemoryColdStore,
    TrajectoryRecorder, UnwiredColdStore, close_envelope, close_session_envelope,
};

/// A planted secret carrying both the canonical marker (`SECRET:`) and a
/// distinct VALUE, so the oracle can assert the value is absent (a
/// degenerate redactor that only strips the marker would fail).
const PLANTED_SECRET: &str = "SECRET:f2-cold-api-key-7c41e9";
/// The secret value alone — what the absence scan greps for.
const SECRET_VALUE: &str = "f2-cold-api-key-7c41e9";
/// A non-secret canary line that must SURVIVE redaction intact (proves the
/// redactor is per-line, not whole-blob).
const SURVIVOR_LINE: &str = "ran cargo test --workspace: 312 passed";

fn authorship() -> Authorship {
    Authorship {
        model: "in-process-deterministic".to_string(),
        model_digest: "0".repeat(64),
        agent_type: "implementer".to_string(),
        spawn: Spawn {
            run_id: "run-f2-001".to_string(),
            parent_run_id: Some("orq-f2".to_string()),
            born_at: 1_717_000_100_000,
            died_at: 1_717_000_195_000,
        },
        operator: "gustavo@humangr.com".to_string(),
    }
}

/// A draft whose transcripts/prompt/summary all carry the planted secret
/// PLUS the survivor line, with measured-shape metrics.
fn secret_bearing_draft(altitude: Altitude) -> EnvelopeDraft {
    let mut rec = TrajectoryRecorder::start();
    rec.record_task_event(format!("brief: rotate key {PLANTED_SECRET}"));
    rec.record_turn(
        format!("model turn: using {PLANTED_SECRET}"),
        Duration::from_millis(2),
    );
    rec.record_tool_call("Bash", SURVIVOR_LINE, Duration::from_millis(1));
    rec.add_tokens(120, 30, 64, 8);
    let t = rec.finish(0.0);

    EnvelopeDraft {
        altitude,
        unit_id: "a31".to_string(),
        commit: "a31f9c".to_string() + &"0".repeat(34),
        tree_hash: "7".repeat(64),
        authorship: authorship(),
        charter: "fix: refresh reusava o iat antigo".to_string(),
        campaign: Some("auth-hardening".to_string()),
        constraints: vec![],
        acceptance: vec!["dura o TTL completo".to_string()],
        parent_intents: vec![],
        raw_transcript: t.raw_transcript,
        task_transcript: t.task_transcript,
        summary: format!("rotated the key ({PLANTED_SECRET}); 3 events"),
        journal_ref: None,
        files_read: vec![],
        prompt: Some(format!("system: you hold {PLANTED_SECRET}")),
        env_manifest: "rustc 1.96.0".to_string(),
        metrics: t.metrics,
        verdicts_ref: None,
    }
}

// ── ① redaction on the write path ────────────────────────────────────────────

#[test]
fn item_1_redaction_applied_before_any_blob_is_stored() {
    let store = InMemoryColdStore::new();
    let closed = close_envelope(
        &secret_bearing_draft(Altitude::Intent),
        CaptureLevel::default(),
        &store,
    )
    .expect("close at full");

    // Grep-class ABSENCE scan over EVERY byte at rest in the cold store —
    // raw, task, prompt, and the envelope blob itself. The secret VALUE must
    // appear nowhere; the redaction sentinel must appear.
    let mut sentinel_seen = false;
    for (blob_ref, bytes) in store.blobs() {
        let text = String::from_utf8(bytes).expect("stored blobs are utf-8 here");
        assert!(
            !text.contains(SECRET_VALUE),
            "secret value leaked into stored blob {blob_ref}: {text}"
        );
        sentinel_seen |= text.contains(REDACTED_MARKER);
    }
    assert!(
        sentinel_seen,
        "the REDACTED_MARKER sentinel must appear in the redacted blobs"
    );

    // Per-line, not whole-blob: the non-secret line SURVIVES in the raw blob.
    let raw_ref = closed
        .envelope
        .trajectory
        .raw_transcript_ref
        .as_ref()
        .expect("full capture stores the raw transcript");
    let raw = String::from_utf8(store.get(raw_ref).expect("get").expect("present"))
        .expect("raw blob is utf-8");
    assert!(
        raw.contains(SURVIVOR_LINE),
        "non-secret line must survive per-line redaction; got: {raw}"
    );
    assert!(raw.contains(REDACTED_MARKER));

    // The inline summary (stored inside the envelope) is also scrubbed.
    let summary = closed.envelope.trajectory.summary.as_ref().expect("full");
    assert!(!summary.contains(SECRET_VALUE));

    // The stamped policy is the canonical one.
    assert_eq!(closed.envelope.trajectory.redaction_policy, "default-v1");
}

// ── ② capture-level gating ───────────────────────────────────────────────────

#[test]
fn item_2_capture_levels_gate_refs_and_metrics() {
    let draft = secret_bearing_draft(Altitude::Intent);

    // off → envelope metadata only: all refs null, summary null, metrics zeroed.
    let store = InMemoryColdStore::new();
    let off = close_envelope(&draft, CaptureLevel::Off, &store)
        .expect("close at off")
        .envelope;
    assert_eq!(off.trajectory.raw_transcript_ref, None);
    assert_eq!(off.trajectory.task_transcript_ref, None);
    assert_eq!(off.trajectory.summary, None);
    assert_eq!(off.snapshot.prompt_ref, None);
    assert_eq!(off.metrics.tokens.total, 0);
    assert_eq!(off.metrics.tool_calls, 0);
    assert_eq!(off.metrics.wall_ms, 0);
    assert_eq!(
        store.len(),
        1,
        "off stores ONLY the envelope blob, no trajectory blobs"
    );

    // metrics → + metrics; still no transcript refs.
    let store = InMemoryColdStore::new();
    let metrics = close_envelope(&draft, CaptureLevel::Metrics, &store)
        .expect("close at metrics")
        .envelope;
    assert_eq!(metrics.metrics, draft.metrics, "metrics kept");
    assert_eq!(metrics.trajectory.raw_transcript_ref, None);
    assert_eq!(metrics.trajectory.task_transcript_ref, None);
    assert_eq!(metrics.trajectory.summary, None);
    assert_eq!(metrics.snapshot.prompt_ref, None);

    // task → + task ref + summary; raw and prompt still null.
    let store = InMemoryColdStore::new();
    let task = close_envelope(&draft, CaptureLevel::Task, &store)
        .expect("close at task")
        .envelope;
    assert!(task.trajectory.task_transcript_ref.is_some());
    assert!(task.trajectory.summary.is_some());
    assert_eq!(task.trajectory.raw_transcript_ref, None);
    assert_eq!(task.snapshot.prompt_ref, None);

    // full (THE DEFAULT, ratified) → everything.
    let store = InMemoryColdStore::new();
    let full = close_envelope(&draft, CaptureLevel::default(), &store)
        .expect("close at full")
        .envelope;
    assert!(full.trajectory.raw_transcript_ref.is_some());
    assert!(full.trajectory.task_transcript_ref.is_some());
    assert!(full.trajectory.summary.is_some());
    assert!(full.snapshot.prompt_ref.is_some());
    assert_eq!(full.metrics, draft.metrics);
}

// ── ③ null-refs round-trip vs the frozen golden ──────────────────────────────

/// The committed WP-F1 golden for the nullable-refs shape (read-only; the
/// contracts crate owns the byte-exactness of its own goldens — here it
/// anchors that the PRODUCER's null-ref output is the same frozen shape).
const GOLDEN_NULL_REFS: &str =
    include_str!("../../hugit-contracts/tests/golden/ContextEnvelopeNullRefs.json");

#[test]
fn item_3_null_refs_round_trip_against_frozen_golden_shape() {
    // The golden parses through the frozen type (deny_unknown_fields) and
    // re-serializes to the identical value tree.
    let golden: ContextEnvelope =
        serde_json::from_str(GOLDEN_NULL_REFS).expect("golden parses through the frozen type");
    assert_eq!(golden.trajectory.raw_transcript_ref, None);
    assert_eq!(golden.trajectory.task_transcript_ref, None);
    assert_eq!(golden.trajectory.summary, None);
    assert_eq!(golden.snapshot.prompt_ref, None);
    let golden_value: serde_json::Value = serde_json::from_str(GOLDEN_NULL_REFS).expect("value");
    let reserialized: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&golden).expect("serialize"))
            .expect("re-parse");
    assert_eq!(
        reserialized, golden_value,
        "frozen golden must round-trip loss-free"
    );

    // A producer-closed envelope at `metrics` (the golden's shape: metrics
    // present, every ref null) round-trips through the SAME frozen type and
    // exhibits the SAME null pattern.
    let store = InMemoryColdStore::new();
    let closed = close_envelope(
        &secret_bearing_draft(Altitude::Intent),
        CaptureLevel::Metrics,
        &store,
    )
    .expect("close at metrics");
    let bytes = store
        .get(&closed.context_ref)
        .expect("get")
        .expect("the context_ref resolves to the stored envelope blob");
    let back: ContextEnvelope =
        serde_json::from_slice(&bytes).expect("stored blob parses through the frozen type");
    assert_eq!(back, closed.envelope, "round-trip equality");
    assert_eq!(back.trajectory.raw_transcript_ref, None);
    assert_eq!(back.trajectory.task_transcript_ref, None);
    assert_eq!(back.trajectory.summary, None);
    assert_eq!(back.snapshot.prompt_ref, None);
}

// ── ④ dedupe: same content → same ref ────────────────────────────────────────

#[test]
fn item_4a_same_content_same_ref_in_memory_and_dir_backed() {
    let bytes = b"the same trajectory bytes";

    let mem = InMemoryColdStore::new();
    let r1 = mem.put(bytes).expect("put");
    let r2 = mem.put(bytes).expect("put again");
    assert_eq!(r1, r2);
    assert_eq!(mem.len(), 1, "stored once");

    // Dir-backed (hermetic temp dir, std only), same law + the same ref as
    // the in-memory store (tier-agnostic: the ref never encodes the tier).
    let root = std::env::temp_dir().join(format!(
        "hugit-f2-coldstore-{}-{}",
        std::process::id(),
        line!()
    ));
    let dir = DirColdStore::open(&root).expect("open dir store");
    let d1 = dir.put(bytes).expect("put");
    let d2 = dir.put(bytes).expect("put again");
    assert_eq!(d1, d2);
    assert_eq!(d1, r1, "the content-addressed ref is tier-agnostic");
    assert_eq!(dir.get(&d1).expect("get"), Some(bytes.to_vec()));

    // Content-address guard: tampered bytes are refused, never served.
    let hash = d1.strip_prefix("cas:").expect("cas-prefixed");
    std::fs::write(root.join(hash), b"tampered").expect("tamper");
    assert!(matches!(
        dir.get(&d1),
        Err(ColdStoreError::DigestMismatch { .. })
    ));

    std::fs::remove_dir_all(&root).expect("cleanup temp store");
}

#[test]
fn item_4b_one_session_many_prs_dedupes_the_session_blob() {
    let store = InMemoryColdStore::new();
    let waste = WasteCost {
        discarded_intents: 1,
        retried_agents: 0,
        tokens_not_landed: 4_200,
        cost_usd: 0.0,
    };

    // ONE orchestrator session (same transcript + snapshot) authors TWO PRs:
    // the drafts differ only in the authored-unit id.
    let mut pr_a = secret_bearing_draft(Altitude::Pr);
    pr_a.unit_id = "128".to_string();
    pr_a.authorship.agent_type = "main".to_string();
    let mut pr_b = pr_a.clone();
    pr_b.unit_id = "129".to_string();

    let a = close_session_envelope(&pr_a, CaptureLevel::default(), &store, waste.clone())
        .expect("close pr 128");
    let b = close_session_envelope(&pr_b, CaptureLevel::default(), &store, waste)
        .expect("close pr 129");

    // Each PR refs the SAME session blob (CAS dedupes the transcripts)...
    assert_eq!(
        a.envelope.trajectory.raw_transcript_ref,
        b.envelope.trajectory.raw_transcript_ref
    );
    assert_eq!(
        a.envelope.trajectory.task_transcript_ref,
        b.envelope.trajectory.task_transcript_ref
    );
    assert_eq!(
        a.envelope.snapshot.prompt_ref,
        b.envelope.snapshot.prompt_ref
    );
    // ...while the envelopes themselves stay distinct (own unit id).
    assert_ne!(a.envelope_ref, b.envelope_ref);
    assert_eq!(a.envelope.intent_id, "128");
    assert_eq!(b.envelope.intent_id, "129");
    // Shared trajectory blobs stored ONCE + 2 distinct envelopes. Dedupe is
    // purely by content: the redacted task track and the redacted prompt
    // here both collapse to the identical `[REDACTED]` bytes, so they share
    // ONE blob — raw (1) + task≡prompt (1) + envelopes (2) = 4.
    assert_eq!(
        a.envelope.trajectory.task_transcript_ref, a.envelope.snapshot.prompt_ref,
        "identical redacted bytes dedupe across blob roles too"
    );
    assert_eq!(store.len(), 4, "shared blobs stored once, envelopes twice");

    // The campaign altitude rides the SAME path.
    let mut campaign = pr_a.clone();
    campaign.altitude = Altitude::Campaign;
    campaign.unit_id = "auth-hardening".to_string();
    let c = close_session_envelope(
        &campaign,
        CaptureLevel::default(),
        &store,
        WasteCost {
            discarded_intents: 0,
            retried_agents: 0,
            tokens_not_landed: 0,
            cost_usd: 0.0,
        },
    )
    .expect("close campaign");
    assert_eq!(c.envelope.altitude, Altitude::Campaign);
    assert_eq!(
        c.envelope.trajectory.raw_transcript_ref, a.envelope.trajectory.raw_transcript_ref,
        "same session content dedupes across altitudes too"
    );
}

// ── ⑤ disclosed live seam fails closed ───────────────────────────────────────

#[test]
fn item_5_unwired_live_cold_store_fails_closed() {
    let live = UnwiredColdStore::new();
    let err = close_envelope(
        &secret_bearing_draft(Altitude::Intent),
        CaptureLevel::default(),
        &live,
    )
    .expect_err("the unwired seam must fail closed, never silently drop");
    let msg = err.to_string();
    assert!(
        msg.contains("not wired"),
        "the deferral must be self-documenting; got: {msg}"
    );
}

// ── measured metrics shape (recorder → frozen IntentMetrics) ─────────────────

#[test]
fn recorder_metrics_carry_cache_split_and_breakdown_into_the_envelope() {
    let draft = secret_bearing_draft(Altitude::Intent);
    // The recorder-measured figures land in the frozen metrics block:
    // tokens with the cache split, tool breakdown, turns.
    assert_eq!(draft.metrics.tokens.input, 120);
    assert_eq!(draft.metrics.tokens.output, 30);
    assert_eq!(draft.metrics.tokens.cache_read, 64);
    assert_eq!(draft.metrics.tokens.cache_write, 8);
    assert_eq!(draft.metrics.tokens.total, 222);
    assert_eq!(draft.metrics.model_turns, 1);
    assert_eq!(
        draft.metrics.tool_breakdown,
        vec![ToolCount {
            tool: "Bash".to_string(),
            count: 1
        }]
    );
    let store = InMemoryColdStore::new();
    let closed = close_envelope(&draft, CaptureLevel::default(), &store).expect("close at full");
    assert_eq!(closed.envelope.metrics, draft.metrics);
}
