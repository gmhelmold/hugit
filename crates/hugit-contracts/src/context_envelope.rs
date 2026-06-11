//! ContextEnvelope — frozen by ADR-0001 (`docs/adr/0001-intent-context-envelope.md`,
//! ratified 2026-06-10), WP-F1.
//!
//! `context.json` — the Intent Context Envelope: the only durable record of
//! how a change came to exist (its authors are ephemeral agents). Small
//! structured metadata + metrics held inline, with content-addressed refs
//! (`cas:…`) to the large blobs (transcripts, journal, prompt). One envelope
//! per authored unit, **at every altitude** (owner-directed 2026-06-10): the
//! intent carries the subagent's envelope, the PR carries the
//! orchestrator-session envelope, the campaign its own — and the session
//! itself is a fourth first-class altitude (second owner directive, same
//! day; WP-F1b): the physical home of the session's full + compacted
//! transcript blobs, referenced (deduped) by the PR/campaign envelopes that
//! session authored. Same shape at every altitude — only
//! [`ContextEnvelope::altitude`] and the authored-unit id change.
//!
//! Capture levels (ADR-0001 §3): default is `full` — RATIFIED, non-negotiable.
//! The level ladder (`off | metrics | task | full`) survives only as a
//! per-repo privacy opt-DOWN; refs absent below the active level are `null`
//! and consumers MUST tolerate nulls. Retention is forever by default (no
//! TTL field exists by ratified design — erasure only via the explicit
//! tombstone path, never a timer).
//!
//! Frozen here: [`ContextEnvelope`] (+ its sub-structs) and [`IntentMetrics`]
//! — `deny_unknown_fields`, committed JSON Schema, golden byte-exact
//! round-trips at all four altitudes. **Derived, NOT frozen:** [`PrRecord`]
//! and [`CampaignRollup`] (ADR-0001 §2.3) are forge-COMPUTED read shapes —
//! plain serde structs without `deny_unknown_fields` and without a committed
//! schema, so the forge may evolve them additively without a contract break.
//!
//! # Naming reconciliation — "intent" across the frozen v1 names (WP-F1)
//!
//! Product model (ADR-0001): **intent = commit** (the hugr-native authored
//! unit) · **PR = bundle of intents** · **campaign = bundle of PRs** (the
//! landing-queue bundle key — the queue unions and lands PRs, never raw
//! commits; nesting is strict: `commit ⊂ PR ⊂ campaign-bundle`).
//!
//! The frozen v1 contracts predate that refinement and use "intent" for the
//! unit the landing pipeline operates on: [`crate::IntentSidecar`] is
//! documented as *"what the PR intends to do"* (PR-attached), and
//! [`crate::queue_api::LandableEntry`]`.intent_id` lands at that same level.
//! The map, additive (no frozen v1 type changes):
//!
//! | Product altitude | Authored unit | Where its id lives |
//! |---|---|---|
//! | `intent` | a commit | `ContextEnvelope.intent_id` at `altitude:"intent"` |
//! | `pr` | a bundle of intents | frozen `IntentSidecar.intent_id` = `LandableEntry.intent_id` (the v1 landing unit) · `ContextEnvelope.intent_id` at `altitude:"pr"` carries the PR id |
//! | `campaign` | a bundle of PRs | `ContextEnvelope.intent_id` at `altitude:"campaign"` carries the campaign key · `CampaignRollup.campaign` |
//! | `session` | an agent session/run | `ContextEnvelope.intent_id` at `altitude:"session"` carries the session/run id (WP-F1b) |
//!
//! Disambiguation rule: when a frozen v1 name says "intent" it means the
//! **landing unit** (the PR altitude in v1); when ADR-0001 says "intent"
//! unqualified it means the **commit altitude**. On the data itself the
//! [`Altitude`] discriminator declares which sense applies to every envelope
//! — `IntentSidecar.context_ref` points at a `ContextEnvelope`, and that
//! envelope's `altitude` field states the altitude of the unit the sidecar
//! is attached to. The word therefore never silently means two things:
//! the discriminator, not the field name, is authoritative.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The frozen envelope schema version carried in
/// [`ContextEnvelope::schema_version`] (semver; ADR-0001 §2.2).
///
/// `1.1.0` — WP-F1b (second owner directive, 2026-06-10): adds
/// [`Altitude::Session`] as a fourth altitude. Additive amendment made the
/// same day as the 1.0.0 freeze, while **zero producers existed** — no
/// migration path was ever needed.
///
/// `1.2.0` — WA4 money-as-integer amendment (owner-ratified 2026-06-11,
/// SOTA-audit Tier-3 money item, recommended decision "ok shoot it"). **Money
/// leaves f64.** Every cost field — [`IntentMetrics::cost_usd_micros`] and the
/// derived [`CostDecomposition`] family's `*_usd_micros` fields — becomes an
/// integer **micro-USD** `u64` (`1 USD = 1_000_000` micro-USD). The cost
/// identity `total = work + orchestration + verification + ci` is now a
/// *theorem* over the integers — bit-exact, no floating-point epsilon — rather
/// than an approximate float sum that drifted at scale. This is a clean
/// **versioned break** of the 1.1.0 wire shape (`cost_usd: f64` is gone);
/// correct because **zero producers exist in prod** — no migration path was
/// ever needed. Display-only ratios (`overhead_pct`, `cache_savings_pct`) stay
/// f64, computed from the integers at the end (division for display is fine).
pub const CONTEXT_ENVELOPE_SCHEMA_VERSION: &str = "1.2.0";

/// The altitude of the authored unit this envelope records (owner-directed
/// 2026-06-10): `intent` = a commit (subagent-authored) · `pr` = a bundle of
/// intents (orchestrator/human session) · `campaign` = a bundle of PRs
/// (human-owned) · `session` = the agent session/run itself (second owner
/// directive, same day). Same envelope shape at every altitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Altitude {
    /// A commit — the hugr-native authored unit, authored by a subagent.
    Intent,
    /// A PR — a bundle of intents; the envelope is the orchestrator-session
    /// (or human-session) record that planned/dispatched/landed the bundle.
    Pr,
    /// A campaign — a bundle of PRs in the landing queue, owned by a human.
    Campaign,
    /// A session — the fourth first-class altitude (second owner directive,
    /// 2026-06-10). The session envelope is the **physical home** of a
    /// session's full + compacted transcript blobs: one session may author
    /// several PRs/campaigns, and their envelopes reference INTO the session
    /// envelope's blobs (CAS-deduped, never duplicated). Its authored-unit id
    /// ([`ContextEnvelope::intent_id`]) is the session/run id.
    Session,
}

/// The agent's lifespan + spawn lineage (agents spawn and die per intent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    /// Run identifier of the agent run that authored the unit.
    pub run_id: String,
    /// Run identifier of the orchestrator that spawned it; `null` for a
    /// top-level (unspawned) session.
    pub parent_run_id: Option<String>,
    /// Unix ms — when the agent run was born.
    pub born_at: u64,
    /// Unix ms — when the agent run died.
    pub died_at: u64,
}

/// Provenance: who/what authored the unit (ADR-0001 §2.2 `authorship`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Authorship {
    /// Model identifier (field name consistent with `VerdictObject.model`).
    pub model: String,
    /// Pinned model version digest.
    pub model_digest: String,
    /// Subagent type (e.g. `"implementer"`), or `"main"` for a top-level
    /// (orchestrator / campaign) session.
    pub agent_type: String,
    /// The agent's spawn lineage and lifespan.
    pub spawn: Spawn,
    /// Human principal who dispatched the work.
    pub operator: String,
}

/// The trajectory at three altitudes (ADR-0001 §2.1): raw (forensic,
/// born → die) · task (mid-altitude "what happened") · summary (inline
/// digest). Tenant-private, redacted on the write path.
///
/// **Two-transcript imperative (second owner directive, 2026-06-10):**
/// `raw_transcript_ref` (the FULL transcript) AND `task_transcript_ref`
/// (the COMPACTED transcript — "task" is the owner's "compacted") are
/// **IMPERATIVE at every altitude** (intent · pr · campaign · session)
/// under the ratified default capture level (`full`). They are `null`
/// ONLY under an explicit per-tenant capture opt-DOWN — never as a
/// producer convenience. Consumers MUST still tolerate nulls (the
/// opt-down exists), but a producer emitting nulls under default capture
/// is in violation of the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trajectory {
    /// `cas:` ref to the complete agent loop, born → die: every model turn,
    /// every tool call + result, system/charter prompts (redacted).
    /// IMPERATIVE at every altitude under default capture (`full`);
    /// `null` only under an explicit tenant capture opt-down.
    pub raw_transcript_ref: Option<String>,
    /// `cas:` ref to the task-scoped (compacted) mid-altitude transcript:
    /// brief in, plan/step progression, key decisions, result/handoff out.
    /// IMPERATIVE at every altitude under default capture; `null` only
    /// under an explicit tenant capture opt-down (below level `task`).
    pub task_transcript_ref: Option<String>,
    /// Inline LLM digest, a few sentences — for the drawer.
    /// Present at capture level `task` and above; `null` below.
    pub summary: Option<String>,
    /// `cas:` ref to the append-only `hugit-ledger` Journal — the
    /// human-annotation track alongside the machine trajectory; `null` when
    /// absent.
    pub journal_ref: Option<String>,
    /// Which redaction policy scrubbed secrets/PII before the blobs were
    /// written (redaction is on the write path).
    pub redaction_policy: String,
}

/// A file the agent read, content-pinned (ADR-0001 §2.2 `snapshot.files_read`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRead {
    /// Repo-relative path of the file read.
    pub path: String,
    /// Content hash of the file as read (e.g. `sha256:…`).
    pub hash: String,
}

/// What the agent read / the environment it ran in (ADR-0001 §2.2 `snapshot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// Files the agent read, content-pinned.
    pub files_read: Vec<FileRead>,
    /// `cas:` ref to the (redacted) prompt blob; `null` below the capture
    /// level that stores it.
    pub prompt_ref: Option<String>,
    /// Environment manifest (toolchain pin, e.g. `"rustc 1.96.0"`).
    pub env_manifest: String,
}

/// Token spend with the cache split (ADR-0001 §2.2 `metrics.tokens`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenCounts {
    /// Input tokens (non-cached).
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens read from prompt cache.
    pub cache_read: u64,
    /// Tokens written to prompt cache.
    pub cache_write: u64,
    /// Total tokens.
    pub total: u64,
}

/// Per-tool call count (ADR-0001 §2.2 `metrics.tool_breakdown[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolCount {
    /// Tool name (e.g. `"Edit"`, `"Bash"`).
    pub tool: String,
    /// Number of calls to this tool.
    pub count: u64,
}

/// Per-unit measurement (ADR-0001 §2.2 `metrics` / §2.3 per-intent): the
/// vocabulary of fleet accountability. `cost_usd_micros` is derived COGS shown
/// for trust — NOT what the customer is billed (never a usage meter).
///
/// Frozen by WP-F1 (ADR-0001); money widened to integer micro-USD by the WA4
/// amendment (schema 1.2.0, [`CONTEXT_ENVELOPE_SCHEMA_VERSION`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentMetrics {
    /// Token spend with the cache split.
    pub tokens: TokenCounts,
    /// Born → die wall-clock, ms.
    pub wall_ms: u64,
    /// Model + tool busy time, ms (excludes idle).
    pub active_ms: u64,
    /// Total tool calls.
    pub tool_calls: u64,
    /// Per-tool call breakdown.
    pub tool_breakdown: Vec<ToolCount>,
    /// Number of model turns.
    pub model_turns: u64,
    /// Derived COGS in integer **micro-USD** (`1 USD = 1_000_000`); NOT what
    /// the customer is billed. Integer minor units so the cost identity is
    /// bit-exact (WA4 amendment, schema 1.2.0).
    pub cost_usd_micros: u64,
}

/// The Intent Context Envelope — `context.json` (ADR-0001 §2.2).
///
/// One envelope per authored unit at every altitude (intent · PR ·
/// campaign · session); reachable behind `IntentSidecar.context_ref` (a CAS
/// pointer) and the derived records' `envelope_ref`. See the module docs for
/// the naming map onto the frozen v1 `intent_id` fields.
///
/// Frozen by WP-F1 (ADR-0001, ratified 2026-06-10); amended additively by
/// WP-F1b the same day (schema 1.1.0, `Altitude::Session`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextEnvelope {
    /// Envelope schema version (semver); see
    /// [`CONTEXT_ENVELOPE_SCHEMA_VERSION`].
    pub schema_version: String,

    /// The altitude of the authored unit (intent | pr | campaign | session).
    pub altitude: Altitude,

    /// The authored-unit id at this envelope's altitude: the commit-level
    /// intent id at `intent`, the PR id at `pr`, the campaign key at
    /// `campaign`, the session/run id at `session` (ADR-0001 §2.2 + WP-F1b).
    /// See the module docs for how this maps onto the frozen v1 `intent_id`
    /// fields (which name the landing unit).
    pub intent_id: String,

    /// Git commit this unit enriches (the intent's commit; at higher
    /// altitudes, the unit's landed/head commit).
    pub commit: String,

    /// Merkle tree hash of the unit's workspace snapshot.
    pub tree_hash: String,

    /// Provenance: who/what authored the unit (agents are ephemeral).
    pub authorship: Authorship,

    /// The "why" — human-readable charter for the unit.
    pub charter: String,

    /// Campaign key this unit belongs to; `null` when uncampaigned. Drives
    /// subliminal grouping (the landing queue bundles PRs by campaign).
    pub campaign: Option<String>,

    /// Constraints the unit was authored under.
    pub constraints: Vec<String>,

    /// Acceptance criteria for the unit.
    pub acceptance: Vec<String>,

    /// Ids of parent intents this unit builds on.
    pub parent_intents: Vec<String>,

    /// The three-altitude trajectory (§2.1) — tenant-private, redacted.
    pub trajectory: Trajectory,

    /// What the agent read / the environment it ran in.
    pub snapshot: Snapshot,

    /// Per-unit measurement (§2.3).
    pub metrics: IntentMetrics,

    /// `cas:` ref to the adversarial panel verdicts (`VerdictObject`s);
    /// `null` when no panel ran.
    pub verdicts_ref: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Derived shapes (ADR-0001 §2.3) — forge-COMPUTED, NOT frozen.
//
// The forge computes these records from the captured envelopes + queue/authz
// state (WP-F3); they are never stored per envelope and never gate anything.
// Deliberately NO `deny_unknown_fields` and NO committed JSON Schema: the
// forge may evolve them additively without a contract break. Consumers must
// treat them as read models.
// ─────────────────────────────────────────────────────────────────────────────

/// Who authored a PR. **A PR author is an orchestrator or a human — NEVER a
/// subagent** (forge-authz invariant D14, ADR-0001 §2.3/§5); the absence of
/// a subagent variant encodes the rule at the type level.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrAuthorKind {
    /// The orchestrator session that planned/dispatched/landed the bundle.
    Orchestrator,
    /// A human principal.
    Human,
}

/// The PR's author (ADR-0001 §2.3 `author`): `kind:"orchestrator"` carries
/// `model` + `run_id`; `kind:"human"` carries `principal`.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrAuthor {
    /// Author kind — orchestrator or human, never a subagent (D14).
    pub kind: PrAuthorKind,
    /// Model identifier (orchestrator authors); `null` for humans.
    pub model: Option<String>,
    /// Orchestrator run id; `null` for humans.
    pub run_id: Option<String>,
    /// Human principal; `null` for orchestrators.
    pub principal: Option<String>,
}

/// Cost of the work itself — Σ over the unit's intents (the subagents).
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkCost {
    /// Total tokens across the intents.
    pub tokens: u64,
    /// Total tool calls across the intents.
    pub tool_calls: u64,
    /// Derived COGS in integer micro-USD (`1 USD = 1_000_000`).
    pub cost_usd_micros: u64,
}

/// The unit author's own coordination spend on top of the work: planning,
/// decomposing, dispatching, cold-verifying, landing.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrchestrationCost {
    /// Tokens spent by the unit's author.
    pub tokens: u64,
    /// Tool calls made by the unit's author.
    pub tool_calls: u64,
    /// Model turns of the author's session.
    pub turns: u64,
    /// Derived COGS in integer micro-USD (`1 USD = 1_000_000`).
    pub cost_usd_micros: u64,
}

/// Verification spend — the adversarial review panels.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationCost {
    /// Tokens spent on review calls.
    pub tokens: u64,
    /// Number of verdict panels run.
    pub verdict_panels: u64,
    /// Derived COGS in integer micro-USD (`1 USD = 1_000_000`).
    pub cost_usd_micros: u64,
}

/// CI spend and the memoization economics.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CiCost {
    /// Check executions served from the memo cache.
    pub cache_hit: u64,
    /// Check executions actually run.
    pub exec: u64,
    /// Derived COGS in integer micro-USD of the executed checks.
    pub cost_usd_micros: u64,
    /// Micro-USD saved by memoization (would-be cost of the cache hits).
    pub saved_usd_micros: u64,
}

/// Waste — spent-but-not-landed, shown not hidden (gross spend vs landed
/// spend = honest first-pass yield).
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WasteCost {
    /// Intents authored but discarded (never landed).
    pub discarded_intents: u64,
    /// Agent runs retried.
    pub retried_agents: u64,
    /// Tokens spent on work that never landed.
    pub tokens_not_landed: u64,
    /// Derived COGS in integer micro-USD of the waste.
    pub cost_usd_micros: u64,
}

/// The total line: work + orchestration + verification + ci.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TotalCost {
    /// Total tokens.
    pub tokens: u64,
    /// Total derived COGS in integer micro-USD.
    pub cost_usd_micros: u64,
}

/// Cost decomposed by WHERE it went (ADR-0001 §2.3 `cost`) — work
/// (subagents) vs orchestration (the unit's author) vs verification (panels)
/// vs CI (memoized), with waste shown, not hidden. One vocabulary at every
/// altitude.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostDecomposition {
    /// Σ intents (the subagents' spend).
    pub work: WorkCost,
    /// The unit author's own coordination spend.
    pub orchestration: OrchestrationCost,
    /// Adversarial review calls.
    pub verification: VerificationCost,
    /// CI spend + memoization economics.
    pub ci: CiCost,
    /// Spent-but-not-landed.
    pub waste: WasteCost,
    /// work + orchestration + verification + ci.
    pub total: TotalCost,
}

/// The PR's time block (ADR-0001 §2.3 `time`). Span vs sum is deliberate:
/// `wall_span_ms` = clock time (reflects fleet parallelism), `agent_sum_ms`
/// = Σ agent-time (> span when parallel). The forge shows both.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrTime {
    /// First activity → landed (cycle time), ms.
    pub wall_span_ms: u64,
    /// Σ per-intent active time (agent-time), ms.
    pub agent_sum_ms: u64,
    /// Time held in the landing queue, ms.
    pub queue_wait_ms: u64,
    /// Human decisions/comments on the PR.
    pub human_touches: u64,
    /// Unix ms — when the PR landed.
    pub landed_at: u64,
}

/// Efficiency figures (ADR-0001 §2.3 `efficiency`). `overhead_pct`
/// (orchestration ÷ total) is how you tell a lean fleet from a bloated one.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Efficiency {
    /// orchestration ÷ total.
    pub overhead_pct: f64,
    /// CI saved ÷ would-be.
    pub cache_savings_pct: f64,
    /// Intents landed without rework.
    pub first_pass_yield: f64,
    /// Cost per net kLOC landed.
    pub cost_per_net_kloc: f64,
}

/// The PR record (ADR-0001 §2.3) — NOT just a sum: work (subagents) + the
/// coordination the PR author spent on top + verification + CI, with waste
/// shown. The forge computes this record; it is not stored per envelope.
/// Carries `envelope_ref` → the PR's OWN captured [`ContextEnvelope`]
/// (`altitude:"pr"`, owner 2026-06-10).
///
/// Derived shape (forge-computed, WP-F3) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrRecord {
    /// PR identifier.
    pub pr_id: String,
    /// The PR's author — orchestrator or human, never a subagent (D14).
    pub author: PrAuthor,
    /// The intent (commit) ids bundled in this PR.
    pub intent_ids: Vec<String>,
    /// Number of intents in the bundle.
    pub intent_count: u64,
    /// Number of distinct agent runs that authored the intents.
    pub agent_count: u64,
    /// Models used across the bundle.
    pub models_used: Vec<String>,
    /// `cas:` ref to the PR's own captured envelope
    /// ([`ContextEnvelope`] with `altitude:"pr"`): the orchestrator session
    /// that planned/dispatched/landed this bundle. If one session authors
    /// several PRs, each PR refs the same session blob (CAS dedupes).
    pub envelope_ref: String,
    /// Cost decomposed by where it went.
    pub cost: CostDecomposition,
    /// The PR's time block (span vs sum vs queue wait).
    pub time: PrTime,
    /// Efficiency figures.
    pub efficiency: Efficiency,
}

/// A campaign's owner — always a human principal, never a subagent
/// (forge-authz invariant D14).
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignOwner {
    /// The human principal who owns the campaign goal.
    pub principal: String,
}

/// The campaign's time block (ADR-0001 §2.3): lead time over the whole
/// bundle of PRs.
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignTime {
    /// Campaign opened → last PR landed (lead time), ms.
    pub wall_span_ms: u64,
    /// Σ agent-time across the campaign, ms.
    pub agent_sum_ms: u64,
    /// Σ landing-queue wait, ms.
    pub queue_wait_ms: u64,
}

/// Campaign completion (ADR-0001 §2.3 `progress`).
///
/// Derived shape (forge-computed) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignProgress {
    /// PRs landed.
    pub landed: u64,
    /// PRs in flight.
    pub in_flight: u64,
    /// PRs blocked.
    pub blocked: u64,
}

/// The campaign rollup (ADR-0001 §2.3) — the third altitude. A campaign is
/// a **bundle of PRs** in the landing queue (NOT a bundle of commits),
/// owned by a human, consolidated the same way a PR consolidates its
/// intents — same cost decomposition, plus campaign progress. Carries
/// `envelope_ref` → the campaign's own captured [`ContextEnvelope`]
/// (`altitude:"campaign"`, owner 2026-06-10).
///
/// Derived shape (forge-computed, WP-F3) — NOT frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignRollup {
    /// Campaign key (the landing-queue bundle key).
    pub campaign: String,
    /// Human-defined goal of the campaign.
    pub charter: String,
    /// The human owner — never a subagent (D14).
    pub owner: CampaignOwner,
    /// `cas:` ref to the campaign's own captured envelope
    /// ([`ContextEnvelope`] with `altitude:"campaign"`).
    pub envelope_ref: String,
    /// The PR ids bundled in this campaign.
    pub pr_ids: Vec<String>,
    /// Number of PRs in the campaign.
    pub pr_count: u64,
    /// Number of intents across all PRs.
    pub intent_count: u64,
    /// Number of distinct agent runs across all PRs.
    pub agent_count: u64,
    /// Models used across the campaign.
    pub models_used: Vec<String>,
    /// Cost decomposed by where it went — Σ over PRs, same vocabulary.
    pub cost: CostDecomposition,
    /// The campaign's time block (lead time).
    pub time: CampaignTime,
    /// Efficiency figures.
    pub efficiency: Efficiency,
    /// Campaign completion.
    pub progress: CampaignProgress,
}
