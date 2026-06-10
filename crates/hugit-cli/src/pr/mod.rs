//! `hugit pr open | land | show` — the PR-altitude porcelain (WP-PC3).
//!
//! A PR is a **bundle of intents** proposed to land together. This module is
//! the verb surface for that lifecycle; it owns no new primitive. Every
//! operation runs through the REAL engine seams:
//!
//! - the append-only, hash-chained event log
//!   ([`hugit_refstore::EventLog`]) — the source of truth a PR's lifecycle
//!   events are appended to and projected back out of (the same store
//!   `intent.landed` lives on; this module adds the `pr.opened` / `pr.queued`
//!   kinds **additively**, never a parallel store);
//! - the union-testing landing queue ([`hugit_queue::core`]) — `land` enters
//!   it through [`hugit_contracts::LandableEntry`] + [`hugit_queue::core::Batch`],
//!   so the reported position is the queue's own ordering, never invented;
//! - the F3 three-altitude rollup ([`hugit_ledger::pr_record`]) — `show`
//!   surfaces the PR's cost block when (and only when) the envelope/metrics
//!   data is present in the log.
//!
//! # Design law (the porcelain wave, 2026-06-10)
//!
//! - **Stable JSON on stdout always.** Every command returns a single JSON
//!   value (success or structured error). Structured errors carry the
//!   suggested fix.
//! - **Idempotent.** Re-running `open` with the same PR id + campaign returns
//!   the existing record (`"already_exists":true`, exit 0). Re-running `land`
//!   on an already-queued PR returns `"already_queued":true`, exit 0. Agents
//!   retry safely.
//! - **D14 at the door.** `open` takes an explicit `--author-kind`
//!   (`orchestrator | human`); `subagent` is NOT an accepted value — it is
//!   rejected with a structured error explaining the rule (a PR author is an
//!   orchestrator or a human, never a subagent — ADR-0001 §2.3/§5). The
//!   forge enforces the same rule again at the projection level (the F3
//!   rollup's `SubagentAuthor`); this is the porcelain's first line.
//! - **Hermetic-first.** Operates on the local event log through the real
//!   append/projection paths; live DO/CAS binding stays the P2 disclosed
//!   seam.
//! - **Honest gaps.** `show`'s cost block is the F3 `pr_record` rollup; its
//!   fields are honestly absent/null when the envelope/metrics data was never
//!   captured (sub-`full` capture, or no PR-altitude envelope on the log).

mod cli;

pub use cli::{PrArgs, PrCommand, run};

use std::collections::BTreeSet;

use hugit_contracts::context_envelope::{Altitude, CiCost, ContextEnvelope, PrRecord};
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::VerdictObject;
use hugit_ledger::rollup::{PrQueueInput, pr_record};
use hugit_queue::core::affected::AffectedSet;
use hugit_queue::core::batch::Batch;
use hugit_refstore::EventLog;
use serde::Serialize;
use serde_json::{Value, json};

/// Event kind: a PR was proposed (PROPOSED state). Additive over the D1 log,
/// same store as `intent.landed`. Payload (canonical JSON):
/// `{"pr_id","campaign","author_kind","run_id","principal","intent_ids":[...]}`.
pub const PR_OPENED_KIND: &str = "pr.opened";

/// Event kind: a PR entered the landing queue. Payload (canonical JSON):
/// `{"pr_id","item_id","order_index","mode":"union"}`.
pub const PR_QUEUED_KIND: &str = "pr.queued";

/// The landing mode a PR enters the queue under — always `union` in v1 (the
/// union-testing landing queue is the only landing path; the wedge property).
pub const LANDING_MODE: &str = "union";

/// The author kind the PR-altitude D14 authz rule accepts at the door.
///
/// **`subagent` is deliberately not a variant.** A PR author is an
/// orchestrator or a human, never a subagent (ADR-0001 §2.3/§5); the absence
/// of the variant encodes the rule at the type level. The string `"subagent"`
/// (or anything else) is rejected by [`AuthorKind::parse`] with a structured
/// error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    /// The orchestrator session that planned/dispatched/landed the bundle.
    Orchestrator,
    /// A human principal.
    Human,
}

impl AuthorKind {
    /// Parse an `--author-kind` token, enforcing D14 at the door.
    ///
    /// `orchestrator` / `human` accepted; `subagent` and any other value
    /// rejected (the caller turns the [`None`] into the structured
    /// `subagent_author` error so the message can name the offending value).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "orchestrator" => Some(AuthorKind::Orchestrator),
            "human" => Some(AuthorKind::Human),
            _ => None,
        }
    }

    /// The wire token for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            AuthorKind::Orchestrator => "orchestrator",
            AuthorKind::Human => "human",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors — structured, JSON, fix-carrying (design law).
// ─────────────────────────────────────────────────────────────────────────────

/// A structured PR-command error. Every variant serialises to a single JSON
/// object `{"error":<code>,"message":<text>,"fix":<suggested fix>, …}` so an
/// agent can branch on `error` and a human can read `message` + `fix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrError {
    /// `--author-kind subagent` (or any non-`orchestrator`/`human` value) was
    /// supplied: D14 refusal at the door.
    SubagentAuthor {
        /// The rejected value the caller passed.
        got: String,
    },
    /// `open` was asked for a PR id that already exists with a *different*
    /// campaign — not the idempotent re-open case (same campaign), a genuine
    /// conflict.
    CampaignMismatch {
        /// The PR id.
        pr_id: String,
        /// The campaign already recorded for this PR.
        existing: String,
        /// The campaign the re-open attempt supplied.
        attempted: String,
    },
    /// `land` was asked for a PR id with no `pr.opened` on the log.
    UnknownPr {
        /// The PR id.
        pr_id: String,
    },
    /// `land` was asked to queue a PR that bundles zero intents — nothing to
    /// land.
    EmptyPr {
        /// The PR id.
        pr_id: String,
    },
    /// A queue position could not be derived because the queue rejected the
    /// batch (e.g. a duplicate order_index across already-queued PRs — a
    /// corrupt log). Carries the queue engine's message.
    QueueRefused {
        /// The PR id being landed.
        pr_id: String,
        /// The queue engine's reason.
        reason: String,
    },
    /// `show` was asked for a PR id with no `pr.opened` on the log.
    NotFound {
        /// The PR id.
        pr_id: String,
    },
}

impl PrError {
    /// The stable machine-readable error code (the `error` field).
    pub fn code(&self) -> &'static str {
        match self {
            PrError::SubagentAuthor { .. } => "subagent_author",
            PrError::CampaignMismatch { .. } => "campaign_mismatch",
            PrError::UnknownPr { .. } => "unknown_pr",
            PrError::EmptyPr { .. } => "empty_pr",
            PrError::QueueRefused { .. } => "queue_refused",
            PrError::NotFound { .. } => "pr_not_found",
        }
    }

    /// Serialise to the stable JSON error object.
    pub fn to_json(&self) -> Value {
        match self {
            PrError::SubagentAuthor { got } => json!({
                "error": self.code(),
                "message": format!(
                    "author-kind '{got}' is not accepted: a PR author is an \
                     orchestrator or a human, never a subagent (D14)"
                ),
                "fix": "pass --author-kind orchestrator (with --run-id) or \
                        --author-kind human (with --principal)",
                "got": got,
            }),
            PrError::CampaignMismatch {
                pr_id,
                existing,
                attempted,
            } => json!({
                "error": self.code(),
                "message": format!(
                    "pr '{pr_id}' already exists under campaign '{existing}', \
                     cannot re-open under '{attempted}'"
                ),
                "fix": format!("re-open with --campaign {existing}, or open a new PR id"),
                "pr_id": pr_id,
                "existing_campaign": existing,
                "attempted_campaign": attempted,
            }),
            PrError::UnknownPr { pr_id } => json!({
                "error": self.code(),
                "message": format!("pr '{pr_id}' has no pr.opened on the log — open it first"),
                "fix": format!("hugit pr open --pr {pr_id} --campaign <key> --intent <id>…"),
                "pr_id": pr_id,
            }),
            PrError::EmptyPr { pr_id } => json!({
                "error": self.code(),
                "message": format!("pr '{pr_id}' bundles zero intents — nothing to land"),
                "fix": "re-open the PR with one or more --intent <id> before landing",
                "pr_id": pr_id,
            }),
            PrError::QueueRefused { pr_id, reason } => json!({
                "error": self.code(),
                "message": format!("landing queue refused pr '{pr_id}': {reason}"),
                "fix": "the event log's queue order is corrupt; inspect with hugit pr show",
                "pr_id": pr_id,
            }),
            PrError::NotFound { pr_id } => json!({
                "error": self.code(),
                "message": format!("pr '{pr_id}' not found on the log"),
                "fix": format!("hugit pr open --pr {pr_id} … to create it"),
                "pr_id": pr_id,
            }),
        }
    }
}

impl std::fmt::Display for PrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_json())
    }
}

impl std::error::Error for PrError {}

// ─────────────────────────────────────────────────────────────────────────────
// open — bundle intents into a PROPOSED PR.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr open`.
#[derive(Debug, Clone)]
pub struct OpenArgs {
    /// PR id (`--pr <n|auto>`). The caller resolves `auto` to a concrete id
    /// before calling (the porcelain owns id allocation); this is the chosen id.
    pub pr_id: String,
    /// Campaign key the PR belongs to (`--campaign <key>`).
    pub campaign: String,
    /// Author kind (D14 at the door) — already validated to be
    /// orchestrator/human by [`AuthorKind::parse`].
    pub author_kind: AuthorKind,
    /// Orchestrator run id (`--run-id`), when `author_kind == Orchestrator`.
    pub run_id: Option<String>,
    /// Human principal (`--principal`), when `author_kind == Human`.
    pub principal: Option<String>,
    /// Bundled intent ids (`--intent <id>…`, repeatable).
    pub intent_ids: Vec<String>,
    /// Unix ms to stamp the appended event with.
    pub recorded_at: u64,
}

/// `hugit pr open` — bundle intents into a PROPOSED PR (appends `pr.opened`).
///
/// Idempotent: if a `pr.opened` already exists for this PR id **with the same
/// campaign**, no event is appended and the existing record is returned with
/// `"already_exists":true` (exit 0). A re-open with a *different* campaign is a
/// genuine conflict ([`PrError::CampaignMismatch`]).
///
/// The D14 author-kind is validated by the caller via [`AuthorKind::parse`]
/// (a `subagent` token never reaches here as a valid [`AuthorKind`]).
pub fn open(log: &mut EventLog, args: &OpenArgs) -> Result<Value, PrError> {
    // Idempotency: look for an existing pr.opened for this id.
    if let Some(existing) = find_pr_opened(log, &args.pr_id) {
        if existing.campaign != args.campaign {
            return Err(PrError::CampaignMismatch {
                pr_id: args.pr_id.clone(),
                existing: existing.campaign.clone(),
                attempted: args.campaign.clone(),
            });
        }
        // Same campaign → idempotent no-op: return the existing record.
        return Ok(open_json(&existing, true));
    }

    let principal_chain = author_principal_chain(args.author_kind, &args.run_id, &args.principal);
    let payload = canonical_open_payload(args);
    log.append(PR_OPENED_KIND, principal_chain, payload, args.recorded_at);

    let opened = find_pr_opened(log, &args.pr_id).expect("pr.opened was just appended for this id");
    Ok(open_json(&opened, false))
}

// ─────────────────────────────────────────────────────────────────────────────
// land — enter the union-testing landing queue.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr land`.
#[derive(Debug, Clone)]
pub struct LandArgs {
    /// PR id to land (`--pr <n>`).
    pub pr_id: String,
    /// Unix ms to stamp the appended event with.
    pub recorded_at: u64,
}

/// `hugit pr land` — enter the union-testing landing queue.
///
/// Drives the REAL queue path: every already-queued PR plus this one is folded
/// into a [`hugit_queue::core::Batch`] of [`hugit_contracts::LandableEntry`]s
/// (ordered by `order_index`), so the reported `position` is the queue's own
/// ordering — never invented. Reports `{"queued":true,"position":N,"mode":"union"}`.
///
/// Refusals (structured): the PR has no `pr.opened` ([`PrError::UnknownPr`]),
/// or it bundles zero intents ([`PrError::EmptyPr`]).
///
/// Idempotent: a PR already queued returns `"already_queued":true` (exit 0),
/// no second `pr.queued` is appended, and the original position is reported.
pub fn land(log: &mut EventLog, args: &LandArgs) -> Result<Value, PrError> {
    let opened = find_pr_opened(log, &args.pr_id).ok_or_else(|| PrError::UnknownPr {
        pr_id: args.pr_id.clone(),
    })?;

    if opened.intent_ids.is_empty() {
        return Err(PrError::EmptyPr {
            pr_id: args.pr_id.clone(),
        });
    }

    // Idempotency: already queued → no-op, report the recorded position.
    if let Some(existing) = find_pr_queued(log, &args.pr_id) {
        return Ok(json!({
            "queued": true,
            "already_queued": true,
            "pr_id": args.pr_id,
            "position": existing.order_index,
            "mode": LANDING_MODE,
        }));
    }

    // The next free order_index: the count of PRs already queued (tail append).
    // We derive it THROUGH the real queue: fold the existing queued PRs plus
    // this new tail entry into a Batch, and read the position the queue assigns
    // (queue order is order_index order — Batch enforces uniqueness + sort).
    let queued = all_pr_queued(log);
    let order_index = queued.len() as u64;
    let item_id = format!("{}#{}", args.pr_id, order_index);

    let mut entries: Vec<(hugit_contracts::LandableEntry, AffectedSet)> = queued
        .iter()
        .map(|q| {
            (
                hugit_contracts::LandableEntry {
                    item_id: q.item_id.clone(),
                    intent_id: q.pr_id.clone(),
                    tree_hash: String::new(),
                    order_index: q.order_index,
                },
                // Affected-set is unknown at porcelain altitude (the queue
                // computes disjointness from B3's shape); an empty set is the
                // honest placeholder — it does not affect the position, which
                // is purely order_index ordering.
                AffectedSet::new(Vec::<String>::new()),
            )
        })
        .collect();
    entries.push((
        hugit_contracts::LandableEntry {
            item_id: item_id.clone(),
            intent_id: args.pr_id.clone(),
            tree_hash: String::new(),
            order_index,
        },
        AffectedSet::new(Vec::<String>::new()),
    ));

    // The queue is the authority on position: build the batch through the real
    // path and read where this PR landed in queue order.
    let batch = Batch::try_from_entries(format!("land-{}", args.pr_id), entries).map_err(|e| {
        PrError::QueueRefused {
            pr_id: args.pr_id.clone(),
            reason: e.to_string(),
        }
    })?;
    let position = batch
        .entries()
        .iter()
        .position(|e| e.item_id() == item_id)
        .expect("the entry just pushed is present in the batch") as u64;

    let payload = format!(
        "{{\"item_id\":{item},\"mode\":{mode},\"order_index\":{idx},\"pr_id\":{pr}}}",
        item = json_str(&item_id),
        mode = json_str(LANDING_MODE),
        idx = order_index,
        pr = json_str(&args.pr_id),
    );
    log.append(PR_QUEUED_KIND, vec![], payload, args.recorded_at);

    Ok(json!({
        "queued": true,
        "already_queued": false,
        "pr_id": args.pr_id,
        "position": position,
        "mode": LANDING_MODE,
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// show — the PR's intents, queue state, and the F3 cost rollup when present.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr show`.
#[derive(Debug, Clone)]
pub struct ShowArgs {
    /// PR id to show (`--pr <n>`).
    pub pr_id: String,
}

/// `hugit pr show` — the PR's intents, queue state, and (when captured) the F3
/// `pr_record` rollup cost block.
///
/// The cost block is computed from the PR-altitude envelope + intent-altitude
/// envelopes carried (additively) on the log under the `pr.envelope` /
/// `intent.envelope` kinds — see [`envelopes_for_pr`]. When no PR-altitude
/// envelope is on the log, the `cost` field is honestly `null` (the rollup was
/// never captured), never faked.
pub fn show(log: &EventLog, args: &ShowArgs) -> Result<Value, PrError> {
    let opened = find_pr_opened(log, &args.pr_id).ok_or_else(|| PrError::NotFound {
        pr_id: args.pr_id.clone(),
    })?;

    // Queue state, if landed into the queue.
    let queue_state = match find_pr_queued(log, &args.pr_id) {
        Some(q) => json!({
            "queued": true,
            "position": q.order_index,
            "mode": LANDING_MODE,
        }),
        None => json!({ "queued": false }),
    };

    // The F3 pr_record rollup — present only when a PR-altitude envelope is
    // captured on the log; null otherwise (honest gap, never faked).
    let cost = pr_record_for(log, &opened);

    Ok(json!({
        "pr_id": opened.pr_id,
        "campaign": opened.campaign,
        "author_kind": opened.author_kind.as_str(),
        "intent_ids": opened.intent_ids,
        "intent_count": opened.intent_ids.len(),
        "queue": queue_state,
        "cost": cost,
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Projections over the event log — the read models the verbs share.
// ─────────────────────────────────────────────────────────────────────────────

/// A projected `pr.opened` event (the PROPOSED PR record).
#[derive(Debug, Clone)]
pub struct OpenedPr {
    /// The PR id.
    pub pr_id: String,
    /// Campaign key the PR belongs to.
    pub campaign: String,
    /// The author kind recorded at open time (D14: orchestrator | human).
    pub author_kind: AuthorKind,
    /// Bundled intent ids.
    pub intent_ids: Vec<String>,
    /// Log seq of the `pr.opened` event.
    pub seq: u64,
}

/// A projected `pr.queued` event (the PR's landing-queue position).
#[derive(Debug, Clone)]
pub struct QueuedPr {
    /// The PR id.
    pub pr_id: String,
    /// The queue item id.
    pub item_id: String,
    /// The queue position (`order_index`).
    pub order_index: u64,
}

/// Project the latest `pr.opened` for a PR id out of the log, if any.
///
/// Folds the log in chain order; the last `pr.opened` for the id wins (a
/// re-open with the same campaign is an idempotent no-op so there is normally
/// at most one, but folding the latest is robust under any future re-state).
pub fn find_pr_opened(log: &EventLog, pr_id: &str) -> Option<OpenedPr> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_OPENED_KIND)
        .filter_map(parse_opened)
        .rfind(|o| o.pr_id == pr_id)
}

/// Project the `pr.queued` for a PR id out of the log, if any.
pub fn find_pr_queued(log: &EventLog, pr_id: &str) -> Option<QueuedPr> {
    all_pr_queued(log).into_iter().find(|q| q.pr_id == pr_id)
}

/// Project every `pr.queued` on the log, in log (queue) order.
pub fn all_pr_queued(log: &EventLog) -> Vec<QueuedPr> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_QUEUED_KIND)
        .filter_map(parse_queued)
        .collect()
}

fn parse_opened(r: &EventRecord) -> Option<OpenedPr> {
    let v: Value = serde_json::from_str(&r.payload).ok()?;
    let pr_id = v.get("pr_id")?.as_str()?.to_string();
    let campaign = v.get("campaign")?.as_str()?.to_string();
    let author_kind = AuthorKind::parse(v.get("author_kind")?.as_str()?)?;
    let intent_ids = v
        .get("intent_ids")?
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    Some(OpenedPr {
        pr_id,
        campaign,
        author_kind,
        intent_ids,
        seq: r.seq,
    })
}

fn parse_queued(r: &EventRecord) -> Option<QueuedPr> {
    let v: Value = serde_json::from_str(&r.payload).ok()?;
    Some(QueuedPr {
        pr_id: v.get("pr_id")?.as_str()?.to_string(),
        item_id: v.get("item_id")?.as_str()?.to_string(),
        order_index: v.get("order_index")?.as_u64()?,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// The F3 cost rollup seam (show's cost block).
// ─────────────────────────────────────────────────────────────────────────────

/// Event kind carrying a captured PR-altitude [`ContextEnvelope`] for the
/// rollup. Additive; payload is the envelope as canonical JSON.
pub const PR_ENVELOPE_KIND: &str = "pr.envelope";

/// Event kind carrying a captured intent-altitude [`ContextEnvelope`].
pub const INTENT_ENVELOPE_KIND: &str = "intent.envelope";

/// The PR-altitude + intent-altitude envelopes captured on the log for a PR.
struct PrEnvelopes {
    pr: ContextEnvelope,
    intents: Vec<ContextEnvelope>,
}

/// Read the captured envelopes for a PR off the log, if a PR-altitude envelope
/// is present. Intent-altitude envelopes whose `intent_id` is in the PR's
/// bundle are attributed to it.
fn envelopes_for_pr(log: &EventLog, opened: &OpenedPr) -> Option<PrEnvelopes> {
    let bundle: BTreeSet<&str> = opened.intent_ids.iter().map(String::as_str).collect();

    let pr = log
        .records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == opened.pr_id)?;

    let intents = log
        .records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .filter(|e| e.altitude == Altitude::Intent && bundle.contains(e.intent_id.as_str()))
        .collect();

    Some(PrEnvelopes { pr, intents })
}

/// Compute the F3 [`PrRecord`] for a PR when its envelope/metrics data is
/// present in the log; `None` (→ JSON `null`) when no PR-altitude envelope was
/// captured (honest gap, never faked).
///
/// `landed_intent_ids` is the PR's own bundle (the v1 landing unit is the
/// intent id; the bundle's intents are what the PR proposes to land). CI cost
/// and queue timing have no porcelain seam yet, so they are passed as zero /
/// default — the rollup tolerates that and computes the measured figures
/// (work/orchestration/waste/first-pass-yield) from the envelopes.
fn pr_record_for(log: &EventLog, opened: &OpenedPr) -> Option<Value> {
    let envs = envelopes_for_pr(log, opened)?;
    // The PR's bundled intents are its landing unit.
    let landed: Vec<String> = opened.intent_ids.clone();
    let verdicts: Vec<VerdictObject> = Vec::new();
    let record: PrRecord = pr_record(
        &envs.pr,
        // No CAS binding at porcelain altitude (P2 disclosed seam): the
        // envelope_ref is threaded through as empty rather than faked.
        "",
        &envs.intents,
        &landed,
        &verdicts,
        // No CI / pricing seam at porcelain altitude — zero, never faked.
        CiCost {
            cache_hit: 0,
            exec: 0,
            cost_usd: 0.0,
            saved_usd: 0.0,
        },
        PrQueueInput::default(),
    )
    .ok()?; // a subagent-authored PR envelope is rejected → no cost block.
    serde_json::to_value(record).ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON helpers.
// ─────────────────────────────────────────────────────────────────────────────

fn open_json(opened: &OpenedPr, already_exists: bool) -> Value {
    json!({
        "pr_id": opened.pr_id,
        "campaign": opened.campaign,
        "author_kind": opened.author_kind.as_str(),
        "intent_ids": opened.intent_ids,
        "intent_count": opened.intent_ids.len(),
        "state": "proposed",
        "already_exists": already_exists,
    })
}

/// The principal chain to stamp a `pr.opened` event with: the human principal
/// for a human author, the orchestrator run id for an orchestrator author.
fn author_principal_chain(
    kind: AuthorKind,
    run_id: &Option<String>,
    principal: &Option<String>,
) -> Vec<String> {
    match kind {
        AuthorKind::Human => principal.clone().into_iter().collect(),
        AuthorKind::Orchestrator => run_id.clone().into_iter().collect(),
    }
}

/// Build the canonical-JSON `pr.opened` payload (sorted keys, no insignificant
/// whitespace — the hash chain covers these bytes verbatim).
fn canonical_open_payload(args: &OpenArgs) -> String {
    // Keys in sorted order: author_kind, campaign, intent_ids, pr_id, principal,
    // run_id. Optional fields are emitted as null when absent so the payload is
    // self-describing.
    let intents = args
        .intent_ids
        .iter()
        .map(|s| json_str(s))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"author_kind\":{ak},\"campaign\":{camp},\"intent_ids\":[{intents}],\
         \"pr_id\":{pr},\"principal\":{prin},\"run_id\":{rid}}}",
        ak = json_str(args.author_kind.as_str()),
        camp = json_str(&args.campaign),
        pr = json_str(&args.pr_id),
        prin = opt_json_str(&args.principal),
        rid = opt_json_str(&args.run_id),
    )
}

/// JSON-encode a string (quoted + escaped) via serde, so payload bytes are
/// always valid JSON regardless of the input.
fn json_str(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

/// JSON-encode an optional string: the value (quoted) or the literal `null`.
fn opt_json_str(s: &Option<String>) -> String {
    match s {
        Some(v) => json_str(v),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_args(pr: &str, campaign: &str, intents: &[&str]) -> OpenArgs {
        OpenArgs {
            pr_id: pr.to_string(),
            campaign: campaign.to_string(),
            author_kind: AuthorKind::Orchestrator,
            run_id: Some("run-1".to_string()),
            principal: None,
            intent_ids: intents.iter().map(|s| s.to_string()).collect(),
            recorded_at: 1000,
        }
    }

    #[test]
    fn author_kind_rejects_subagent() {
        assert_eq!(AuthorKind::parse("subagent"), None);
        assert_eq!(
            AuthorKind::parse("orchestrator"),
            Some(AuthorKind::Orchestrator)
        );
        assert_eq!(AuthorKind::parse("human"), Some(AuthorKind::Human));
    }

    #[test]
    fn open_payload_is_canonical_json() {
        let args = open_args("7", "camp-a", &["i1", "i2"]);
        let payload = canonical_open_payload(&args);
        // Round-trips and re-canonicalises to itself (sorted keys).
        let recanon = hugit_refstore::canonical_json(&payload).unwrap();
        assert_eq!(payload, recanon, "payload must already be canonical JSON");
    }

    #[test]
    fn open_then_show_roundtrips_over_real_log() {
        let mut log = EventLog::new();
        let out = open(&mut log, &open_args("7", "camp-a", &["i1", "i2"])).unwrap();
        assert_eq!(out["already_exists"], json!(false));
        assert_eq!(out["state"], json!("proposed"));

        let shown = show(
            &log,
            &ShowArgs {
                pr_id: "7".to_string(),
            },
        )
        .unwrap();
        assert_eq!(shown["intent_count"], json!(2));
        assert_eq!(shown["queue"]["queued"], json!(false));
        // No envelope captured → honest null cost block.
        assert_eq!(shown["cost"], Value::Null);
    }

    #[test]
    fn open_is_idempotent_same_campaign() {
        let mut log = EventLog::new();
        open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
        let n_before = log.len();
        let again = open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
        assert_eq!(again["already_exists"], json!(true));
        assert_eq!(log.len(), n_before, "re-open appends no second event");
    }
}
