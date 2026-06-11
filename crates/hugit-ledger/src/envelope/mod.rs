//! Envelope producer (WP-F2, ADR-0001) — metrics emission + 3-altitude
//! trajectory capture at intent close, over the [`cold_store`] seam.
//!
//! The runner/agent harness produces the frozen
//! [`ContextEnvelope`](hugit_contracts::ContextEnvelope) when an authored
//! unit closes:
//!
//! - **Metrics** ([`hugit_contracts::IntentMetrics`]): tokens with the cache
//!   split, `wall_ms`, `active_ms`, tool calls + per-tool breakdown, model
//!   turns, derived `cost_usd_micros` — measured by a [`TrajectoryRecorder`] over
//!   the agent's born → die loop.
//! - **The three trajectory altitudes** (ADR-0001 §2.1): the raw born → die
//!   transcript and the task-scoped transcript go to the **cold object
//!   store** (never the CoreLink hot CAS — see [`cold_store`]); the inline
//!   `summary` rides in the envelope.
//! - **Redaction on the WRITE path** (ADR-0001 §3/§5): every transcript
//!   line, the prompt, and the summary are scrubbed via the established X3
//!   redaction code path ([`crate::redact::apply`], the
//!   `REDACTED_MARKER` policy) **before any blob is stored** — per line,
//!   never whole-blob (the X3 lesson: a non-secret line survives).
//! - **Capture-level gating** (`off | metrics | task | full`): refs absent
//!   below the active level are `null`. The default is
//!   [`CaptureLevel::Full`] — RATIFIED, non-negotiable (owner 2026-06-10);
//!   the ladder survives only as a per-repo privacy opt-DOWN.
//! - The serialized envelope itself is stored content-addressed and its ref
//!   returned as `context_ref` — the blob behind
//!   [`hugit_contracts::IntentSidecar::context_ref`].
//!
//! **Four altitudes** (ADR-0001 §2, second owner directive 2026-06-10). The
//! same envelope shape closes at each; only `altitude` + the authored-unit
//! id change:
//!
//! - **`intent`** — a commit, subagent-authored; closed via
//!   [`close_envelope`].
//! - **`session`** — the agent session/run ITSELF, the fourth first-class
//!   altitude. The session envelope is the **physical home** of that
//!   session's raw + task transcript blobs. One session may author several
//!   PRs/campaigns; their envelopes REFERENCE the same deduped blobs.
//! - **`pr`** — a bundle of intents; the orchestrator-session record that
//!   planned/dispatched/landed the bundle.
//! - **`campaign`** — a bundle of PRs, human-owned.
//!
//! Session/PR/campaign altitudes ride the SAME path
//! ([`close_session_envelope`]): the author emits its own envelope —
//! transcript + snapshot + coordination metrics — plus **waste** (discarded
//! intents, retried agents, tokens-not-landed). Because the transcript blobs
//! are content-addressed, the relationship between a session and the PRs /
//! campaigns it authors is **explicit, not coincidental**: a PR or campaign
//! envelope's trajectory refs MAY EQUAL the home session envelope's refs —
//! the CAS dedupe proves identical content collapses to one stored blob, so
//! the session envelope is provably the physical home and the others
//! reference into it. These feed the derived `PrRecord.envelope_ref` /
//! `orchestration` / `waste` (computed by WP-F3, not here).
//!
//! **The two-transcript imperative** (ADR-0001 §2, second owner directive):
//! under [`CaptureLevel::Full`] (the ratified default) BOTH
//! `raw_transcript_ref` (full) and `task_transcript_ref` (compacted) MUST be
//! present at EVERY altitude — closing Full without both is a hard
//! [`EnvelopeError::TwoTranscriptViolation`], never a silent null. Null refs
//! for these two are legal ONLY under an explicit opt-DOWN
//! (`off`/`metrics`/`task`).
//!
//! Retention: forever, no TTL — by ratified design nothing here tags,
//! expires, or deletes a blob.

pub mod cold_store;

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hugit_contracts::context_envelope::{
    Authorship, FileRead, OrchestrationCost, Snapshot, TokenCounts, ToolCount, Trajectory,
    WasteCost,
};
use hugit_contracts::{Altitude, CONTEXT_ENVELOPE_SCHEMA_VERSION, ContextEnvelope, IntentMetrics};

pub use cold_store::{
    COLD_REF_PREFIX, ColdBlobStore, ColdStoreError, DirColdStore, GetOutcome, InMemoryColdStore,
    TOMBSTONE_MARKER, Tombstone, TombstoneRecord, UnwiredColdStore, cold_ref_for,
};

/// The redaction policy identifier stamped into
/// [`Trajectory::redaction_policy`] — the policy that scrubbed the blobs on
/// the write path (ADR-0001 §2.2; matches the frozen golden fixtures).
pub const REDACTION_POLICY_DEFAULT: &str = "default-v1";

// ─────────────────────────────────────────────────────────────────────────────
// Capture level
// ─────────────────────────────────────────────────────────────────────────────

/// The per-repo capture level (ADR-0001 §3). **Default `full` at every
/// altitude — RATIFIED, non-negotiable** (owner 2026-06-10: "transcript 100%
/// tem que ser salvo sempre, inegociável"). The ladder below survives only
/// as a per-repo privacy opt-DOWN for customer tenants; refs absent below
/// the active level are `null` and consumers must tolerate nulls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum CaptureLevel {
    /// Envelope metadata only (metrics zeroed, all refs null).
    Off,
    /// \+ metrics (cost/time visibility, no transcript).
    Metrics,
    /// \+ `task_transcript_ref` + inline `summary` (mid privacy dial).
    Task,
    /// \+ `raw_transcript_ref` + `prompt_ref` — **the default**
    /// (forensics / replay).
    #[default]
    Full,
}

impl CaptureLevel {
    /// Parse the repo dial (`off | metrics | task | full`). Unknown values
    /// return `None` (fail-closed at the caller, never a silent default).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "metrics" => Some(Self::Metrics),
            "task" => Some(Self::Task),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// The canonical dial string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Metrics => "metrics",
            Self::Task => "task",
            Self::Full => "full",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Errors from the envelope producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// A blob write/read against the cold store failed.
    Store(ColdStoreError),
    /// [`close_session_envelope`] was called for the `intent` altitude —
    /// session envelopes are emitted at `session`/`pr`/`campaign` (an intent
    /// is closed via [`close_envelope`]; the waste/orchestration emission is
    /// a session-author concern, never a subagent's).
    NotASessionAltitude(Altitude),
    /// The **two-transcript imperative** was violated (ADR-0001 §2, second
    /// owner directive): under [`CaptureLevel::Full`] — the ratified default
    /// — EVERY altitude MUST carry BOTH `raw_transcript_ref` (full) and
    /// `task_transcript_ref` (compacted). A Full close that would leave
    /// either null is rejected hard, never written as a silent null. Carries
    /// the altitude and which of the two refs was missing. (Null refs remain
    /// legal under an explicit opt-DOWN: `off`/`metrics`/`task`.)
    TwoTranscriptViolation {
        /// The altitude being closed when the imperative was violated.
        altitude: Altitude,
        /// Which mandatory ref was absent (`raw_transcript_ref` /
        /// `task_transcript_ref` / both).
        missing: &'static str,
    },
    /// The [`ContextEnvelope`] could not be serialized to JSON before the
    /// `context_ref` store write. The envelope shape is frozen and all money
    /// fields are integers (WA4), so this path is unreachable in practice —
    /// but `serde_json::to_vec(...).expect(...)` is a panic class that dies
    /// on any unexpected non-serializable value (e.g. a future non-finite
    /// f64 sneaking in). This error variant replaces that panic with a
    /// fail-closed `Result` so the capture path never aborts the process.
    Serialize(String),
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvelopeError::Store(e) => write!(f, "envelope blob store failed: {e}"),
            EnvelopeError::NotASessionAltitude(a) => write!(
                f,
                "session close requires altitude session|pr|campaign, got {a:?} \
                 (intents close via close_envelope)"
            ),
            EnvelopeError::TwoTranscriptViolation { altitude, missing } => write!(
                f,
                "two-transcript imperative violated at altitude {altitude:?}: capture \
                 level is full (the ratified default) but {missing} is absent — full \
                 demands BOTH the full and the compacted transcript at every altitude; \
                 null refs are legal only under an explicit opt-down (off/metrics/task)"
            ),
            EnvelopeError::Serialize(msg) => write!(
                f,
                "envelope JSON serialization failed (fail-closed, capture aborted): {msg}"
            ),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl From<ColdStoreError> for EnvelopeError {
    fn from(e: ColdStoreError) -> Self {
        EnvelopeError::Store(e)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Trajectory recorder — measures the born → die loop
// ─────────────────────────────────────────────────────────────────────────────

/// Records an agent run's born → die loop so metrics are **measured, not
/// fabricated**: wall time from spawn to close, active (busy) time summed
/// over recorded turns/tool calls, per-tool call counts, model turns, and
/// token spend with the cache split as reported by the harness.
///
/// The raw track receives EVERY recorded event (the forensic altitude);
/// task-scoped events additionally land on the task track (raw ⊇ task).
#[derive(Debug)]
pub struct TrajectoryRecorder {
    /// Unix ms when the run was born.
    born_at: u64,
    /// Monotonic anchor for wall-time measurement.
    started: Instant,
    /// Accumulated busy (model + tool) time.
    active: Duration,
    /// The raw born → die track: every turn, tool call + result, task event.
    raw: Vec<String>,
    /// The task-scoped mid-altitude track.
    task: Vec<String>,
    /// Per-tool call counts (BTreeMap for a deterministic breakdown order).
    tool_counts: BTreeMap<String, u64>,
    /// Model turns observed.
    model_turns: u64,
    /// Token spend accumulated from harness reports.
    tokens: TokenCounts,
}

impl TrajectoryRecorder {
    /// Start recording — the agent is born now.
    #[must_use]
    pub fn start() -> Self {
        Self {
            born_at: unix_ms_now(),
            started: Instant::now(),
            active: Duration::ZERO,
            raw: Vec::new(),
            task: Vec::new(),
            tool_counts: BTreeMap::new(),
            model_turns: 0,
            tokens: TokenCounts {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                total: 0,
            },
        }
    }

    /// Record one model turn: the raw event line and the busy time it took.
    pub fn record_turn(&mut self, raw_event: impl Into<String>, busy: Duration) {
        self.model_turns += 1;
        self.active += busy;
        self.raw.push(raw_event.into());
    }

    /// Record one tool call (+ its result line) and the busy time it took.
    pub fn record_tool_call(&mut self, tool: &str, raw_event: impl Into<String>, busy: Duration) {
        *self.tool_counts.entry(tool.to_string()).or_insert(0) += 1;
        self.active += busy;
        self.raw.push(raw_event.into());
    }

    /// Record a task-scoped event (brief in, plan/step progression, key
    /// decision, result/handoff out). Lands on BOTH tracks — the raw track
    /// is a superset of the task track.
    pub fn record_task_event(&mut self, event: impl Into<String>) {
        let event = event.into();
        self.raw.push(event.clone());
        self.task.push(event);
    }

    /// Accumulate token spend as reported by the harness (input/output and
    /// the cache split). `total` is derived as the sum of all four.
    pub fn add_tokens(&mut self, input: u64, output: u64, cache_read: u64, cache_write: u64) {
        self.tokens.input += input;
        self.tokens.output += output;
        self.tokens.cache_read += cache_read;
        self.tokens.cache_write += cache_write;
        self.tokens.total += input + output + cache_read + cache_write;
    }

    /// Close the recording — the agent dies now. `cost_usd_micros` is the
    /// derived COGS in integer micro-USD (`1 USD = 1_000_000`) the harness
    /// computed for this run (shown for trust, never a usage meter; `0` is the
    /// honest figure for a model-free hermetic run). WA4 / schema 1.2.0.
    #[must_use]
    pub fn finish(self, cost_usd_micros: u64) -> RecordedTrajectory {
        let wall = self.started.elapsed();
        let died_at = self.born_at + u64::try_from(wall.as_millis()).unwrap_or(u64::MAX);
        let tool_calls = self.tool_counts.values().sum();
        RecordedTrajectory {
            born_at: self.born_at,
            died_at,
            raw_transcript: self.raw,
            task_transcript: self.task,
            metrics: IntentMetrics {
                tokens: self.tokens,
                wall_ms: u64::try_from(wall.as_millis()).unwrap_or(u64::MAX),
                active_ms: u64::try_from(self.active.as_millis()).unwrap_or(u64::MAX),
                tool_calls,
                tool_breakdown: self
                    .tool_counts
                    .into_iter()
                    .map(|(tool, count)| ToolCount { tool, count })
                    .collect(),
                model_turns: self.model_turns,
                cost_usd_micros,
            },
        }
    }
}

/// The measured output of a [`TrajectoryRecorder`]: lifespan, the two
/// transcript tracks (NOT yet redacted — redaction happens on the write
/// path in [`close_envelope`]), and the measured [`IntentMetrics`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedTrajectory {
    /// Unix ms — born.
    pub born_at: u64,
    /// Unix ms — died.
    pub died_at: u64,
    /// The raw born → die track.
    pub raw_transcript: Vec<String>,
    /// The task-scoped track.
    pub task_transcript: Vec<String>,
    /// The measured per-unit metrics.
    pub metrics: IntentMetrics,
}

/// Current unix time in ms.
fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// The draft → close producer
// ─────────────────────────────────────────────────────────────────────────────

/// Everything the harness knows at unit close, BEFORE gating and redaction.
/// [`close_envelope`] turns this into a frozen [`ContextEnvelope`] behind a
/// `context_ref` — it never stores a byte that has not passed redaction.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvelopeDraft {
    /// The altitude of the authored unit (intent | pr | campaign).
    pub altitude: Altitude,
    /// The authored-unit id at this altitude (intent id / PR id / campaign
    /// key — lands in `ContextEnvelope.intent_id` per the WP-F1 naming map).
    pub unit_id: String,
    /// Git commit the unit enriches.
    pub commit: String,
    /// Merkle tree hash of the unit's workspace snapshot.
    pub tree_hash: String,
    /// Provenance: who/what authored the unit.
    pub authorship: Authorship,
    /// The "why" — human-readable charter.
    pub charter: String,
    /// Campaign key; `None` when uncampaigned.
    pub campaign: Option<String>,
    /// Constraints the unit was authored under.
    pub constraints: Vec<String>,
    /// Acceptance criteria.
    pub acceptance: Vec<String>,
    /// Parent intent ids.
    pub parent_intents: Vec<String>,
    /// The raw born → die transcript (one event per line, unredacted).
    pub raw_transcript: Vec<String>,
    /// The task-scoped transcript (one event per line, unredacted).
    pub task_transcript: Vec<String>,
    /// The inline LLM digest for the drawer (unredacted).
    pub summary: String,
    /// `cas:` ref to the D11 ledger Journal (the human-annotation track),
    /// produced and owned by the ledger — passed through, never gated (it
    /// is not a trajectory blob this producer writes).
    pub journal_ref: Option<String>,
    /// Files the agent read, content-pinned.
    pub files_read: Vec<FileRead>,
    /// The system/charter prompt (unredacted); stored at `full` only.
    pub prompt: Option<String>,
    /// Environment manifest (toolchain pin).
    pub env_manifest: String,
    /// The measured per-unit metrics (from a [`TrajectoryRecorder`]).
    pub metrics: IntentMetrics,
    /// `cas:` ref to the adversarial panel verdicts; pass-through.
    pub verdicts_ref: Option<String>,
}

/// A closed, stored envelope: the frozen [`ContextEnvelope`] plus the
/// content-addressed ref of its serialized blob — the value that goes
/// behind [`hugit_contracts::IntentSidecar::context_ref`].
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedEnvelope {
    /// The envelope as stored (post-redaction, post-gating).
    pub envelope: ContextEnvelope,
    /// Content-addressed ref of the stored envelope blob.
    pub context_ref: String,
}

/// Redact a transcript on the write path: the established X3 rule
/// ([`crate::redact::apply`], `REDACTED_MARKER` policy) applied
/// **per line** — only secret-bearing lines are replaced, every non-secret
/// line survives (never whole-blob; the X3 defect). The joined text is the
/// blob that gets stored.
fn redact_transcript(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| crate::redact::apply(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Redact each element of a string vector on the write path (the
/// `constraints` / `acceptance` / `parent_intents` envelope fields). Per
/// element — a non-secret entry survives intact (the X3 lesson).
fn redact_each(items: &[String]) -> Vec<String> {
    items.iter().map(|s| crate::redact::apply(s)).collect()
}

/// Close an authored unit: gate by `level`, redact on the write path, store
/// the trajectory blobs in the cold store, and store the envelope itself —
/// returning it with its `context_ref`.
///
/// Gating (ADR-0001 §3 — absent refs are `null`):
///
/// | level | metrics | `task_transcript_ref` + `summary` | `raw_transcript_ref` + `prompt_ref` |
/// |---|---|---|---|
/// | `off` | zeroed | null | null |
/// | `metrics` | kept | null | null |
/// | `task` | kept | stored | null |
/// | `full` (default) | kept | stored | stored |
///
/// `journal_ref`/`verdicts_ref` are pass-throughs (owned elsewhere, not
/// trajectory blobs written here).
///
/// # Errors
/// Fails iff a blob write fails (including the disclosed unwired live
/// seam, which fails closed).
pub fn close_envelope<S: ColdBlobStore>(
    draft: &EnvelopeDraft,
    level: CaptureLevel,
    store: &S,
) -> Result<ClosedEnvelope, EnvelopeError> {
    // Redaction happens BEFORE any store.put — the write path is the only
    // path to bytes-at-rest, and it only ever sees redacted text. An EMPTY
    // transcript stores nothing and yields a null ref: "nothing captured" is
    // not a real transcript. Under `full` that null is exactly the silent
    // omission the two-transcript imperative forbids — caught below — so an
    // empty transcript can never masquerade as a captured one.
    let raw_transcript_ref = if level >= CaptureLevel::Full && !draft.raw_transcript.is_empty() {
        Some(store.put(redact_transcript(&draft.raw_transcript).as_bytes())?)
    } else {
        None
    };
    let task_transcript_ref = if level >= CaptureLevel::Task && !draft.task_transcript.is_empty() {
        Some(store.put(redact_transcript(&draft.task_transcript).as_bytes())?)
    } else {
        None
    };
    let summary = if level >= CaptureLevel::Task {
        Some(crate::redact::apply(&draft.summary))
    } else {
        None
    };
    let prompt_ref = if level >= CaptureLevel::Full {
        match &draft.prompt {
            Some(p) => Some(store.put(crate::redact::apply(p).as_bytes())?),
            None => None,
        }
    } else {
        None
    };
    // `off` = envelope metadata only: the metrics block is zeroed (the
    // frozen shape keeps the field; zero is the honest "not captured").
    let metrics = if level >= CaptureLevel::Metrics {
        draft.metrics.clone()
    } else {
        IntentMetrics {
            tokens: TokenCounts {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                total: 0,
            },
            wall_ms: 0,
            active_ms: 0,
            tool_calls: 0,
            tool_breakdown: vec![],
            model_turns: 0,
            cost_usd_micros: 0,
        }
    };

    // The two-transcript imperative (ADR-0001 §2, second owner directive):
    // under `full` — the ratified default — BOTH transcript refs MUST be
    // present at EVERY altitude. Enforced AFTER gating, BEFORE the envelope is
    // built, so a Full close can never emit a silent null for either ref. The
    // `Full >= Task >= ...` gating already populates both at `full`; this is
    // the hard invariant that keeps it that way (and catches any future
    // regression). Nulls are legal only when the level is an explicit
    // opt-down (off/metrics/task) — checked solely at Full.
    if level >= CaptureLevel::Full {
        let missing = match (raw_transcript_ref.is_none(), task_transcript_ref.is_none()) {
            (true, true) => Some("raw_transcript_ref and task_transcript_ref"),
            (true, false) => Some("raw_transcript_ref"),
            (false, true) => Some("task_transcript_ref"),
            (false, false) => None,
        };
        if let Some(missing) = missing {
            return Err(EnvelopeError::TwoTranscriptViolation {
                altitude: draft.altitude,
                missing,
            });
        }
    }

    // Redaction on the WRITE path covers EVERY string field that carries
    // author-supplied text — routed through [`crate::redact::apply`] before
    // the envelope is serialized and stored (the charter/constraints/
    // acceptance/parent_intents/env_manifest/files_read-paths were previously
    // persisted verbatim — a `ghp_`-style secret in a charter rode through).
    // Exempt by design (not author-supplied text):
    //   • `redaction_policy` — a policy identifier literal, not user content.
    //   • `summary` — already scrubbed above via `redact_transcript`.
    //   • `files_read[].hash` — a content-address digest (e.g. `sha256:…`),
    //     produced by the harness, not typed by the author; redacting it would
    //     destroy the content-pinning guarantee. Only the `path` field of each
    //     `FileRead` entry is scrubbed (could contain a secret-bearing path).
    let envelope = ContextEnvelope {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.to_string(),
        altitude: draft.altitude,
        intent_id: draft.unit_id.clone(),
        commit: draft.commit.clone(),
        tree_hash: draft.tree_hash.clone(),
        authorship: draft.authorship.clone(),
        charter: crate::redact::apply(&draft.charter),
        campaign: draft.campaign.clone(),
        constraints: redact_each(&draft.constraints),
        acceptance: redact_each(&draft.acceptance),
        parent_intents: redact_each(&draft.parent_intents),
        trajectory: Trajectory {
            raw_transcript_ref,
            task_transcript_ref,
            summary,
            journal_ref: draft.journal_ref.clone(),
            redaction_policy: REDACTION_POLICY_DEFAULT.to_string(),
        },
        snapshot: Snapshot {
            files_read: draft
                .files_read
                .iter()
                .map(|f| FileRead {
                    path: crate::redact::apply(&f.path),
                    hash: f.hash.clone(),
                })
                .collect(),
            prompt_ref,
            env_manifest: crate::redact::apply(&draft.env_manifest),
        },
        metrics,
        verdicts_ref: draft.verdicts_ref.clone(),
    };

    // The envelope blob itself is content-addressed behind the same
    // tier-agnostic ref scheme — this is the `IntentSidecar.context_ref`.
    // The shape is frozen and all money fields are integers (WA4), so
    // serialization failure is unreachable in practice — but a panic here
    // would abort the capture path. `EnvelopeError::Serialize` replaces the
    // panic class with a fail-closed `Result` (the error surfaces to the
    // caller; the process continues).
    let bytes =
        serde_json::to_vec(&envelope).map_err(|e| EnvelopeError::Serialize(e.to_string()))?;
    let context_ref = store.put(&bytes)?;

    Ok(ClosedEnvelope {
        envelope,
        context_ref,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Session (PR / campaign) close — orchestrator envelope + waste
// ─────────────────────────────────────────────────────────────────────────────

/// What a session close emits at the `session`/`pr`/`campaign` altitudes:
/// the captured envelope + its ref (feeds the derived `PrRecord.envelope_ref`
/// / `CampaignRollup.envelope_ref`; the `session` altitude is the physical
/// home those refs dedupe into), the author's own coordination spend (feeds
/// `cost.orchestration`), and the **waste** figures (feeds `cost.waste`) —
/// shown, not hidden. The rollup itself is WP-F3's computation; this is the
/// producer-side emission it consumes.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEmission {
    /// The session envelope as stored (`altitude: session | pr | campaign`).
    pub envelope: ContextEnvelope,
    /// Content-addressed ref of the stored session envelope blob.
    pub envelope_ref: String,
    /// The session author's coordination spend, derived from the session's
    /// measured metrics (tokens/tool-calls/turns spent planning,
    /// dispatching, cold-verifying, landing — NOT attributed to any intent).
    pub orchestration: OrchestrationCost,
    /// Spent-but-not-landed: discarded intents, retried agents,
    /// tokens-not-landed.
    pub waste: WasteCost,
}

/// Close a `session` / `pr` / `campaign` envelope — the SAME capture path as
/// intents ([`close_envelope`]): redact-on-write, capture-level gating,
/// cold-store blobs, and the two-transcript imperative. The transcript blobs
/// are content-addressed, so the `session` altitude is the physical HOME of a
/// session's raw + task blobs and the `pr` / `campaign` envelopes that same
/// session authors REFERENCE the identical bytes (CAS dedupes: equal content
/// → one stored blob, equal ref). That relationship is explicit, not
/// coincidental — a Pr/Campaign envelope's trajectory refs MAY equal the
/// Session envelope's refs, and the dedupe proves it.
///
/// # Errors
/// Fails on a blob-store failure, on the two-transcript imperative (a `full`
/// close missing either transcript ref), or fail-closed if called at the
/// `intent` altitude (waste/orchestration emission is a session-author
/// concern; intents close via [`close_envelope`]).
pub fn close_session_envelope<S: ColdBlobStore>(
    draft: &EnvelopeDraft,
    level: CaptureLevel,
    store: &S,
    waste: WasteCost,
) -> Result<SessionEmission, EnvelopeError> {
    if draft.altitude == Altitude::Intent {
        return Err(EnvelopeError::NotASessionAltitude(draft.altitude));
    }
    let closed = close_envelope(draft, level, store)?;
    let m = &closed.envelope.metrics;
    let orchestration = OrchestrationCost {
        tokens: m.tokens.total,
        tool_calls: m.tool_calls,
        turns: m.model_turns,
        cost_usd_micros: m.cost_usd_micros,
    };
    Ok(SessionEmission {
        envelope: closed.envelope,
        envelope_ref: closed.context_ref,
        orchestration,
        waste,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_contracts::context_envelope::Spawn;

    fn authorship(run_id: &str) -> Authorship {
        Authorship {
            model: "in-process-deterministic".to_string(),
            model_digest: "0".repeat(64),
            agent_type: "implementer".to_string(),
            spawn: Spawn {
                run_id: run_id.to_string(),
                parent_run_id: None,
                born_at: 1,
                died_at: 2,
            },
            operator: "test@humangr.com".to_string(),
        }
    }

    fn draft(altitude: Altitude) -> EnvelopeDraft {
        EnvelopeDraft {
            altitude,
            unit_id: "u-1".to_string(),
            commit: "c".repeat(40),
            tree_hash: "t".repeat(64),
            authorship: authorship("run-1"),
            charter: "test charter".to_string(),
            campaign: None,
            constraints: vec![],
            acceptance: vec![],
            parent_intents: vec![],
            raw_transcript: vec!["turn 1".to_string()],
            task_transcript: vec!["brief in".to_string()],
            summary: "did the thing".to_string(),
            journal_ref: None,
            files_read: vec![],
            prompt: Some("system prompt".to_string()),
            env_manifest: "rustc 1.96.0".to_string(),
            metrics: IntentMetrics {
                tokens: TokenCounts {
                    input: 10,
                    output: 5,
                    cache_read: 3,
                    cache_write: 2,
                    total: 20,
                },
                wall_ms: 7,
                active_ms: 4,
                tool_calls: 1,
                tool_breakdown: vec![ToolCount {
                    tool: "Bash".to_string(),
                    count: 1,
                }],
                model_turns: 1,
                cost_usd_micros: 0,
            },
            verdicts_ref: None,
        }
    }

    #[test]
    fn capture_level_parse_and_default() {
        assert_eq!(CaptureLevel::parse("off"), Some(CaptureLevel::Off));
        assert_eq!(CaptureLevel::parse("metrics"), Some(CaptureLevel::Metrics));
        assert_eq!(CaptureLevel::parse("task"), Some(CaptureLevel::Task));
        assert_eq!(CaptureLevel::parse("full"), Some(CaptureLevel::Full));
        assert_eq!(CaptureLevel::parse("FULL"), None, "fail-closed on unknown");
        // The ratified default is `full` at every altitude.
        assert_eq!(CaptureLevel::default(), CaptureLevel::Full);
        assert_eq!(CaptureLevel::default().as_str(), "full");
    }

    #[test]
    fn recorder_measures_tokens_with_cache_split_and_tools() {
        let mut rec = TrajectoryRecorder::start();
        rec.record_task_event("brief: do x");
        rec.record_turn("model turn 1", Duration::from_millis(3));
        rec.record_tool_call("Bash", "Bash(cargo test) -> ok", Duration::from_millis(2));
        rec.record_tool_call("Bash", "Bash(cargo fmt) -> ok", Duration::from_millis(1));
        rec.record_tool_call("Edit", "Edit(src/lib.rs) -> ok", Duration::from_millis(1));
        rec.add_tokens(100, 40, 60, 10);
        rec.add_tokens(50, 10, 0, 0);
        let t = rec.finish(0);

        assert_eq!(t.metrics.tokens.input, 150);
        assert_eq!(t.metrics.tokens.output, 50);
        assert_eq!(t.metrics.tokens.cache_read, 60);
        assert_eq!(t.metrics.tokens.cache_write, 10);
        assert_eq!(t.metrics.tokens.total, 270);
        assert_eq!(t.metrics.tool_calls, 3);
        assert_eq!(
            t.metrics.tool_breakdown,
            vec![
                ToolCount {
                    tool: "Bash".to_string(),
                    count: 2
                },
                ToolCount {
                    tool: "Edit".to_string(),
                    count: 1
                },
            ]
        );
        assert_eq!(t.metrics.model_turns, 1);
        assert!(t.metrics.active_ms >= 7, "busy time is summed");
        assert!(t.died_at >= t.born_at, "lifespan is ordered");
        // raw ⊇ task: the task event also rides the raw track
        // (1 task event + 1 turn + 3 tool calls = 5 raw events).
        assert_eq!(t.raw_transcript.len(), 5);
        assert_eq!(t.task_transcript, vec!["brief: do x".to_string()]);
    }

    #[test]
    fn session_close_refuses_intent_altitude() {
        let store = InMemoryColdStore::new();
        let waste = WasteCost {
            discarded_intents: 0,
            retried_agents: 0,
            tokens_not_landed: 0,
            cost_usd_micros: 0,
        };
        let err = close_session_envelope(
            &draft(Altitude::Intent),
            CaptureLevel::default(),
            &store,
            waste,
        )
        .expect_err("intent altitude must be refused");
        assert_eq!(err, EnvelopeError::NotASessionAltitude(Altitude::Intent));
    }

    #[test]
    fn session_close_emits_orchestration_from_measured_metrics() {
        let store = InMemoryColdStore::new();
        let waste = WasteCost {
            discarded_intents: 2,
            retried_agents: 1,
            tokens_not_landed: 30,
            cost_usd_micros: 0,
        };
        let emission = close_session_envelope(
            &draft(Altitude::Pr),
            CaptureLevel::default(),
            &store,
            waste.clone(),
        )
        .expect("session close");
        assert_eq!(emission.envelope.altitude, Altitude::Pr);
        assert_eq!(emission.orchestration.tokens, 20);
        assert_eq!(emission.orchestration.tool_calls, 1);
        assert_eq!(emission.orchestration.turns, 1);
        assert_eq!(emission.waste, waste);
        // The emitted ref resolves to the stored envelope blob.
        let bytes = store
            .get(&emission.envelope_ref)
            .expect("get")
            .present()
            .expect("present");
        let back: ContextEnvelope = serde_json::from_slice(&bytes).expect("frozen shape");
        assert_eq!(back, emission.envelope);
    }

    #[test]
    fn session_altitude_is_first_class() {
        // Session is the fourth altitude and a valid session close (it is the
        // physical home of the session's blobs — NOT refused like `intent`).
        let store = InMemoryColdStore::new();
        let waste = WasteCost {
            discarded_intents: 0,
            retried_agents: 0,
            tokens_not_landed: 0,
            cost_usd_micros: 0,
        };
        let emission = close_session_envelope(
            &draft(Altitude::Session),
            CaptureLevel::default(),
            &store,
            waste,
        )
        .expect("session altitude is first-class, not refused");
        assert_eq!(emission.envelope.altitude, Altitude::Session);
        // Full close ⇒ both transcript refs present (imperative satisfied).
        assert!(emission.envelope.trajectory.raw_transcript_ref.is_some());
        assert!(emission.envelope.trajectory.task_transcript_ref.is_some());
    }

    #[test]
    fn two_transcript_imperative_full_demands_both_refs() {
        let store = InMemoryColdStore::new();

        // Full close with an EMPTY raw transcript ⇒ the raw ref would be null:
        // a hard violation, not a silent null.
        let mut no_raw = draft(Altitude::Intent);
        no_raw.raw_transcript = vec![];
        let err = close_envelope(&no_raw, CaptureLevel::Full, &store)
            .expect_err("full without raw must be refused");
        assert_eq!(
            err,
            EnvelopeError::TwoTranscriptViolation {
                altitude: Altitude::Intent,
                missing: "raw_transcript_ref",
            }
        );

        // Full close with an EMPTY task transcript ⇒ task ref null ⇒ refused.
        let mut no_task = draft(Altitude::Pr);
        no_task.task_transcript = vec![];
        let err = close_envelope(&no_task, CaptureLevel::Full, &store)
            .expect_err("full without task must be refused");
        assert_eq!(
            err,
            EnvelopeError::TwoTranscriptViolation {
                altitude: Altitude::Pr,
                missing: "task_transcript_ref",
            }
        );

        // Both empty ⇒ the message names both.
        let mut neither = draft(Altitude::Campaign);
        neither.raw_transcript = vec![];
        neither.task_transcript = vec![];
        let err = close_envelope(&neither, CaptureLevel::Full, &store)
            .expect_err("full without either must be refused");
        assert_eq!(
            err,
            EnvelopeError::TwoTranscriptViolation {
                altitude: Altitude::Campaign,
                missing: "raw_transcript_ref and task_transcript_ref",
            }
        );

        // The violation flows through close_session_envelope too (same path).
        let mut session_no_raw = draft(Altitude::Session);
        session_no_raw.raw_transcript = vec![];
        let waste = WasteCost {
            discarded_intents: 0,
            retried_agents: 0,
            tokens_not_landed: 0,
            cost_usd_micros: 0,
        };
        let err = close_session_envelope(&session_no_raw, CaptureLevel::Full, &store, waste)
            .expect_err("the imperative holds at the session altitude too");
        assert!(matches!(
            err,
            EnvelopeError::TwoTranscriptViolation {
                altitude: Altitude::Session,
                ..
            }
        ));
    }

    #[test]
    fn two_transcript_imperative_nulls_legal_only_under_opt_down() {
        let store = InMemoryColdStore::new();
        // An empty-transcript draft that would violate Full is FINE under an
        // explicit opt-down: nulls are legal there, by design.
        let mut empty = draft(Altitude::Intent);
        empty.raw_transcript = vec![];
        empty.task_transcript = vec![];

        for level in [CaptureLevel::Off, CaptureLevel::Metrics, CaptureLevel::Task] {
            let closed = close_envelope(&empty, level, &store)
                .unwrap_or_else(|e| panic!("opt-down {level:?} tolerates nulls, got {e}"));
            assert_eq!(closed.envelope.trajectory.raw_transcript_ref, None);
            // task ref is null too (empty task transcript, even at `task`).
            assert_eq!(closed.envelope.trajectory.task_transcript_ref, None);
        }
    }

    /// **NaN/serialization panic (A2-F14)**: `EnvelopeError::Serialize` is the
    /// fail-closed replacement for `serde_json::to_vec(...).expect(...)`. The
    /// frozen `ContextEnvelope` shape carries no `f64` fields (WA4 — money is
    /// integer micro-USD), so serialization failure is unreachable with the
    /// current schema; the test proves the error variant and its `Display` are
    /// correctly wired rather than panicking.
    ///
    /// We verify: (a) the variant constructs and round-trips through `Display`;
    /// (b) it is distinct from all other `EnvelopeError` arms; (c) a successful
    /// close (the normal path) does NOT return `Serialize`.
    #[test]
    fn serialize_error_variant_is_fail_closed_not_a_panic() {
        // (a) The variant constructs and its Display is informative.
        let err = EnvelopeError::Serialize("simulated: value is not finite".to_string());
        let msg = err.to_string();
        assert!(
            msg.contains("fail-closed"),
            "Display must mention fail-closed: {msg}"
        );
        assert!(
            msg.contains("simulated"),
            "Display must include the underlying message: {msg}"
        );

        // (b) The variant is distinguishable from other arms (pattern match).
        assert!(
            matches!(err, EnvelopeError::Serialize(_)),
            "variant matches its own arm"
        );
        assert!(
            !matches!(err, EnvelopeError::Store(_)),
            "not a Store variant"
        );

        // (c) A well-formed envelope does NOT return Serialize — the normal path
        // succeeds, confirming the error arm is wired but unreachable with the
        // current frozen (all-integer) schema.
        let store = InMemoryColdStore::new();
        let result = close_envelope(&draft(Altitude::Intent), CaptureLevel::Full, &store);
        assert!(
            !matches!(result, Err(EnvelopeError::Serialize(_))),
            "a well-formed frozen envelope never hits the Serialize arm: {result:?}"
        );
    }
}
