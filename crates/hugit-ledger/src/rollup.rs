//! WP-F3 — the three-altitude metric rollups (ADR-0001 §2.3):
//! intent → **PR record** → **campaign rollup**.
//!
//! The forge COMPUTES these records from the captured [`ContextEnvelope`]s
//! plus queue/verdict/check data available in the projection seams; they are
//! never stored per envelope and never gate anything. The output shapes
//! ([`PrRecord`], [`CampaignRollup`]) are the derived (non-frozen) read models
//! in `hugit-contracts`; this module is the computation behind them.
//!
//! # The decomposition (one vocabulary at every altitude)
//!
//! - **work** — Σ over the unit's *landed* intents (the subagents' spend):
//!   the final attempt per landed intent id.
//! - **orchestration** — the unit author's own session spend (the PR-altitude
//!   envelope's metrics): planning, decomposing, dispatching, cold-verifying,
//!   landing.
//! - **verification** — the adversarial panels.
//! - **ci** — check executions + the memoization economics, supplied from the
//!   checks seam (the AC hit-rate meter measures the counts).
//! - **waste** — spent-but-not-landed, shown not hidden: every retried
//!   attempt of a landed intent and every attempt of an intent that never
//!   landed.
//! - **total** — `work + orchestration + verification + ci` (waste is shown
//!   in its own line and **excluded** from total, per the ADR JSONC).
//!
//! # Authz (D14, ADR-0001 §2.3/§5)
//!
//! A PR is authored by the **orchestrator or a human — never a subagent**.
//! [`PrAuthorKind`] already makes a subagent author unrepresentable at the
//! type level; this module enforces the same rule at the projection level:
//! a PR- or campaign-altitude envelope whose authorship carries a subagent
//! marker (`agent_type != "main"`, or a non-null `spawn.parent_run_id` — a
//! spawned session is a subagent) is **rejected** with
//! [`RollupError::SubagentAuthor`], fail-closed.
//!
//! # Null-ref tolerance (capture levels below `full`)
//!
//! The rollup reads only ids, authorship and `metrics` from the envelopes —
//! every `cas:` ref inside `trajectory` / `snapshot` / `verdicts_ref` may be
//! `null` (sub-full capture) without affecting any figure. A landed intent id
//! with **no** captured envelope at all is tolerated too: it contributes zero
//! spend (and is counted as a single, first-pass attempt).
//!
//! # Honest gaps (inputs that do not exist in the seams yet — documented,
//! # not faked)
//!
//! - `verification.tokens` / `verification.cost_usd_micros` compute to `0`:
//!   the verdict seam ([`VerdictObject`]) carries no token/cost fields, only
//!   the panels themselves. `verdict_panels` is the real panel count.
//! - `efficiency.cost_per_net_kloc` computes to `0.0`: no projection seam
//!   carries diff/LOC statistics (the `intent.landed` payload has
//!   ref/target/charter only).
//! - `ci.cost_usd_micros` / `ci.saved_usd_micros` are whatever the checks seam
//!   supplies; when no pricing seam exists they are `0` and
//!   `cache_savings_pct` honestly computes to `0.0` (the hit/exec **counts**
//!   are measured).
//! - `queue_wait_ms` / `human_touches` / `landed_at` are queue-seam state,
//!   supplied via [`PrQueueInput`] (the landing queue owns them; the
//!   envelope does not carry them).

use std::collections::BTreeSet;

use hugit_contracts::context_envelope::{
    Altitude, CampaignOwner, CampaignProgress, CampaignRollup, CampaignTime, CiCost,
    ContextEnvelope, CostDecomposition, Efficiency, OrchestrationCost, PrAuthor, PrAuthorKind,
    PrRecord, PrTime, TotalCost, VerificationCost, WasteCost, WorkCost,
};
use hugit_contracts::verdict_object::VerdictObject;

/// Why a rollup refused to compute. Fail-closed: a record is either computed
/// from inputs that satisfy the altitude + authz rules, or not at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollupError {
    /// An envelope arrived at the wrong altitude for its position (e.g. an
    /// `altitude:"intent"` envelope where the PR's own envelope belongs).
    WrongAltitude {
        /// The altitude this input position requires.
        expected: Altitude,
        /// The altitude the envelope actually declares.
        found: Altitude,
        /// The envelope's authored-unit id, for attribution.
        intent_id: String,
    },
    /// A PR- or campaign-altitude envelope is authored by a **subagent** —
    /// forbidden by the D14 forge-authz rule (a PR author is an orchestrator
    /// or a human, never a subagent; a campaign is owned by a human).
    SubagentAuthor {
        /// The altitude of the rejected envelope.
        altitude: Altitude,
        /// The envelope's authored-unit id.
        intent_id: String,
        /// The offending session's run id.
        run_id: String,
        /// The subagent marker that triggered the rejection.
        agent_type: String,
    },
    /// A money (micro-USD) or token accumulator overflowed `u64` while summing
    /// across attempts / intents / PRs. **Saturate-or-error policy: ERROR**
    /// (fail-closed, WA4) — the rollup refuses to emit a silently-wrapped
    /// figure. `u64` micro-USD spans ~18.4 trillion USD, so an overflow here is
    /// a tamper/garbage signal, not a real fleet bill; surfacing it is correct.
    Overflow {
        /// Which accumulator overflowed (e.g. `"work.cost_usd_micros"`).
        field: &'static str,
    },
}

impl std::fmt::Display for RollupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RollupError::WrongAltitude {
                expected,
                found,
                intent_id,
            } => write!(
                f,
                "envelope '{intent_id}' has altitude {found:?}, expected {expected:?}"
            ),
            RollupError::SubagentAuthor {
                altitude,
                intent_id,
                run_id,
                agent_type,
            } => write!(
                f,
                "envelope '{intent_id}' at altitude {altitude:?} is subagent-authored \
                 (run {run_id}, agent_type '{agent_type}') — a PR/campaign author is \
                 never a subagent (D14)"
            ),
            RollupError::Overflow { field } => write!(
                f,
                "rollup accumulator '{field}' overflowed u64 — refusing to emit a \
                 wrapped figure (fail-closed)"
            ),
        }
    }
}

impl std::error::Error for RollupError {}

/// Landing-queue state for one PR — the inputs the queue seam (D5) owns and
/// the envelope does not carry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrQueueInput {
    /// Time the PR was held in the landing queue, ms.
    pub queue_wait_ms: u64,
    /// Human decisions/comments on the PR.
    pub human_touches: u64,
    /// Unix ms when the PR landed; `0` when it has not landed (yet).
    pub landed_at: u64,
}

/// A PR's landing-queue phase, for [`CampaignProgress`]. The campaign bundles
/// **PRs** (never raw commits); each PR in the bundle is in exactly one phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrPhase {
    /// The PR landed (union-tested green and integrated).
    Landed,
    /// The PR is queued / union-testing.
    InFlight,
    /// The PR is blocked (e.g. isolated by a minimal failing pair).
    Blocked,
}

/// The `agent_type` an envelope's authorship carries for a **top-level**
/// (orchestrator / human / campaign) session — anything else is a subagent
/// type (frozen [`hugit_contracts::context_envelope::Authorship`] doc).
const TOP_LEVEL_AGENT_TYPE: &str = "main";

// ─────────────────────────────────────────────────────────────────────────────
// PR record — the second altitude
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the [`PrRecord`] for one PR (ADR-0001 §2.3) — NOT just a sum.
///
/// Inputs, by projection seam:
/// - `pr_envelope` — the PR's OWN captured envelope (`altitude:"pr"`, owner
///   2026-06-10): the orchestrator/human session that planned, dispatched and
///   landed the bundle. Its `intent_id` carries the PR id; its metrics are
///   the **orchestration** line; its authorship yields the [`PrAuthor`]
///   (rejected fail-closed if subagent-authored — D14).
/// - `pr_envelope_ref` — the `cas:` ref under which that envelope is stored
///   (the capture path owns CAS writes; the rollup is a pure projection and
///   threads the ref through).
/// - `intent_envelopes` — every captured intent-altitude envelope attributed
///   to this PR, including retried attempts and discarded intents. Multiple
///   envelopes sharing one `intent_id` are attempts; the latest-born attempt
///   of a **landed** id is **work**, every other envelope is **waste**.
/// - `landed_intent_ids` — which intent ids actually landed, from the intent
///   projection over the event log (`intents_from_log` /
///   [`crate::Ledger::from_records`]) — the queue's truth, never inferred
///   from the envelopes themselves.
/// - `verdicts` — the adversarial panels recorded for this PR's intents
///   (`verdict.recorded` on the event log). Panel **count** is real;
///   panel token/cost spend has no seam yet (see module docs).
/// - `ci` — the CI line from the checks seam (measured hit/exec counts +
///   memoization economics).
/// - `queue` — landing-queue state (wait, human touches, landed-at).
pub fn pr_record(
    pr_envelope: &ContextEnvelope,
    pr_envelope_ref: &str,
    intent_envelopes: &[ContextEnvelope],
    landed_intent_ids: &[String],
    verdicts: &[VerdictObject],
    ci: CiCost,
    queue: PrQueueInput,
) -> Result<PrRecord, RollupError> {
    ensure_altitude(pr_envelope, Altitude::Pr)?;
    for env in intent_envelopes {
        ensure_altitude(env, Altitude::Intent)?;
    }
    let author = pr_author(pr_envelope)?;

    // ── Group intent envelopes into attempts per intent id (first-seen order).
    let attempts = group_attempts(intent_envelopes);
    let landed: BTreeSet<&str> = landed_intent_ids.iter().map(String::as_str).collect();

    // ── work (Σ landed intents — the final attempt per landed id) + waste.
    let mut work = WorkCost {
        tokens: 0,
        tool_calls: 0,
        cost_usd_micros: 0,
    };
    let mut waste = WasteCost {
        discarded_intents: 0,
        retried_agents: 0,
        tokens_not_landed: 0,
        cost_usd_micros: 0,
    };
    // First-pass yield bookkeeping: ids authored vs landed on the 1st attempt.
    let mut authored_ids: BTreeSet<&str> = attempts.iter().map(|(id, _)| *id).collect();
    for id in &landed {
        authored_ids.insert(id);
    }
    let mut landed_first_pass: u64 = 0;

    for (id, group) in &attempts {
        let is_landed = landed.contains(id);
        waste.retried_agents += (group.len() as u64).saturating_sub(1);
        if is_landed {
            // Final attempt (latest-born; ties → last in input order) is work…
            let final_idx = final_attempt_idx(group);
            for (i, env) in group.iter().enumerate() {
                if i == final_idx {
                    work.tokens = add(work.tokens, env.metrics.tokens.total, "work.tokens")?;
                    work.tool_calls =
                        add(work.tool_calls, env.metrics.tool_calls, "work.tool_calls")?;
                    work.cost_usd_micros = add(
                        work.cost_usd_micros,
                        env.metrics.cost_usd_micros,
                        "work.cost_usd_micros",
                    )?;
                } else {
                    // …every earlier attempt is retried spend that never landed.
                    waste.tokens_not_landed = add(
                        waste.tokens_not_landed,
                        env.metrics.tokens.total,
                        "waste.tokens_not_landed",
                    )?;
                    waste.cost_usd_micros = add(
                        waste.cost_usd_micros,
                        env.metrics.cost_usd_micros,
                        "waste.cost_usd_micros",
                    )?;
                }
            }
            if group.len() <= 1 {
                landed_first_pass += 1;
            }
        } else {
            // Discarded intent: every attempt is spend that never landed.
            waste.discarded_intents += 1;
            for env in group {
                waste.tokens_not_landed = add(
                    waste.tokens_not_landed,
                    env.metrics.tokens.total,
                    "waste.tokens_not_landed",
                )?;
                waste.cost_usd_micros = add(
                    waste.cost_usd_micros,
                    env.metrics.cost_usd_micros,
                    "waste.cost_usd_micros",
                )?;
            }
        }
    }
    // Landed ids with no captured envelope (sub-full capture): tolerated —
    // zero spend, counted as a single first-pass attempt.
    for id in &landed {
        if !attempts.iter().any(|(aid, _)| aid == id) {
            landed_first_pass += 1;
        }
    }

    // ── orchestration: the PR author's own session spend.
    let orchestration = OrchestrationCost {
        tokens: pr_envelope.metrics.tokens.total,
        tool_calls: pr_envelope.metrics.tool_calls,
        turns: pr_envelope.metrics.model_turns,
        cost_usd_micros: pr_envelope.metrics.cost_usd_micros,
    };

    // ── verification: real panel count; spend has no seam yet (module docs).
    let verification = VerificationCost {
        tokens: 0,
        verdict_panels: verdicts.len() as u64,
        cost_usd_micros: 0,
    };

    let cost = decompose(work, orchestration, verification, ci, waste)?;

    // ── time: span vs sum (ADR-0001 §2.3 — documented law, not a theorem).
    //
    // `wall_span_ms` is the wall-clock interval from first activity to
    // landing (or last session death if not yet landed). `agent_sum_ms` is
    // GROSS agent-time: the sum of every intent attempt's `active_ms`, waste
    // included (that time was spent, regardless of whether it landed).
    //
    // **Span ≤ sum holds for genuinely parallel fleets** (agents run
    // concurrently — their individual times accumulate faster than the wall
    // clock advances). **Span > sum is LEGAL and expected for serial
    // workloads or idle-heavy sessions** (a single agent occupies a fraction
    // of the wall interval; the remainder is queue wait, review, or
    // orchestration time between dispatches). The relationship is a
    // STRUCTURAL PROPERTY of the inputs, not an invariant the rollup
    // enforces — the acceptance suite covers both cases (see
    // `wall_span_le_agent_sum_under_parallelism` and
    // `wall_span_gt_agent_sum_serial_is_legal` in acceptance_f3).
    let first_born = intent_envelopes
        .iter()
        .map(|e| e.authorship.spawn.born_at)
        .chain(std::iter::once(pr_envelope.authorship.spawn.born_at))
        .min()
        .unwrap_or(0);
    let last_died = intent_envelopes
        .iter()
        .map(|e| e.authorship.spawn.died_at)
        .chain(std::iter::once(pr_envelope.authorship.spawn.died_at))
        .max()
        .unwrap_or(0);
    let span_end = if queue.landed_at > 0 {
        queue.landed_at
    } else {
        last_died // not landed (yet): span runs to the last session death.
    };
    // agent_sum: saturating_add (timing counter — a wrap would indicate
    // garbage input, not a real overspend; capped at u64::MAX is the correct
    // behaviour for a display field, unlike money which is fail-closed).
    let agent_sum_ms = intent_envelopes
        .iter()
        .fold(0u64, |acc, e| acc.saturating_add(e.metrics.active_ms));
    let time = PrTime {
        wall_span_ms: span_end.saturating_sub(first_born),
        agent_sum_ms,
        queue_wait_ms: queue.queue_wait_ms,
        human_touches: queue.human_touches,
        landed_at: queue.landed_at,
    };

    let first_pass_yield = ratio(landed_first_pass, authored_ids.len() as u64);
    let efficiency = efficiency_from(&cost, first_pass_yield);

    // ── identity / bundle facts.
    let mut models_used = dedup_push(Vec::new(), &pr_envelope.authorship.model);
    for env in intent_envelopes {
        models_used = dedup_push(models_used, &env.authorship.model);
    }
    let agent_runs: BTreeSet<&str> = intent_envelopes
        .iter()
        .map(|e| e.authorship.spawn.run_id.as_str())
        .collect();

    Ok(PrRecord {
        pr_id: pr_envelope.intent_id.clone(),
        author,
        intent_ids: landed_intent_ids.to_vec(),
        intent_count: landed_intent_ids.len() as u64,
        agent_count: agent_runs.len() as u64,
        models_used,
        envelope_ref: pr_envelope_ref.to_string(),
        cost,
        time,
        efficiency,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Campaign rollup — the third altitude
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the [`CampaignRollup`] (ADR-0001 §2.3) over the campaign's **PRs**
/// — the landing-queue bundle of PRs, NOT a bundle of commits.
///
/// The campaign consolidates its PRs the same way a PR consolidates its
/// intents: the cost decomposition is the component-wise Σ over the PR
/// records (so the `total` identity holds by construction), plus campaign
/// progress (landed / in-flight / blocked).
///
/// `campaign_envelope` is the campaign's OWN captured envelope
/// (`altitude:"campaign"`, owner 2026-06-10): its `intent_id` carries the
/// campaign key, its `charter` the human-defined goal, its
/// `authorship.operator` the human owner principal. Per the ADR JSONC the
/// campaign cost is "Σ over PRs" — the campaign session's own metrics are
/// reachable behind `envelope_ref`, not folded into the decomposition.
/// A subagent-authored campaign envelope is rejected (D14: a campaign is
/// owned by a human, never a subagent).
pub fn campaign_rollup(
    campaign_envelope: &ContextEnvelope,
    campaign_envelope_ref: &str,
    prs: &[(PrRecord, PrPhase)],
) -> Result<CampaignRollup, RollupError> {
    ensure_altitude(campaign_envelope, Altitude::Campaign)?;
    ensure_top_level(campaign_envelope)?;

    // ── Σ over PRs, component-wise (one vocabulary at every altitude).
    let mut work = WorkCost {
        tokens: 0,
        tool_calls: 0,
        cost_usd_micros: 0,
    };
    let mut orchestration = OrchestrationCost {
        tokens: 0,
        tool_calls: 0,
        turns: 0,
        cost_usd_micros: 0,
    };
    let mut verification = VerificationCost {
        tokens: 0,
        verdict_panels: 0,
        cost_usd_micros: 0,
    };
    let mut ci = CiCost {
        cache_hit: 0,
        exec: 0,
        cost_usd_micros: 0,
        saved_usd_micros: 0,
    };
    let mut waste = WasteCost {
        discarded_intents: 0,
        retried_agents: 0,
        tokens_not_landed: 0,
        cost_usd_micros: 0,
    };
    let mut pr_ids = Vec::with_capacity(prs.len());
    let mut intent_count = 0u64;
    let mut agent_count = 0u64;
    let mut models_used: Vec<String> = Vec::new();
    let mut agent_sum_ms = 0u64;
    let mut queue_wait_ms = 0u64;
    let mut progress = CampaignProgress {
        landed: 0,
        in_flight: 0,
        blocked: 0,
    };
    let mut last_landed_at = 0u64;
    // First-pass yield, weighted by ids authored per PR (intent_count landed
    // + discarded): Σ first-pass-landed ÷ Σ authored, recovered from each
    // PR's own yield × its authored count.
    let mut authored_total = 0.0f64;
    let mut first_pass_total = 0.0f64;

    for (pr, phase) in prs {
        pr_ids.push(pr.pr_id.clone());
        // intent_count / agent_count: saturating (counter, not money).
        intent_count = intent_count.saturating_add(pr.intent_count);
        agent_count = agent_count.saturating_add(pr.agent_count);
        for m in &pr.models_used {
            models_used = dedup_push(models_used, m);
        }

        work.tokens = add(work.tokens, pr.cost.work.tokens, "work.tokens")?;
        work.tool_calls = add(work.tool_calls, pr.cost.work.tool_calls, "work.tool_calls")?;
        work.cost_usd_micros = add(
            work.cost_usd_micros,
            pr.cost.work.cost_usd_micros,
            "work.cost_usd_micros",
        )?;
        orchestration.tokens = add(
            orchestration.tokens,
            pr.cost.orchestration.tokens,
            "orchestration.tokens",
        )?;
        orchestration.tool_calls = add(
            orchestration.tool_calls,
            pr.cost.orchestration.tool_calls,
            "orchestration.tool_calls",
        )?;
        orchestration.turns = add(
            orchestration.turns,
            pr.cost.orchestration.turns,
            "orchestration.turns",
        )?;
        orchestration.cost_usd_micros = add(
            orchestration.cost_usd_micros,
            pr.cost.orchestration.cost_usd_micros,
            "orchestration.cost_usd_micros",
        )?;
        verification.tokens = add(
            verification.tokens,
            pr.cost.verification.tokens,
            "verification.tokens",
        )?;
        verification.verdict_panels = add(
            verification.verdict_panels,
            pr.cost.verification.verdict_panels,
            "verification.verdict_panels",
        )?;
        verification.cost_usd_micros = add(
            verification.cost_usd_micros,
            pr.cost.verification.cost_usd_micros,
            "verification.cost_usd_micros",
        )?;
        ci.cache_hit = add(ci.cache_hit, pr.cost.ci.cache_hit, "ci.cache_hit")?;
        ci.exec = add(ci.exec, pr.cost.ci.exec, "ci.exec")?;
        ci.cost_usd_micros = add(
            ci.cost_usd_micros,
            pr.cost.ci.cost_usd_micros,
            "ci.cost_usd_micros",
        )?;
        ci.saved_usd_micros = add(
            ci.saved_usd_micros,
            pr.cost.ci.saved_usd_micros,
            "ci.saved_usd_micros",
        )?;
        // waste counters: saturating (display, not money).
        waste.discarded_intents = waste
            .discarded_intents
            .saturating_add(pr.cost.waste.discarded_intents);
        waste.retried_agents = waste
            .retried_agents
            .saturating_add(pr.cost.waste.retried_agents);
        waste.tokens_not_landed = add(
            waste.tokens_not_landed,
            pr.cost.waste.tokens_not_landed,
            "waste.tokens_not_landed",
        )?;
        waste.cost_usd_micros = add(
            waste.cost_usd_micros,
            pr.cost.waste.cost_usd_micros,
            "waste.cost_usd_micros",
        )?;

        // timing accumulators: saturating (display counters, not money).
        agent_sum_ms = agent_sum_ms.saturating_add(pr.time.agent_sum_ms);
        queue_wait_ms = queue_wait_ms.saturating_add(pr.time.queue_wait_ms);

        match phase {
            PrPhase::Landed => {
                progress.landed += 1;
                last_landed_at = last_landed_at.max(pr.time.landed_at);
            }
            PrPhase::InFlight => progress.in_flight += 1,
            PrPhase::Blocked => progress.blocked += 1,
        }

        let authored = (pr.intent_count + pr.cost.waste.discarded_intents) as f64;
        authored_total += authored;
        first_pass_total += pr.efficiency.first_pass_yield * authored;
    }

    let cost = decompose(work, orchestration, verification, ci, waste)?;

    // ── time: lead time — campaign opened (its session's birth) → last PR
    // landed; if nothing landed yet, the span runs to the campaign session's
    // death (lead time so far).
    let born = campaign_envelope.authorship.spawn.born_at;
    let end = if last_landed_at > 0 {
        last_landed_at
    } else {
        campaign_envelope.authorship.spawn.died_at
    };
    let time = CampaignTime {
        wall_span_ms: end.saturating_sub(born),
        agent_sum_ms,
        queue_wait_ms,
    };

    let first_pass_yield = if authored_total > 0.0 {
        first_pass_total / authored_total
    } else {
        0.0
    };
    let efficiency = efficiency_from(&cost, first_pass_yield);

    Ok(CampaignRollup {
        campaign: campaign_envelope.intent_id.clone(),
        charter: campaign_envelope.charter.clone(),
        owner: CampaignOwner {
            principal: campaign_envelope.authorship.operator.clone(),
        },
        envelope_ref: campaign_envelope_ref.to_string(),
        pr_count: pr_ids.len() as u64,
        pr_ids,
        intent_count,
        agent_count,
        models_used,
        cost,
        time,
        efficiency,
        progress,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared computation
// ─────────────────────────────────────────────────────────────────────────────

/// `a + b` over `u64`, fail-closed on overflow (WA4 saturate-or-error policy:
/// **ERROR**). Used for every money (micro-USD) and token accumulator so a
/// silent `u64` wrap can never reach a rollup figure.
fn add(a: u64, b: u64, field: &'static str) -> Result<u64, RollupError> {
    a.checked_add(b).ok_or(RollupError::Overflow { field })
}

/// Assemble the [`CostDecomposition`] with the `total` identity built in:
/// `total = work + orchestration + verification + ci` — waste shown in its
/// own line and excluded from total (ADR-0001 §2.3 JSONC).
///
/// Money is integer micro-USD (WA4): the identity is a **theorem over the
/// integers** — `total.cost_usd_micros` is the exact `checked_add` sum of the
/// four component micro-USD figures, with no floating-point epsilon. Overflow
/// is an error, never a silent wrap.
///
/// CI contributes no tokens to `total.tokens` (its spend is compute, not
/// model tokens — the [`CiCost`] shape has no token field by design).
fn decompose(
    work: WorkCost,
    orchestration: OrchestrationCost,
    verification: VerificationCost,
    ci: CiCost,
    waste: WasteCost,
) -> Result<CostDecomposition, RollupError> {
    let tokens = add(
        add(work.tokens, orchestration.tokens, "total.tokens")?,
        verification.tokens,
        "total.tokens",
    )?;
    // total = work + orchestration + verification + ci, exact integer sum.
    let cost_usd_micros = add(
        add(
            add(
                work.cost_usd_micros,
                orchestration.cost_usd_micros,
                "total.cost_usd_micros",
            )?,
            verification.cost_usd_micros,
            "total.cost_usd_micros",
        )?,
        ci.cost_usd_micros,
        "total.cost_usd_micros",
    )?;
    let total = TotalCost {
        tokens,
        cost_usd_micros,
    };
    Ok(CostDecomposition {
        work,
        orchestration,
        verification,
        ci,
        waste,
        total,
    })
}

/// Compute the [`Efficiency`] block from a decomposition + the first-pass
/// yield.
///
/// - `overhead_pct` = orchestration ÷ total, as a percentage. The money is
///   integer micro-USD (WA4); the division to a percentage is **display-only**
///   and computed into f64 at the very end (the accumulation that feeds it is
///   exact-integer). When total cost is `0` (no pricing data) it falls back to
///   the token ratio so the lean-vs-bloated signal survives without a
///   pricing seam.
/// - `cache_savings_pct` = CI saved ÷ would-be (saved + executed), as a
///   percentage; `0.0` when the checks seam supplied no USD economics.
/// - `first_pass_yield` ∈ [0, 1] — intents landed without rework ÷ intents
///   authored.
/// - `cost_per_net_kloc` = `0.0` — HONEST GAP: no projection seam carries
///   diff/LOC statistics yet (the `intent.landed` payload has only
///   ref/target/charter), so there is no net-kLOC denominator to compute
///   against. Documented, not faked.
fn efficiency_from(cost: &CostDecomposition, first_pass_yield: f64) -> Efficiency {
    let overhead_pct = if cost.total.cost_usd_micros > 0 {
        cost.orchestration.cost_usd_micros as f64 / cost.total.cost_usd_micros as f64 * 100.0
    } else if cost.total.tokens > 0 {
        cost.orchestration.tokens as f64 / cost.total.tokens as f64 * 100.0
    } else {
        0.0
    };
    // `saved + executed` can in principle overflow u64; for a display-only
    // ratio, saturate (the percentage is meaningful even at the ceiling).
    let would_be = cost
        .ci
        .saved_usd_micros
        .saturating_add(cost.ci.cost_usd_micros);
    let cache_savings_pct = if would_be > 0 {
        cost.ci.saved_usd_micros as f64 / would_be as f64 * 100.0
    } else {
        0.0
    };
    Efficiency {
        overhead_pct,
        cache_savings_pct,
        first_pass_yield,
        cost_per_net_kloc: 0.0,
    }
}

/// Reject an envelope at the wrong altitude for its input position.
fn ensure_altitude(env: &ContextEnvelope, expected: Altitude) -> Result<(), RollupError> {
    if env.altitude == expected {
        Ok(())
    } else {
        Err(RollupError::WrongAltitude {
            expected,
            found: env.altitude,
            intent_id: env.intent_id.clone(),
        })
    }
}

/// Reject a PR/campaign-altitude envelope whose session is a **subagent**
/// (D14, fail-closed): either subagent marker — a non-`"main"` `agent_type`
/// or a non-null `spawn.parent_run_id` (a spawned session) — rejects.
fn ensure_top_level(env: &ContextEnvelope) -> Result<(), RollupError> {
    let a = &env.authorship;
    if a.agent_type != TOP_LEVEL_AGENT_TYPE || a.spawn.parent_run_id.is_some() {
        return Err(RollupError::SubagentAuthor {
            altitude: env.altitude,
            intent_id: env.intent_id.clone(),
            run_id: a.spawn.run_id.clone(),
            agent_type: a.agent_type.clone(),
        });
    }
    Ok(())
}

/// Derive the [`PrAuthor`] from the PR-altitude envelope, enforcing the D14
/// rule (never a subagent) first.
///
/// Kind convention over the frozen envelope shape: a top-level session that
/// records a `model` identity is the **orchestrator** (carrying `model` +
/// `run_id`); a session with an empty `model` is a **human** session, whose
/// principal is the envelope's `authorship.operator` (the human who drives
/// the work).
fn pr_author(env: &ContextEnvelope) -> Result<PrAuthor, RollupError> {
    ensure_top_level(env)?;
    let a = &env.authorship;
    if a.model.is_empty() {
        Ok(PrAuthor {
            kind: PrAuthorKind::Human,
            model: None,
            run_id: None,
            principal: Some(a.operator.clone()),
        })
    } else {
        Ok(PrAuthor {
            kind: PrAuthorKind::Orchestrator,
            model: Some(a.model.clone()),
            run_id: Some(a.spawn.run_id.clone()),
            principal: None,
        })
    }
}

/// Group intent envelopes into attempts per intent id, preserving first-seen
/// id order (deterministic: equal inputs → equal grouping).
fn group_attempts(envelopes: &[ContextEnvelope]) -> Vec<(&str, Vec<&ContextEnvelope>)> {
    let mut groups: Vec<(&str, Vec<&ContextEnvelope>)> = Vec::new();
    for env in envelopes {
        match groups.iter_mut().find(|(id, _)| *id == env.intent_id) {
            Some((_, g)) => g.push(env),
            None => groups.push((env.intent_id.as_str(), vec![env])),
        }
    }
    groups
}

/// Index of the attempt that counts as the landed work: the **latest-born**
/// among the group's envelopes. Ties (equal `born_at`) are broken by a
/// **total order** so the work/waste split is deterministic regardless of
/// envelope arrival order:
///
/// 1. `born_at` descending — the chronologically later session wins.
/// 2. `run_id` lexicographic ascending — stable across any ordering of
///    same-timestamp siblings (run ids are unique per run).
/// 3. Input sequence ascending — last fallback; in practice unreachable
///    once `run_id` is unique, but included to make the ordering provably
///    total without relying on that external uniqueness invariant.
///
/// The first tiebreak (`born_at`) handles the normal case; `run_id` makes
/// the result independent of the input slice order even when two attempts
/// share a millisecond; the seq tiebreak closes the last gap.
fn final_attempt_idx(group: &[&ContextEnvelope]) -> usize {
    let mut best = 0usize;
    for (i, env) in group.iter().enumerate() {
        let best_env = group[best];
        let cmp_born = env
            .authorship
            .spawn
            .born_at
            .cmp(&best_env.authorship.spawn.born_at);
        let is_better = match cmp_born {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => {
                // Tiebreak 2: run_id lexicographic ascending (lower run_id
                // loses — the *higher* run_id is "later" in the stable
                // ordering; we want the *maximum* to win so we take the
                // one where the candidate's run_id is GREATER).
                let cmp_run = env
                    .authorship
                    .spawn
                    .run_id
                    .cmp(&best_env.authorship.spawn.run_id);
                match cmp_run {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Equal => {
                        // Tiebreak 3: higher input sequence wins (later
                        // in the slice — `i > best` is always true here
                        // so this is equivalent to taking `i`).
                        i > best
                    }
                    std::cmp::Ordering::Less => false,
                }
            }
            std::cmp::Ordering::Less => false,
        };
        if is_better {
            best = i;
        }
    }
    best
}

/// Push a model name if non-empty and not already present (first-seen order).
fn dedup_push(mut models: Vec<String>, model: &str) -> Vec<String> {
    if !model.is_empty() && !models.iter().any(|m| m == model) {
        models.push(model.to_string());
    }
    models
}

/// `num ÷ den` as f64, `0.0` when the denominator is zero.
fn ratio(num: u64, den: u64) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}
