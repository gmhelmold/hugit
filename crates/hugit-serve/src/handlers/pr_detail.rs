//! `GET /v1/repos/{repo}/prs/{n}` → [`PrDetailVm`] (None = 404). FROZEN signature;
//! body filled by the fleet per master-plan §5 (REAL: PR projection + cost split
//! via ledger + intents + check_rows + envelope; STUB: diff/added/removed,
//! impact, reviewers, labels, mirror, branches).
//!
//! Data-readiness (master-plan §0, the prs/{n} row): cost-real, structurally
//! sparse. REAL = number/title/state/author/session, campaign, cost split
//! (ledger `pr_record`), intents, check_rows, envelope/CAS, why/acceptance (from
//! envelope). STUB (honest defaults, never faked) = diff/added/removed/file_count
//! (no diffstat seam), impact (blast-radius not wired), source/target_branch
//! (no git-ref tracking), reviewers/labels/conversation, mirror (P2).

use crate::fmt::{CHECKS_CAP, pct_u8, scrub, scrub_all, str_field};
use hugit_cli::checks::CHECK_RECORDED_KIND;
use hugit_cli::pr::{
    INTENT_ENVELOPE_KIND, OpenedPr, PR_ABANDONED_KIND, PR_ENVELOPE_KIND, PR_LANDED_KIND,
    PR_QUEUED_KIND, find_pr_opened,
};
use hugit_contracts::context_envelope::{Altitude, CiCost, ContextEnvelope, PrRecord};
use hugit_http_contracts::common::{
    CampaignChipVm, CheckRowVm, DiffVm, EnvelopeVm, IntentSummaryVm, MirrorVm, UnionVm,
};
use hugit_http_contracts::{CostSplitVm, ImpactVm, PrDetailVm};
use hugit_ledger::{Ledger, PrQueueInput, pr_record};
use hugit_refstore::EventLog;
use serde_json::Value;
use std::collections::BTreeSet;

/// Build the PR-detail view-model for PR `pr_number`.
///
/// The `log` is ALREADY chain-verified by the caller — do NOT re-load or
/// re-verify. Returns `None` when no `pr.opened` for `pr_number` is on the log
/// (→ HTTP 404; the read is `get_opt`: not-found and no-access are
/// indistinguishable — no existence leak).
///
/// Field provenance is tagged inline: REAL (a real engine fn), PRESENTATION
/// (adapter humanizes a real datum), or STUB (no local source → honest default,
/// never faked) per master-plan §0/§5.
pub fn build_pr_detail(log: &EventLog, repo: &str, pr_number: u32) -> Option<PrDetailVm> {
    let pr_id = pr_number.to_string();

    // ── REAL: the PR must exist on the log (else 404, no existence leak) ─────
    let opened = find_pr_opened(log, &pr_id)?;

    // ── REAL: state_label — PR lifecycle state projected off the log ────────
    // Mirrors the documented `pr_state` precedence (landed ≻ abandoned ≻ queued
    // ≻ proposed); the engine's fn is private, so the projection is replicated.
    let state_label = pr_state_label(log, &pr_id).to_string();

    // ── REAL: campaign chip from the OpenedPr ───────────────────────────────
    let campaign = if opened.campaign.is_empty() {
        None
    } else {
        Some(CampaignChipVm {
            id: scrub(&opened.campaign),
            label: scrub(&opened.campaign),
            color_class: String::new(),   // STUB — no color seam
            display_label: String::new(), // serde(default)
        })
    };
    let provenance_campaign = scrub(&opened.campaign);

    // ── REAL: PR-altitude ContextEnvelope (charter→why, acceptance→note,
    //          authorship→author/session, context_ref→envelope_cas) ──────────
    let pr_env = pr_altitude_envelope(log, &pr_id);

    let (author, session, why, acceptance_note, envelope, envelope_cas) = match &pr_env {
        Some(env) => {
            let author = scrub(&env.authorship.operator); // REAL (scrubbed free text)
            let session = env.intent_id.clone(); // REAL — structural session/run id
            let why = scrub(&env.charter); // REAL (scrubbed free text)
            let acceptance_note = scrub_all(&env.acceptance).join("; "); // REAL (scrubbed)
            let envelope = envelope_vm(env); // REAL (free text scrubbed inside)
            let envelope_cas = env.snapshot.prompt_ref.clone().unwrap_or_default(); // structural ref
            (
                author,
                session,
                why,
                acceptance_note,
                envelope,
                envelope_cas,
            )
        }
        None => (
            String::new(),         // STUB — no envelope authorship on log
            String::new(),         // STUB
            String::new(),         // STUB — no charter
            String::new(),         // STUB — no acceptance
            EnvelopeVm::default(), // STUB — honest empty envelope
            String::new(),         // STUB
        ),
    };

    // ── REAL: intents in this PR's bundle (Ledger projection) ───────────────
    let intents = build_intents(log, &opened);

    // ── REAL: check_rows from check.recorded events (capped — the VM is the page) ─
    let mut check_rows = build_check_rows(log);
    check_rows.truncate(CHECKS_CAP);
    let checks_count = check_rows.len() as u32;

    // ── REAL or honest-ZERO: the F3 cost split (ledger pr_record) ───────────
    // Present only when a PR-altitude envelope is captured on the log; honest
    // ZERO (all *_usd 0.0, notes "") otherwise — NEVER faked.
    let cost = build_cost(log, &opened, pr_env.as_ref());

    // ── REAL: title from the PR's campaign + bundle (presentation) ──────────
    let title = pr_title(&opened);

    // ── Assemble ────────────────────────────────────────────────────────────
    Some(PrDetailVm {
        repo: repo.to_string(),
        number: pr_number,
        title,
        state_label,
        author,
        session,
        // STUB — no git-ref tracking seam
        source_branch: String::new(),
        target_branch: String::new(),
        models_note: String::new(), // STUB — no models seam at this altitude
        // STUB — no diffstat seam
        file_count: 0,
        added: 0,
        removed: 0,
        better_chips: vec![], // STUB
        regen_note: None,     // STUB
        why,
        acceptance_note,
        stats: vec![], // STUB — no stat strip source
        envelope,
        campaign,
        // STUB — blast-radius not wired; checks_selected/total are REAL (row count)
        impact: ImpactVm {
            crates_touched: vec![],
            direct_dependents: vec![],
            transitive_dependents: vec![],
            critical_paths_note: String::new(),
            critical_paths_target: String::new(),
            critical_paths_anchor: String::new(),
            critical_paths_safe: true,
            checks_selected: checks_count, // REAL — count of executed/checked rows
            checks_total: checks_count,    // REAL
            source_note: String::new(),
        },
        cost,
        conversation: vec![], // STUB — no Q&A seam
        intents,
        // STUB — live union-oracle is P2; honest empty/false
        union: UnionVm {
            batch: vec![],
            verdict: String::new(),
            green: false,
        },
        check_rows,
        checks_cost_note: String::new(),  // STUB — not in log payload
        checks_cache_note: String::new(), // STUB
        diff: DiffVm {
            files: vec![],
            hunks: vec![],
        }, // STUB — no diffstat seam
        landing_status: vec![],           // STUB — no landing-status (k,v) seam
        reviewers: vec![],                // STUB — no reviewer rail seam
        panel_note: String::new(),
        assignee: String::new(),  // STUB
        labels: vec![],           // STUB
        milestone: None,          // STUB
        milestone_progress: None, // STUB
        provenance_campaign,
        stack: None,                     // STUB
        change_id: opened.pr_id.clone(), // REAL — the PR's stable id
        attested_label: String::new(),   // STUB
        envelope_cas,
        transcripts_label: String::new(), // STUB
        mirror: MirrorVm {
            synced: false,
            detail: String::new(),
        }, // STUB — P2 mirror
    })
}

// ---------------------------------------------------------------------------
// Projections (replicate the documented engine folds; the engine fns are
// private, so the logic is transcribed, never re-designed).
// ---------------------------------------------------------------------------

/// PR lifecycle state, projected off the log (mirrors `pr::pr_state`):
/// `landed` ≻ `abandoned` ≻ `queued` ≻ `proposed` (most-settled wins).
fn pr_state_label(log: &EventLog, pr_id: &str) -> &'static str {
    if has_pr_event(log, PR_LANDED_KIND, pr_id) {
        "landed"
    } else if has_pr_event(log, PR_ABANDONED_KIND, pr_id) {
        "abandoned"
    } else if has_pr_event(log, PR_QUEUED_KIND, pr_id) {
        "queued"
    } else {
        "proposed"
    }
}

/// Whether the log carries an event of `kind` whose payload names `pr_id`.
fn has_pr_event(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// The latest PR-altitude [`ContextEnvelope`] captured for `pr_id`, if any
/// (mirrors `pr::envelopes_for_pr`'s PR-envelope selection).
fn pr_altitude_envelope(log: &EventLog, pr_id: &str) -> Option<ContextEnvelope> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == pr_id)
}

/// The intent-altitude envelopes attributed to this PR's bundle (mirrors
/// `pr::envelopes_for_pr`'s intent selection).
fn intent_altitude_envelopes(log: &EventLog, opened: &OpenedPr) -> Vec<ContextEnvelope> {
    let bundle: BTreeSet<&str> = opened.intent_ids.iter().map(String::as_str).collect();
    log.records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .filter(|e| e.altitude == Altitude::Intent && bundle.contains(e.intent_id.as_str()))
        .collect()
}

/// Map a PR-altitude [`ContextEnvelope`] into the wire [`EnvelopeVm`] — REAL.
fn envelope_vm(env: &ContextEnvelope) -> EnvelopeVm {
    let summary = env.trajectory.summary.clone().unwrap_or_default();
    let headline = scrub(&summary);
    EnvelopeVm {
        session: env.intent_id.clone(),      // structural
        model: scrub(&env.authorship.model), // free text — scrubbed
        window: String::new(),               // STUB — no window seam
        headline: headline.clone(),          // free text — scrubbed
        transcript_complete: env.trajectory.raw_transcript_ref.is_some()
            && env.trajectory.task_transcript_ref.is_some(),
        session_summary: headline, // same scrubbed summary text
        compact_transcript_ref: env
            .trajectory
            .task_transcript_ref
            .clone()
            .unwrap_or_default(),
        raw_transcript_ref: env
            .trajectory
            .raw_transcript_ref
            .clone()
            .unwrap_or_default(),
        snapshot_files: env
            .snapshot
            .files_read
            .iter()
            .map(|f| scrub(&f.path)) // free text — scrubbed
            .collect(),
        context_json: String::new(), // STUB — no pretty-printed context.json here
        context_cas: env.snapshot.prompt_ref.clone().unwrap_or_default(),
        compact_context_ref: String::new(), // STUB
        compact_json_note: String::new(),   // STUB
        bundle_note: String::new(),         // STUB
    }
}

/// The PR's intents (Ledger projection), filtered to the PR's bundle — REAL.
fn build_intents(log: &EventLog, opened: &OpenedPr) -> Vec<IntentSummaryVm> {
    let bundle: BTreeSet<&str> = opened.intent_ids.iter().map(String::as_str).collect();
    let ledger = Ledger::from_records(log.records());
    ledger
        .entries()
        .iter()
        .filter(|e| bundle.contains(e.intent_id.as_str()))
        .map(|e| IntentSummaryVm {
            id: e.intent_id.clone(),
            title: e.charter.clone(),
            status: if e.rejected {
                "REJECTED".to_string()
            } else if e.proven {
                "PROVEN".to_string()
            } else {
                "LANDED".to_string()
            },
            charter: e.charter.clone(),
            context_json: String::new(), // STUB — no context.json snapshot here
            diff: DiffVm {
                files: vec![],
                hunks: vec![],
            }, // STUB — no diffstat seam
            verdicts: vec![],            // STUB — verdict detail not projected here
            model: None,                 // STUB
            envelope: None,              // STUB — intent envelope refs not surfaced here
            blame_quote: None,           // serde(default)
            blame_proof: None,           // serde(default)
        })
        .collect()
}

/// The check rows from `check.recorded` events — REAL (same projection as the
/// checks handler).
fn build_check_rows(log: &EventLog) -> Vec<CheckRowVm> {
    log.records()
        .iter()
        .filter(|r| r.kind == CHECK_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .map(|v| {
            let ok = v
                .get("exit")
                .and_then(Value::as_i64)
                .map(|e| e == 0)
                .unwrap_or(false);
            CheckRowVm {
                name: scrub(&str_field(&v, "name").unwrap_or_default()), // REAL (scrubbed read-boundary)
                ok,                                                      // REAL (exit==0)
                duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0), // REAL
                cache_hit: v.get("cache_hit").and_then(Value::as_bool).unwrap_or(false), // REAL
                log: String::new(),                                      // STUB — not in payload
                memo_key: str_field(&v, "memo_key").unwrap_or_default(), // REAL
                reason: String::new(),                                   // STUB
                cost: String::new(),                                     // STUB
            }
        })
        .collect()
}

/// The cost split — REAL from `pr_record` when a PR-altitude envelope is
/// captured, honest ZERO otherwise (all `*_usd` 0.0, notes ""). NEVER faked.
fn build_cost(log: &EventLog, opened: &OpenedPr, pr_env: Option<&ContextEnvelope>) -> CostSplitVm {
    let record = pr_env.and_then(|env| {
        let intents = intent_altitude_envelopes(log, opened);
        // No CI / queue / verdict seam at this altitude (P2 disclosed) — passed
        // zero/default exactly as the engine's `pr_record_for` does; the rollup
        // computes work/orchestration/waste/first-pass-yield from the envelopes.
        pr_record(
            env,
            "", // no CAS binding at this altitude — threaded empty, not faked
            &intents,
            &opened.intent_ids,
            &[],
            CiCost {
                cache_hit: 0,
                exec: 0,
                cost_usd_micros: 0,
                saved_usd_micros: 0,
            },
            PrQueueInput::default(),
        )
        .ok() // a subagent-authored PR envelope is rejected → honest ZERO
    });

    match record {
        Some(r) => cost_split_from_record(&r),
        None => zero_cost(),
    }
}

/// Micro-USD → USD (`1 USD = 1_000_000`).
fn usd(micros: u64) -> f64 {
    micros as f64 / 1_000_000.0
}

/// Map a computed [`PrRecord`] into the wire [`CostSplitVm`]. The `*_usd` figures
/// are REAL; the notes/labels are PRESENTATION (short honest strings).
fn cost_split_from_record(r: &PrRecord) -> CostSplitVm {
    let c = &r.cost;
    CostSplitVm {
        author_line: String::new(), // PRESENTATION — no author line composed here
        author_badge: None,
        author_suffix: None,
        work_usd: usd(c.work.cost_usd_micros), // REAL
        work_note: format!("{} intents", r.intent_count),
        orchestration_usd: usd(c.orchestration.cost_usd_micros), // REAL
        orchestration_note: String::new(),
        verification_usd: usd(c.verification.cost_usd_micros), // REAL
        verification_note: format!("{} painéis", c.verification.verdict_panels),
        ci_usd: usd(c.ci.cost_usd_micros), // REAL
        ci_note: String::new(),
        total_usd: usd(c.total.cost_usd_micros), // REAL
        waste_usd: usd(c.waste.cost_usd_micros), // REAL
        waste_note: String::new(),
        // The efficiency ratios are fractions in `[0,1]` per ADR-0001 — scale to
        // percent at the call site; `pct_u8` clamps to 0..=100 (NOT 255).
        overhead_pct: pct_u8(r.efficiency.overhead_pct * 100.0), // REAL
        cache_savings_pct: pct_u8(r.efficiency.cache_savings_pct * 100.0), // REAL
        time_note: String::new(), // PRESENTATION — no time line composed here
    }
}

/// The honest-ZERO cost block (no PR-altitude envelope on the log).
fn zero_cost() -> CostSplitVm {
    CostSplitVm {
        author_line: String::new(),
        author_badge: None,
        author_suffix: None,
        work_usd: 0.0,
        work_note: String::new(),
        orchestration_usd: 0.0,
        orchestration_note: String::new(),
        verification_usd: 0.0,
        verification_note: String::new(),
        ci_usd: 0.0,
        ci_note: String::new(),
        total_usd: 0.0,
        waste_usd: 0.0,
        waste_note: String::new(),
        overhead_pct: 0,
        cache_savings_pct: 0,
        time_note: String::new(),
    }
}

/// A presentation title for the PR (no stored title field — composed honestly
/// from the campaign + bundle size).
fn pr_title(opened: &OpenedPr) -> String {
    let n = opened.intent_ids.len();
    // `pr_id` is payload-derived free text (not a router-validated u32), so the
    // WHOLE composed title is scrubbed at the read boundary — a secret-shaped
    // pr_id embedded here would otherwise echo verbatim.
    let raw = if opened.campaign.is_empty() {
        format!("PR #{} — {} intents", opened.pr_id, n)
    } else {
        format!("PR #{} ({}) — {} intents", opened.pr_id, opened.campaign, n)
    };
    scrub(&raw)
}
