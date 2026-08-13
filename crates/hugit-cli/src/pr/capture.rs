//! WP-F2 — capture the context envelope on land (cost/metrics legibility).
//!
//! The serve cost/metrics/legibility surfaces (`pr show`'s F3 rollup, the
//! `/v1` read side) read REAL data off `intent.envelope` / `pr.envelope`
//! records — the ADR-0001 [`ContextEnvelope`] with its [`IntentMetrics`]
//! (tokens, `cost_usd_micros`, tool calls, active ms, model turns) +
//! authorship.model + cas/proof refs. Until this module, **nothing in the
//! production land flow wrote those records** — the
//! [`hugit_ledger::envelope`] producer had zero non-test callers, so the cost
//! block was honestly-null everywhere.
//!
//! This module wires the **capture mechanism** into the terminal land step
//! ([`super::settle`], the `hugit pr land` confirm). When a PR settles as
//! LANDED it:
//!
//! 1. builds a `pr`-altitude [`ContextEnvelope`] for the bundle (top-level
//!    author — orchestrator/human, never a subagent: the rollup's D14 rule),
//!    and one `intent`-altitude envelope per bundled intent id, through the
//!    EXISTING frozen [`hugit_ledger::envelope::close_envelope`] builder (the
//!    ADR-0001 schema is frozen — we never invent a new one);
//! 2. appends each closed envelope as the payload of a `pr.envelope` /
//!    `intent.envelope` record, through the SAME D14-guarded
//!    [`hugit_refstore::EventLog::append_authorized`] seam the other `pr.*`
//!    records use (so the append is authorized + hash-chain-valid), under the
//!    PR's own author class.
//!
//! # The honest-zero / dogfood seam
//!
//! Real run metrics ride in through OPTIONAL `--tokens` / `--cost-usd-micros`
//! / `--tool-calls` / `--active-ms` / `--model-turns` / `--model` flags (the
//! dogfood path: our orchestrator passes the figures it measured → the cost
//! surfaces show REAL cost). When a flag is omitted the corresponding metric
//! is **honest-zero** and the model is the empty string (a HUMAN-authored
//! PR per the rollup's `pr_author` rule) — the envelope still captures the
//! REAL structure (which PR, which intents, the campaign, the schema). We
//! never fabricate a cost number: generic per-agent attribution still needs
//! the runner fabric (F7); this is the capture mechanism + the explicit/
//! dogfood metrics path, nothing more.
//!
//! # Capture level — `metrics`, not `full`
//!
//! The porcelain land path has no transcript blobs to store (the agent's
//! born → die transcript is the runner/harness's concern, not the land
//! confirm's). Capturing at [`CaptureLevel::Metrics`] is the honest level:
//! the metrics block is kept, both transcript refs are legally `null` (the
//! two-transcript imperative applies only under `full`). The envelope JSON
//! itself is the record payload the serve read side parses — the cold store
//! only ever holds transcript blobs, of which there are none here.

use hugit_contracts::context_envelope::{Authorship, Spawn, TokenCounts};
use hugit_contracts::model_price_card::{self, PRICE_CARD_VERSION};
use hugit_contracts::{Altitude, IntentMetrics};
use hugit_ledger::envelope::{CaptureLevel, EnvelopeDraft, InMemoryColdStore, close_envelope};
use hugit_refstore::EventLog;

use crate::ctx::CTX_USAGE_KIND;

use super::{INTENT_ENVELOPE_KIND, OpenedPr, PR_ENVELOPE_KIND, author_authz};

/// The complete-or-nothing priced authoring usage for a PR's target set
/// (WP-COST-3, Option A: real tokens × EXACT published rate = the invoice).
///
/// Returned by [`priced_usage`] ONLY when every contributing `ctx.usage` record
/// priced to a real figure — see that function's contract for the honesty rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PricedUsage {
    /// The Σ of the contributing records' token cache-splits (the summed
    /// [`TokenCounts`] the figure was priced from — recorded for reproducibility).
    pub(super) tokens: TokenCounts,
    /// Σ over ALL contributing records of `cost_micros(card, record.model,
    /// record.tokens)`, in integer micro-USD.
    pub(super) cost_usd_micros: u64,
}

/// Fold + price the authoring `ctx.usage` records for a PR's target set
/// (the PR id itself + each bundled intent id) — WP-COST-3, the cost-killer's
/// final link.
///
/// # The honesty rule — COMPLETE-OR-NOTHING (the load-bearing invariant)
///
/// Prices each contributing record by ITS OWN model against the FROZEN
/// [`model_price_card::CURRENT`] card, and returns:
///
/// - `Some(PricedUsage { tokens, cost_usd_micros })` — **only when there is at
///   least one contributing record AND every one of them priced to `Some`**
///   (a known model) AND no `u64` overflow occurred anywhere in the accumulation.
///   `cost_usd_micros` is the EXACT `Σ(usage × published rate)`; `tokens` is the
///   summed cache-split it was priced from.
/// - `None` — honest-zero — when there are **no** contributing records, OR **any**
///   contributing record's model is unknown to the card (`cost_micros → None`),
///   OR a matching record is malformed, OR the accumulation overflows. NEVER a
///   partial / under-counted sum, NEVER an estimate, NEVER the modeled
///   [`IntentMetrics`] COGS (a partial cost would be UNTRUE for the intent — the
///   #113 per-PR honesty law forbids it).
///
/// A record contributes iff it is a `ctx.usage` record whose
/// `(target_kind, target_id)` is either `("pr", opened.pr_id)` or `("intent",
/// one of opened.intent_ids)`. Records for other targets are ignored. The
/// append-only ACCUMULATE rule holds: multiple records for one target each
/// contribute (WP-COST-2 never dedups; land sums them here).
pub(super) fn priced_usage(log: &EventLog, opened: &OpenedPr) -> Option<PricedUsage> {
    use serde_json::Value;

    let intents: std::collections::BTreeSet<&str> =
        opened.intent_ids.iter().map(String::as_str).collect();

    let mut contributing = 0u64;
    let mut cost: u64 = 0;
    let mut sum = TokenCounts {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        total: 0,
    };

    for r in log.records().iter().filter(|r| r.kind == CTX_USAGE_KIND) {
        let Ok(v) = serde_json::from_str::<Value>(&r.payload) else {
            // A malformed ctx.usage payload we cannot even parse is not a
            // matching contributor — skip it (an id/target we can't read cannot
            // belong to this PR's target set). Fail-closed happens below for a
            // record that DOES match but is malformed.
            continue;
        };
        let target_id = v
            .get("target_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let target_kind = v
            .get("target_kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let contributes = (target_kind == "pr" && target_id == opened.pr_id)
            || (target_kind == "intent" && intents.contains(target_id));
        if !contributes {
            continue;
        }

        // A CONTRIBUTING record that is malformed (no tokens block / a missing
        // component field) → fail-closed to None for the WHOLE total (never a
        // partial figure). `?` here short-circuits the whole function.
        let t = v.get("tokens")?;
        let rec = TokenCounts {
            input: t.get("input").and_then(Value::as_u64)?,
            output: t.get("output").and_then(Value::as_u64)?,
            cache_read: t.get("cache_read").and_then(Value::as_u64)?,
            cache_write: t.get("cache_write").and_then(Value::as_u64)?,
            // total is the TokenCounts identity — recomputed (checked) below
            // rather than trusting a possibly-absent stored field.
            total: 0,
        };
        let model = v.get("model").and_then(Value::as_str).unwrap_or_default();

        // Price by THIS record's OWN model. Unknown model → None → whole None
        // (honest-zero; NEVER a fallback/nearest rate). Overflow inside
        // cost_micros is likewise None.
        let c = model_price_card::cost_micros(&model_price_card::CURRENT, model, &rec)?;
        cost = cost.checked_add(c)?;

        sum.input = sum.input.checked_add(rec.input)?;
        sum.output = sum.output.checked_add(rec.output)?;
        sum.cache_read = sum.cache_read.checked_add(rec.cache_read)?;
        sum.cache_write = sum.cache_write.checked_add(rec.cache_write)?;
        contributing += 1;
    }

    if contributing == 0 {
        // No contributing records → honest-zero (None), never a fabricated figure.
        return None;
    }

    // The summed cache-split total (checked — overflow ⇒ None, never wrong).
    sum.total = sum
        .input
        .checked_add(sum.output)?
        .checked_add(sum.cache_read)?
        .checked_add(sum.cache_write)?;

    Some(PricedUsage {
        tokens: sum,
        cost_usd_micros: cost,
    })
}

/// The agent type that marks a top-level (orchestrator / human) authored unit
/// — the rollup's D14 gate (`agent_type == "main"`, no `parent_run_id`).
/// Restated here as the porcelain's read-side mirror of the rollup constant so
/// the captured envelope is provably top-level (never a subagent).
const TOP_LEVEL_AGENT_TYPE: &str = "main";

/// The real run metrics an orchestrator may carry into the captured envelope
/// (the dogfood / explicit-metrics path). Every field is OPTIONAL — an omitted
/// flag is honest-zero (the ref / model is `None` / empty), never a fabricated
/// figure. Plumbed from the `hugit pr land` CLI flags.
///
/// When a runner can supply a complete [`IntentMetrics`] (the §13.1 cache-split
/// path), set [`EnvelopeMetricsArgs::full_metrics`] instead of the individual
/// flag fields. [`build_metrics`] passes it through verbatim — preserving
/// `input / output / cache_read / cache_write / total` exactly as the runner
/// measured them. The flag-derived fields are still used when `full_metrics` is
/// `None` (honest-zero defaults for the manual CLI path).
#[derive(Debug, Clone, Default)]
pub struct EnvelopeMetricsArgs {
    /// Total tokens spent on the run (`--tokens`).
    pub tokens: u64,
    /// Derived COGS in integer micro-USD (`--cost-usd-micros`).
    pub cost_usd_micros: u64,
    /// Total tool calls (`--tool-calls`).
    pub tool_calls: u64,
    /// Model + tool busy time, ms (`--active-ms`).
    pub active_ms: u64,
    /// Number of model turns (`--model-turns`).
    pub model_turns: u64,
    /// Model identifier (`--model`); empty ⇒ a HUMAN-authored PR (rollup rule).
    pub model: Option<String>,
    /// `cas:` ref to the context blob, when the orchestrator captured one
    /// (`--context-cas`). Carried on the intent envelopes' `commit` is NOT —
    /// this is a pass-through ref stored in the snapshot env manifest's place
    /// is also NOT; see [`build_metrics`]. Reserved for the F7 wiring; today
    /// it rides as `None` unless supplied.
    pub context_cas: Option<String>,
    /// `cas:` ref to the compacted transcript, when available
    /// (`--compact-transcript-ref`). Honest-`None` at the porcelain altitude.
    pub compact_transcript_ref: Option<String>,
    /// `cas:` ref to the raw transcript, when available
    /// (`--raw-transcript-ref`). Honest-`None` at the porcelain altitude.
    pub raw_transcript_ref: Option<String>,
    /// `cas:` ref to the adversarial-panel verdicts (`--verdicts-ref`).
    pub verdicts_ref: Option<String>,
    /// A complete [`IntentMetrics`] supplied by the runner (the §13.1
    /// cache-split carrier). When `Some`, [`build_metrics`] passes it through
    /// verbatim — `input / output / cache_read / cache_write / total` all
    /// survive unchanged. When `None`, the flag-derived path is used instead
    /// (honest-zero split, `total` from `self.tokens`). The CLI never sets
    /// this field (no CLI flag exists for a full metrics blob); it is wired
    /// by the runner integration in a later PR.
    pub full_metrics: Option<IntentMetrics>,
}

impl EnvelopeMetricsArgs {
    /// Whether the orchestrator supplied any real metric (vs the all-default
    /// honest-zero case). Used only for diagnostics in the settle result.
    #[must_use]
    pub fn any_supplied(&self) -> bool {
        self.tokens != 0
            || self.cost_usd_micros != 0
            || self.tool_calls != 0
            || self.active_ms != 0
            || self.model_turns != 0
            || self.model.as_deref().is_some_and(|m| !m.is_empty())
            || self.context_cas.is_some()
            || self.compact_transcript_ref.is_some()
            || self.raw_transcript_ref.is_some()
            || self.verdicts_ref.is_some()
            || self.full_metrics.is_some()
    }
}

/// Build the [`IntentMetrics`] from the supplied args.
///
/// Two paths:
/// - **Full-metrics path** (`m.full_metrics` is `Some`): the caller supplied a
///   complete [`IntentMetrics`] (e.g. the §13.1 runner carrier). It is passed
///   through **verbatim** — `input / output / cache_read / cache_write / total`
///   all survive unchanged. This is the cache-split-preserving path.
/// - **Flag-derived path** (`m.full_metrics` is `None`): the orchestrator
///   supplied individual flags (or none). Tokens arrive as a single aggregate
///   (`--tokens`); the cache split is not a porcelain seam, so `total` carries
///   it and the split fields are honest-zero. An omitted flag is honest-zero —
///   never a fabricated figure. `tool_breakdown` stays empty (the porcelain has
///   no per-tool seam; the aggregate `tool_calls` is the honest figure).
fn build_metrics(m: &EnvelopeMetricsArgs) -> IntentMetrics {
    // Full-metrics path: pass through verbatim, preserving the cache split.
    if let Some(ref full) = m.full_metrics {
        return full.clone();
    }
    // Flag-derived path: honest-zero for the cache split, total from --tokens.
    IntentMetrics {
        // Tokens arrive as a single aggregate (`--tokens`); the cache split is
        // not a porcelain seam, so `total` carries it and the split is zero
        // (honest: we know the total the orchestrator measured, not the split).
        tokens: TokenCounts {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            total: m.tokens,
        },
        // No wall-clock seam at the land confirm; active_ms is the supplied
        // busy time. wall_ms stays zero (honest — not measured here).
        wall_ms: 0,
        active_ms: m.active_ms,
        tool_calls: m.tool_calls,
        tool_breakdown: vec![],
        model_turns: m.model_turns,
        cost_usd_micros: m.cost_usd_micros,
    }
}

/// The authorship for a captured land-time envelope. **Top-level by
/// construction** (`agent_type == "main"`, no `parent_run_id`) so the F3
/// rollup's D14 gate accepts it: a PR author is an orchestrator or a human,
/// never a subagent. `model` non-empty ⇒ orchestrator; empty ⇒ human (the
/// rollup's `pr_author` rule).
fn land_authorship(opened: &OpenedPr, m: &EnvelopeMetricsArgs) -> Authorship {
    let model = m.model.clone().unwrap_or_default();
    Authorship {
        model,
        model_digest: String::new(),
        agent_type: TOP_LEVEL_AGENT_TYPE.to_string(),
        spawn: Spawn {
            // The PR id is the stable run handle at the land altitude (the
            // orchestrator run id is an optional refinement the runner fabric
            // supplies; absent it, the PR id is the honest, non-fabricated id).
            run_id: format!("land-{}", opened.pr_id),
            parent_run_id: None,
            born_at: 0,
            died_at: 0,
        },
        // The human principal who dispatched the land (the PR's campaign is the
        // closest non-fabricated operator handle we have at this altitude).
        operator: String::new(),
    }
}

/// Build the `pr`-altitude envelope draft for a settled PR. The PR's bundle is
/// its acceptance/parent set; the metrics are the supplied (or honest-zero)
/// figures.
fn pr_draft(opened: &OpenedPr, m: &EnvelopeMetricsArgs) -> EnvelopeDraft {
    EnvelopeDraft {
        altitude: Altitude::Pr,
        unit_id: opened.pr_id.clone(),
        // No materialized tree at the porcelain altitude (the P2 live-infra
        // seam) — honest empty, never a fabricated sha.
        commit: m.context_cas.clone().unwrap_or_default(),
        tree_hash: String::new(),
        authorship: land_authorship(opened, m),
        charter: format!("land PR {} (campaign {})", opened.pr_id, opened.campaign),
        campaign: Some(opened.campaign.clone()),
        constraints: vec![],
        acceptance: vec![],
        // The PR's bundled intents are its parents at the PR altitude.
        parent_intents: opened.intent_ids.clone(),
        // No transcript blobs at this altitude (capture level is `metrics`).
        raw_transcript: vec![],
        task_transcript: vec![],
        // Pre-existing CAS refs from CLI flags (bypass cold store).
        raw_transcript_ref: m.raw_transcript_ref.clone(),
        task_transcript_ref: m.compact_transcript_ref.clone(),
        summary: String::new(),
        journal_ref: None,
        files_read: vec![],
        prompt: None,
        env_manifest: String::new(),
        metrics: build_metrics(m),
        verdicts_ref: m.verdicts_ref.clone(),
    }
}

/// Build an `intent`-altitude envelope draft for one bundled intent id. The
/// intent envelopes carry the SAME honest-zero-or-supplied metrics structure
/// (per-intent attribution is the F7 runner-fabric seam; here every bundled
/// intent shares the PR's supplied figure, or honest-zero).
fn intent_draft(opened: &OpenedPr, intent_id: &str, m: &EnvelopeMetricsArgs) -> EnvelopeDraft {
    EnvelopeDraft {
        altitude: Altitude::Intent,
        unit_id: intent_id.to_string(),
        commit: String::new(),
        tree_hash: String::new(),
        authorship: Authorship {
            model: m.model.clone().unwrap_or_default(),
            model_digest: String::new(),
            // An intent is subagent-authored — `implementer`, not `main`. The
            // rollup attributes intent envelopes to the PR by `intent_id`, it
            // does NOT apply the top-level gate to them (only to the PR
            // envelope), so the subagent agent_type is the honest shape here.
            agent_type: "implementer".to_string(),
            spawn: Spawn {
                run_id: format!("land-{}-{intent_id}", opened.pr_id),
                parent_run_id: Some(format!("land-{}", opened.pr_id)),
                born_at: 0,
                died_at: 0,
            },
            operator: String::new(),
        },
        charter: format!("intent {intent_id} of PR {}", opened.pr_id),
        campaign: Some(opened.campaign.clone()),
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        raw_transcript: vec![],
        task_transcript: vec![],
        // Pre-existing CAS refs from CLI flags (bypass cold store).
        raw_transcript_ref: m.raw_transcript_ref.clone(),
        task_transcript_ref: m.compact_transcript_ref.clone(),
        summary: String::new(),
        journal_ref: None,
        files_read: vec![],
        prompt: None,
        env_manifest: String::new(),
        // Per-intent attribution is the F7 seam — every bundled intent carries
        // honest-zero metrics unless the orchestrator supplied a PR-level
        // figure, which is attributed to the PR envelope, NOT split here. So an
        // intent's own metrics are honest-zero (never a fabricated split).
        metrics: build_metrics(&EnvelopeMetricsArgs {
            // Model rides through (it is the same run's model), but the cost/
            // token figures are NOT split across intents — honest-zero each.
            model: m.model.clone(),
            ..Default::default()
        }),
        verdicts_ref: None,
    }
}

/// Capture the context envelope(s) for a settled PR and append them to the
/// canonical log through the D14-guarded append.
///
/// Fires from [`super::settle`] AFTER the `pr.landed` record is appended (the
/// settlement is the trigger; the envelope is the legibility record that rides
/// alongside it). Appends, in order:
///
/// 1. one `intent.envelope` record per bundled intent id (subagent-authored —
///    the rollup attributes them to the PR by `intent_id`);
/// 2. one `pr.envelope` record (top-level / D14-clean — the rollup's PR-author
///    gate accepts it).
///
/// Each record's payload is the frozen [`ContextEnvelope`] JSON the serve read
/// side ([`super::envelopes_for_pr`]) parses. Capture is at
/// [`CaptureLevel::Metrics`]: the metrics block is kept, transcript refs are
/// legally `null` (no porcelain transcript blobs), so the two-transcript
/// imperative — a `full`-only invariant — never trips.
///
/// The append is best-effort wrt the settlement: it runs on a settlement that
/// has ALREADY succeeded, so a capture-append failure does not unwind the
/// land. It returns the count of records appended (for the settle result's
/// diagnostics). An append denial (impossible for the PR's own author class)
/// or an envelope-close failure (unreachable at `metrics` with empty
/// transcripts) is swallowed into a zero count rather than aborting a
/// completed land — the cost block being absent is the honest degradation,
/// never a failed land.
pub fn capture_on_land(
    log: &mut EventLog,
    opened: &OpenedPr,
    m: &EnvelopeMetricsArgs,
    recorded_at: u64,
) -> u64 {
    // The cold store only ever holds transcript blobs; at `metrics` level there
    // are none, so an in-memory store that drops on return is exactly right —
    // the envelope JSON (the record payload) is what persists on the log.
    let store = InMemoryColdStore::new();
    let (class, endpoint) = author_authz(opened.author_kind);
    let mut appended: u64 = 0;

    // WP-COST-3 — the non-dispatch land path prices the authoring `ctx.usage`
    // records (Option A, complete-or-nothing) into the captured envelope so the
    // land records the REAL measured cost, not honest-zero.
    //
    // Gated fail-safe:
    // - `m.full_metrics.is_none()` — the `--dispatch` path is runner-authoritative
    //   (the fabric's §13.1 readback rides in via `full_metrics`, and the REAL cost
    //   is submitted + recorded on the close), so we NEVER re-price or clobber it
    //   here (that would double-attribute); and
    // - `m.tokens == 0 && m.cost_usd_micros == 0` — an operator who hand-typed the
    //   manual `--tokens` / `--cost-usd-micros` dogfood figures keeps them verbatim
    //   (we only FILL IN the real usage cost when the metrics were left honest-zero).
    //
    // When [`priced_usage`] returns `None` (no records / any unknown model /
    // overflow) the capture is byte-identical to today (honest-zero). When it
    // returns `Some`, the PR envelope carries the EXACT `Σ(usage × published rate)`
    // in `cost_usd_micros` (its ADR-0001 "derived COGS" meaning — the REAL figure,
    // never the modeled COGS) + the summed cache-split, and the price-card snapshot
    // version is stamped for reproducibility.
    let priced = if m.full_metrics.is_none() && m.tokens == 0 && m.cost_usd_micros == 0 {
        priced_usage(log, opened)
    } else {
        None
    };
    let owned_metrics;
    let m: &EnvelopeMetricsArgs = match &priced {
        Some(pu) => {
            owned_metrics = EnvelopeMetricsArgs {
                full_metrics: Some(IntentMetrics {
                    tokens: pu.tokens.clone(),
                    // No wall-clock at the land confirm; carry the operator's
                    // (honest-zero) non-cost fields verbatim.
                    wall_ms: 0,
                    active_ms: m.active_ms,
                    tool_calls: m.tool_calls,
                    tool_breakdown: vec![],
                    model_turns: m.model_turns,
                    cost_usd_micros: pu.cost_usd_micros,
                }),
                ..m.clone()
            };
            &owned_metrics
        }
        None => m,
    };

    // 1. one intent.envelope per bundled intent (subagent-authored).
    for intent_id in &opened.intent_ids {
        let draft = intent_draft(opened, intent_id, m);
        let Ok(closed) = close_envelope(&draft, CaptureLevel::Metrics, &store) else {
            continue;
        };
        let Ok(payload) = serde_json::to_string(&closed.envelope) else {
            continue;
        };
        if log
            .append_authorized(
                class,
                endpoint,
                INTENT_ENVELOPE_KIND,
                vec![],
                payload,
                recorded_at,
            )
            .is_ok()
        {
            appended += 1;
        }
    }

    // 2. the pr.envelope (top-level / D14-clean — the rollup gate accepts it).
    let mut draft = pr_draft(opened, m);
    // WP-COST-3: when the cost is a REAL priced-from-usage figure, stamp the
    // price-card snapshot version onto the (free-text) env manifest so a rendered
    // cost is reproducible (which price snapshot produced it). Only when priced —
    // an honest-zero capture leaves the manifest empty, byte-identical to today.
    if priced.is_some() {
        draft.env_manifest = format!("price_card={PRICE_CARD_VERSION}");
    }
    if let Ok(closed) = close_envelope(&draft, CaptureLevel::Metrics, &store)
        && let Ok(payload) = serde_json::to_string(&closed.envelope)
        && log
            .append_authorized(
                class,
                endpoint,
                PR_ENVELOPE_KIND,
                vec![],
                payload,
                recorded_at,
            )
            .is_ok()
    {
        appended += 1;
    }

    appended
}

/// Read the captured `pr.envelope` for a PR id straight off the log (the
/// round-trip the acceptance tests assert over). Returns the parsed frozen
/// envelope, or `None` if no `pr.envelope` names this PR.
#[cfg(test)]
pub(super) fn captured_pr_envelope(
    log: &EventLog,
    pr_id: &str,
) -> Option<hugit_contracts::context_envelope::ContextEnvelope> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| {
            serde_json::from_str::<hugit_contracts::context_envelope::ContextEnvelope>(&r.payload)
                .ok()
        })
        .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == pr_id)
}

/// Read the captured `intent.envelope` records for a PR's bundle off the log.
#[cfg(test)]
pub(super) fn captured_intent_envelopes(
    log: &EventLog,
    intent_ids: &[String],
) -> Vec<hugit_contracts::context_envelope::ContextEnvelope> {
    let bundle: std::collections::BTreeSet<&str> = intent_ids.iter().map(String::as_str).collect();
    log.records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| {
            serde_json::from_str::<hugit_contracts::context_envelope::ContextEnvelope>(&r.payload)
                .ok()
        })
        .filter(|e| e.altitude == Altitude::Intent && bundle.contains(e.intent_id.as_str()))
        .collect()
}

/// Validate that a `Value` is the metrics-bearing intent/pr envelope shape the
/// serve read side expects — used to confirm the appended record is the frozen
/// shape (not an opaque blob). Returns the parsed envelope.
#[cfg(test)]
pub(super) fn parse_envelope(
    v: &serde_json::Value,
) -> Option<hugit_contracts::context_envelope::ContextEnvelope> {
    serde_json::from_value(v.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::{LandArgs, OpenArgs, SettleArgs, land, open, settle};
    use hugit_ledger::rollup::{PrQueueInput, pr_record};
    use hugit_refstore::EventLog;
    use serde_json::json;

    /// Open + queue a PR, then settle it (the full land path) with the given
    /// metrics. Returns the log + the settle result value.
    fn open_queue_settle(metrics: EnvelopeMetricsArgs) -> (EventLog, serde_json::Value) {
        let mut log = EventLog::new();
        open(
            &mut log,
            &OpenArgs {
                pr_id: "42".to_string(),
                campaign: "camp-f2".to_string(),
                author_kind: crate::pr::AuthorKind::Orchestrator,
                run_id: Some("run-orq".to_string()),
                principal: None,
                intent_ids: vec!["i-a".to_string(), "i-b".to_string()],
                recorded_at: 1000,
            },
        )
        .expect("open");
        land(
            &mut log,
            &LandArgs {
                pr_id: "42".to_string(),
                recorded_at: 2000,
            },
        )
        .expect("queue");
        let out = settle(
            &mut log,
            &SettleArgs {
                pr_id: "42".to_string(),
                recorded_at: 3000,
                envelope_metrics: metrics,
            },
        )
        .expect("settle");
        (log, out)
    }

    /// Append an authoring `ctx.usage` record onto `log` the SAME way
    /// `hugit ctx usage` (WP-COST-2) does — Orchestrator/Land, canonical payload.
    #[allow(clippy::too_many_arguments)]
    fn append_usage(
        log: &mut EventLog,
        target_kind: &str,
        target_id: &str,
        model: &str,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) {
        use hugit_refstore::{Endpoint, PrincipalClass};
        let total = input + output + cache_read + cache_write;
        let payload = json!({
            "target_id": target_id,
            "target_kind": target_kind,
            "source": "provider_usage",
            "model": model,
            "recorded_at": 0,
            "tokens": {
                "input": input, "output": output,
                "cache_read": cache_read, "cache_write": cache_write, "total": total,
            },
        })
        .to_string();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            CTX_USAGE_KIND,
            vec!["orchestrator:hugit".to_string()],
            payload,
            0,
        )
        .expect("ctx.usage append (Orchestrator/Land) is authorized");
    }

    /// The projected `OpenedPr` for the standard "42" bundle used by the
    /// `priced_usage` unit tests (intents i-a / i-b).
    fn opened_42(log: &EventLog) -> OpenedPr {
        crate::pr::find_pr_opened(log, "42").expect("pr.opened for 42")
    }

    /// WP-COST-3: a non-dispatch land with authoring `ctx.usage` records (all
    /// models known) captures the REAL priced cost + the summed cache-split in the
    /// PR envelope, and stamps the price-card version for reproducibility.
    ///
    /// - i-a, opus-4-8, 1_000_000 input  → $5.00  = 5_000_000 µ$
    /// - i-b, sonnet-4-6, 1_000_000 output → $15.00 = 15_000_000 µ$
    ///
    /// Σ = 20_000_000 micro-USD; summed tokens input=1M, output=1M, total=2M.
    #[test]
    fn non_dispatch_land_prices_ctx_usage_into_envelope() {
        let mut log = EventLog::new();
        open(
            &mut log,
            &OpenArgs {
                pr_id: "42".to_string(),
                campaign: "camp-f2".to_string(),
                author_kind: crate::pr::AuthorKind::Orchestrator,
                run_id: Some("run-orq".to_string()),
                principal: None,
                intent_ids: vec!["i-a".to_string(), "i-b".to_string()],
                recorded_at: 1000,
            },
        )
        .expect("open");
        land(
            &mut log,
            &LandArgs {
                pr_id: "42".to_string(),
                recorded_at: 2000,
            },
        )
        .expect("queue");
        append_usage(
            &mut log,
            "intent",
            "i-a",
            "claude-opus-4-8",
            1_000_000,
            0,
            0,
            0,
        );
        append_usage(
            &mut log,
            "intent",
            "i-b",
            "claude-sonnet-4-6",
            0,
            1_000_000,
            0,
            0,
        );
        // Honest-zero metrics args (no manual flags) — the usage pricing fills in.
        settle(
            &mut log,
            &SettleArgs {
                pr_id: "42".to_string(),
                recorded_at: 3000,
                envelope_metrics: EnvelopeMetricsArgs::default(),
            },
        )
        .expect("settle");

        let pr_env = captured_pr_envelope(&log, "42").expect("pr.envelope on log");
        assert_eq!(
            pr_env.metrics.cost_usd_micros, 20_000_000,
            "the REAL Σ(usage × published rate) is captured, not honest-zero"
        );
        // The summed cache-split rides in (reproducibility of the figure).
        assert_eq!(pr_env.metrics.tokens.input, 1_000_000);
        assert_eq!(pr_env.metrics.tokens.output, 1_000_000);
        assert_eq!(pr_env.metrics.tokens.total, 2_000_000);
        // The price-card snapshot version is stamped for reproducibility.
        assert_eq!(
            pr_env.snapshot.env_manifest,
            format!("price_card={PRICE_CARD_VERSION}")
        );
    }

    /// WP-COST-3 COMPLETE-OR-NOTHING (non-dispatch): ANY unknown model ⇒ the
    /// captured envelope is honest-zero (never the partial known-model sum), and
    /// the capture is byte-identical to today (no price-card stamp).
    #[test]
    fn non_dispatch_land_unknown_model_is_honest_zero() {
        let mut log = EventLog::new();
        open(
            &mut log,
            &OpenArgs {
                pr_id: "42".to_string(),
                campaign: "camp-f2".to_string(),
                author_kind: crate::pr::AuthorKind::Orchestrator,
                run_id: Some("run-orq".to_string()),
                principal: None,
                intent_ids: vec!["i-a".to_string(), "i-b".to_string()],
                recorded_at: 1000,
            },
        )
        .expect("open");
        land(
            &mut log,
            &LandArgs {
                pr_id: "42".to_string(),
                recorded_at: 2000,
            },
        )
        .expect("queue");
        append_usage(
            &mut log,
            "intent",
            "i-a",
            "claude-opus-4-8",
            1_000_000,
            0,
            0,
            0,
        );
        append_usage(&mut log, "intent", "i-b", "gpt-4o", 1_000_000, 0, 0, 0);
        settle(
            &mut log,
            &SettleArgs {
                pr_id: "42".to_string(),
                recorded_at: 3000,
                envelope_metrics: EnvelopeMetricsArgs::default(),
            },
        )
        .expect("settle");

        let pr_env = captured_pr_envelope(&log, "42").expect("pr.envelope on log");
        assert_eq!(
            pr_env.metrics.cost_usd_micros, 0,
            "unknown model → honest-zero WHOLE, never the partial known sum"
        );
        assert_eq!(pr_env.metrics.tokens.total, 0, "honest-zero tokens");
        assert_eq!(
            pr_env.snapshot.env_manifest, "",
            "no price-card stamp on an honest-zero capture (byte-identical to today)"
        );
    }

    /// WP-COST-3: a manual `--cost-usd-micros` / `--tokens` figure is NEVER
    /// overridden by usage pricing (the operator's explicit dogfood figure wins;
    /// usage pricing only FILLS IN a left-honest-zero capture).
    #[test]
    fn non_dispatch_manual_cost_flag_is_not_overridden_by_usage() {
        let mut log = EventLog::new();
        open(
            &mut log,
            &OpenArgs {
                pr_id: "42".to_string(),
                campaign: "camp-f2".to_string(),
                author_kind: crate::pr::AuthorKind::Orchestrator,
                run_id: Some("run-orq".to_string()),
                principal: None,
                intent_ids: vec!["i-a".to_string()],
                recorded_at: 1000,
            },
        )
        .expect("open");
        land(
            &mut log,
            &LandArgs {
                pr_id: "42".to_string(),
                recorded_at: 2000,
            },
        )
        .expect("queue");
        append_usage(
            &mut log,
            "intent",
            "i-a",
            "claude-opus-4-8",
            1_000_000,
            0,
            0,
            0,
        );
        settle(
            &mut log,
            &SettleArgs {
                pr_id: "42".to_string(),
                recorded_at: 3000,
                envelope_metrics: EnvelopeMetricsArgs {
                    cost_usd_micros: 777,
                    tokens: 42,
                    ..Default::default()
                },
            },
        )
        .expect("settle");

        let pr_env = captured_pr_envelope(&log, "42").expect("pr.envelope on log");
        assert_eq!(
            pr_env.metrics.cost_usd_micros, 777,
            "the explicit manual cost flag wins over usage pricing"
        );
        assert_eq!(pr_env.metrics.tokens.total, 42, "the manual --tokens wins");
        assert_eq!(
            pr_env.snapshot.env_manifest, "",
            "no price-card stamp when the manual path was used"
        );
    }

    /// WP-COST-3 `priced_usage` unit: no contributing records → None (honest-zero).
    #[test]
    fn priced_usage_no_records_is_none() {
        let (log, _out) = open_queue_settle(EnvelopeMetricsArgs::default());
        assert_eq!(priced_usage(&log, &opened_42(&log)), None);
    }

    /// WP-COST-3 `priced_usage` unit: overflow in the Σ → None (never a panic,
    /// never a wrong number). Two opus records each at u64::MAX input tokens
    /// overflow the checked accumulation.
    #[test]
    fn priced_usage_overflow_is_none_not_panic() {
        let mut log = EventLog::new();
        open(
            &mut log,
            &OpenArgs {
                pr_id: "42".to_string(),
                campaign: "camp-f2".to_string(),
                author_kind: crate::pr::AuthorKind::Orchestrator,
                run_id: Some("run-orq".to_string()),
                principal: None,
                intent_ids: vec!["i-a".to_string(), "i-b".to_string()],
                recorded_at: 1000,
            },
        )
        .expect("open");
        append_usage(
            &mut log,
            "intent",
            "i-a",
            "claude-opus-4-8",
            u64::MAX,
            0,
            0,
            0,
        );
        append_usage(
            &mut log,
            "intent",
            "i-b",
            "claude-opus-4-8",
            u64::MAX,
            0,
            0,
            0,
        );
        assert_eq!(
            priced_usage(&log, &opened_42(&log)),
            None,
            "overflow → None, never a panic or a wrong number"
        );
    }

    /// WP-COST-3 `priced_usage` unit: records for OTHER targets are ignored (they
    /// do not contribute to this PR's priced cost).
    #[test]
    fn priced_usage_ignores_foreign_targets() {
        let mut log = EventLog::new();
        open(
            &mut log,
            &OpenArgs {
                pr_id: "42".to_string(),
                campaign: "camp-f2".to_string(),
                author_kind: crate::pr::AuthorKind::Orchestrator,
                run_id: Some("run-orq".to_string()),
                principal: None,
                intent_ids: vec!["i-a".to_string()],
                recorded_at: 1000,
            },
        )
        .expect("open");
        // A record for an intent NOT in this PR's bundle → ignored.
        append_usage(
            &mut log,
            "intent",
            "other",
            "claude-opus-4-8",
            1_000_000,
            0,
            0,
            0,
        );
        assert_eq!(
            priced_usage(&log, &opened_42(&log)),
            None,
            "a foreign target contributes nothing (no in-bundle records → None)"
        );
        // Add an in-bundle record → now Some, counting ONLY the in-bundle one.
        append_usage(
            &mut log,
            "intent",
            "i-a",
            "claude-opus-4-8",
            1_000_000,
            0,
            0,
            0,
        );
        let priced = priced_usage(&log, &opened_42(&log)).expect("an in-bundle record prices");
        assert_eq!(priced.cost_usd_micros, 5_000_000);
        assert_eq!(priced.tokens.input, 1_000_000, "only the in-bundle tokens");
    }

    /// (a) Landing WITH the metric flags appends an `intent.envelope` /
    /// `pr.envelope` whose `ContextEnvelope` carries EXACTLY those metrics
    /// (round-trip the record off the log, assert the `IntentMetrics` fields).
    #[test]
    fn land_with_metrics_captures_envelope_with_those_metrics() {
        let metrics = EnvelopeMetricsArgs {
            tokens: 12_345,
            cost_usd_micros: 6_700_000,
            tool_calls: 9,
            active_ms: 4_200,
            model_turns: 7,
            model: Some("claude-opus-4-8".to_string()),
            ..Default::default()
        };
        let (log, out) = open_queue_settle(metrics);

        // The settle result reports the count: 2 intents + 1 pr = 3.
        assert_eq!(out["envelopes_captured"], json!(3), "captured 3: {out}");

        // The PR envelope round-trips off the log and carries EXACTLY the
        // supplied metrics.
        let pr_env = captured_pr_envelope(&log, "42").expect("pr.envelope on log");
        assert_eq!(pr_env.altitude, Altitude::Pr);
        assert_eq!(pr_env.intent_id, "42");
        assert_eq!(pr_env.metrics.tokens.total, 12_345);
        assert_eq!(pr_env.metrics.cost_usd_micros, 6_700_000);
        assert_eq!(pr_env.metrics.tool_calls, 9);
        assert_eq!(pr_env.metrics.active_ms, 4_200);
        assert_eq!(pr_env.metrics.model_turns, 7);
        assert_eq!(pr_env.authorship.model, "claude-opus-4-8");
        assert_eq!(
            pr_env.campaign.as_deref(),
            Some("camp-f2"),
            "the real campaign structure is captured"
        );

        // The intent envelopes are present, one per bundled intent.
        let intents = captured_intent_envelopes(&log, &["i-a".to_string(), "i-b".to_string()]);
        assert_eq!(intents.len(), 2, "one intent.envelope per bundled intent");
        let ids: std::collections::BTreeSet<&str> =
            intents.iter().map(|e| e.intent_id.as_str()).collect();
        assert_eq!(
            ids,
            ["i-a", "i-b"].into_iter().collect(),
            "the REAL bundled intent ids are captured"
        );

        // The captured envelopes drive a REAL F3 cost rollup (the serve read
        // path) — proving the metrics are legible end-to-end, not just stored.
        let record = pr_record(
            &pr_env,
            "",
            &intents,
            &["i-a".to_string(), "i-b".to_string()],
            &[],
            hugit_contracts::context_envelope::CiCost {
                cache_hit: 0,
                exec: 0,
                cost_usd_micros: 0,
                saved_usd_micros: 0,
            },
            PrQueueInput::default(),
        )
        .expect("a top-level PR envelope drives the rollup (D14-clean)");
        assert_eq!(
            record.cost.orchestration.tokens, 12_345,
            "the supplied tokens surface as orchestration cost in the rollup"
        );
        assert_eq!(record.cost.orchestration.cost_usd_micros, 6_700_000);
    }

    /// (b) Landing WITHOUT the flags still appends an envelope with honest-zero
    /// metrics + the REAL intent/model structure (never fabricated).
    #[test]
    fn land_without_metrics_captures_honest_zero_envelope() {
        let (log, out) = open_queue_settle(EnvelopeMetricsArgs::default());
        assert_eq!(out["envelopes_captured"], json!(3));

        let pr_env = captured_pr_envelope(&log, "42").expect("pr.envelope on log");
        // Honest-zero metrics — never a fabricated number.
        assert_eq!(pr_env.metrics.tokens.total, 0);
        assert_eq!(pr_env.metrics.cost_usd_micros, 0);
        assert_eq!(pr_env.metrics.tool_calls, 0);
        assert_eq!(pr_env.metrics.active_ms, 0);
        assert_eq!(pr_env.metrics.model_turns, 0);
        // Empty model ⇒ a HUMAN-authored PR (the rollup's pr_author rule).
        assert_eq!(pr_env.authorship.model, "");
        // But the REAL structure IS captured (which PR, which intents, campaign).
        assert_eq!(pr_env.intent_id, "42");
        assert_eq!(pr_env.campaign.as_deref(), Some("camp-f2"));
        assert_eq!(
            pr_env.parent_intents,
            vec!["i-a".to_string(), "i-b".to_string()],
            "the real bundled intent ids are captured even with no metrics"
        );

        // Honest-zero still drives the rollup (a HUMAN-authored PR is D14-clean).
        let intents = captured_intent_envelopes(&log, &["i-a".to_string(), "i-b".to_string()]);
        let record = pr_record(
            &pr_env,
            "",
            &intents,
            &["i-a".to_string(), "i-b".to_string()],
            &[],
            hugit_contracts::context_envelope::CiCost {
                cache_hit: 0,
                exec: 0,
                cost_usd_micros: 0,
                saved_usd_micros: 0,
            },
            PrQueueInput::default(),
        )
        .expect("an honest-zero human PR envelope drives the rollup");
        assert_eq!(record.cost.work.cost_usd_micros, 0, "honest-zero work cost");
    }

    /// (c) The capture append is AUTHORIZED + chain-valid: the records land on
    /// the hash-chained log through `append_authorized`, and the whole log
    /// verifies after capture (no out-of-band write).
    #[test]
    fn captured_records_are_authorized_and_chain_valid() {
        let (log, _out) = open_queue_settle(EnvelopeMetricsArgs {
            tokens: 100,
            model: Some("m".to_string()),
            ..Default::default()
        });

        // The whole log (including the capture records) is chain-valid. This is
        // an in-test integrity ASSERTION over a log we built through the guarded
        // append (not a read-path load), so it legitimately calls the primitive
        // directly — marked readpath-verify-exempt per the PS-13 guard.
        hugit_refstore::verify_chain(log.records()) // readpath-verify-exempt
            .expect("log chain verifies after capture");

        // The records carry the exact additive kinds the serve read side targets.
        let kinds: Vec<&str> = log.records().iter().map(|r| r.kind.as_str()).collect();
        assert!(
            kinds.contains(&PR_ENVELOPE_KIND),
            "a pr.envelope record is on the log: {kinds:?}"
        );
        assert_eq!(
            kinds.iter().filter(|k| **k == INTENT_ENVELOPE_KIND).count(),
            2,
            "two intent.envelope records are on the log: {kinds:?}"
        );
        // No authz-denied audit record — the PR's own author class is allowed.
        assert!(
            !kinds.contains(&hugit_refstore::authz::AUTHZ_DENIED_KIND),
            "the capture append was authorized, not denied: {kinds:?}"
        );

        // The pr.envelope record's payload parses back to the frozen shape.
        let rec = log
            .records()
            .iter()
            .rfind(|r| r.kind == PR_ENVELOPE_KIND)
            .expect("a pr.envelope record");
        let v: serde_json::Value = serde_json::from_str(&rec.payload).expect("payload is JSON");
        let env = parse_envelope(&v).expect("payload is the frozen ContextEnvelope shape");
        assert_eq!(env.altitude, Altitude::Pr);
        assert_eq!(env.metrics.tokens.total, 100);
    }

    /// The idempotent re-settle captures NO second envelope (capture fires once,
    /// on the first settlement).
    #[test]
    fn re_settle_captures_no_second_envelope() {
        let (mut log, _out) = open_queue_settle(EnvelopeMetricsArgs::default());
        let n_pr_before = log
            .records()
            .iter()
            .filter(|r| r.kind == PR_ENVELOPE_KIND)
            .count();
        let again = settle(
            &mut log,
            &SettleArgs {
                pr_id: "42".to_string(),
                recorded_at: 4000,
                envelope_metrics: EnvelopeMetricsArgs::default(),
            },
        )
        .expect("re-settle");
        assert_eq!(again["already_landed"], json!(true));
        assert_eq!(again["envelopes_captured"], json!(0), "no second capture");
        let n_pr_after = log
            .records()
            .iter()
            .filter(|r| r.kind == PR_ENVELOPE_KIND)
            .count();
        assert_eq!(n_pr_before, n_pr_after, "re-settle appends no pr.envelope");
    }

    /// (e) build_metrics with a full IntentMetrics preserves the cache split
    /// (input / output / cache_read / cache_write / total all survive verbatim,
    /// not zeroed). This is the §13.1 runner-carrier path — the whole point of
    /// this WP.
    #[test]
    fn build_metrics_full_metrics_preserves_cache_split() {
        let full = IntentMetrics {
            tokens: TokenCounts {
                input: 1_000,
                output: 2_000,
                cache_read: 3_000,
                cache_write: 4_000,
                total: 10_000,
            },
            wall_ms: 500,
            active_ms: 300,
            tool_calls: 5,
            tool_breakdown: vec![],
            model_turns: 3,
            cost_usd_micros: 9_876_543,
        };
        let args = EnvelopeMetricsArgs {
            // Flag-derived fields are set but must be IGNORED when full_metrics
            // is Some — the full metrics path is the authoritative one.
            tokens: 99_999,
            cost_usd_micros: 1,
            tool_calls: 1,
            active_ms: 1,
            model_turns: 1,
            full_metrics: Some(full.clone()),
            ..Default::default()
        };
        let built = build_metrics(&args);

        // All cache-split fields must survive verbatim.
        assert_eq!(built.tokens.input, 1_000, "input preserved");
        assert_eq!(built.tokens.output, 2_000, "output preserved");
        assert_eq!(built.tokens.cache_read, 3_000, "cache_read preserved");
        assert_eq!(built.tokens.cache_write, 4_000, "cache_write preserved");
        assert_eq!(built.tokens.total, 10_000, "total preserved");
        assert_eq!(
            built.cost_usd_micros, 9_876_543,
            "cost_usd_micros preserved"
        );
        assert_eq!(built.active_ms, 300, "active_ms preserved");
        assert_eq!(built.tool_calls, 5, "tool_calls preserved");
        assert_eq!(built.model_turns, 3, "model_turns preserved");
        assert_eq!(built.wall_ms, 500, "wall_ms preserved");
    }

    /// (f) The existing flag-only path still yields the same honest-zero result
    /// for cache split fields (input / output / cache_read / cache_write = 0)
    /// when `full_metrics` is `None` — regression guard.
    #[test]
    fn build_metrics_flag_only_path_still_zeroes_cache_split() {
        let args = EnvelopeMetricsArgs {
            tokens: 50_000,
            cost_usd_micros: 3_000_000,
            tool_calls: 7,
            active_ms: 1_500,
            model_turns: 4,
            model: Some("claude-sonnet-4-6".to_string()),
            full_metrics: None,
            ..Default::default()
        };
        let built = build_metrics(&args);

        // Cache split must be honest-zero (not fabricated).
        assert_eq!(built.tokens.input, 0, "input is honest-zero on flag path");
        assert_eq!(built.tokens.output, 0, "output is honest-zero on flag path");
        assert_eq!(
            built.tokens.cache_read, 0,
            "cache_read is honest-zero on flag path"
        );
        assert_eq!(
            built.tokens.cache_write, 0,
            "cache_write is honest-zero on flag path"
        );
        // But the aggregate total IS the supplied value.
        assert_eq!(built.tokens.total, 50_000, "total from --tokens flag");
        assert_eq!(
            built.cost_usd_micros, 3_000_000,
            "cost_usd_micros from flag"
        );
        assert_eq!(built.tool_calls, 7, "tool_calls from flag");
        assert_eq!(built.active_ms, 1_500, "active_ms from flag");
        assert_eq!(built.model_turns, 4, "model_turns from flag");
        assert_eq!(built.wall_ms, 0, "wall_ms honest-zero on flag path");
    }
}
