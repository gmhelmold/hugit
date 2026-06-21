//! `GET /v1/repos/{repo}/landing` → [`LandingVm`]. FROZEN signature; body filled
//! per master-plan §0/§5 source map. REAL: PR list/state/queue-position +
//! campaign chips + cost (ledger `pr_record` rollup, micro-USD→usd) + drawer
//! intents (ledger). PRESENTATION: list_badge / date (humanized). STUB (no local
//! engine seam): main_green/main_status (NO main-CI seam), file_count + drawer
//! files (no diffstat seam), mirror (P2), union (best-effort empty), list_groups
//! (no fleet grouping), draft_count (no draft state), stack/change_id/landed_ago.

use crate::fmt::{PR_CARDS_CAP, humanize_age, scrub};
use hugit_cli::pr::{
    CAMPAIGN_OPENED_KIND, INTENT_ENVELOPE_KIND, OpenedPr, PR_ABANDONED_KIND, PR_ENVELOPE_KIND,
    PR_LANDED_KIND, PR_OPENED_KIND, all_pr_queued, find_pr_opened,
};
use hugit_contracts::context_envelope::{Altitude, CiCost, ContextEnvelope};
use hugit_http_contracts::common::{
    CampaignChipVm, CostVm, IntentSummaryVm, MirrorVm, UnionVm, VerdictVm,
};
use hugit_http_contracts::landing::{
    ChecksBadgeVm, LandingColumnVm, LandingItemVm, PrCardVm, PrDrawerVm, PrState,
};
use hugit_http_contracts::{DiffVm, LandingVm};
use hugit_ledger::rollup::{PrQueueInput, pr_record};
use hugit_ledger::{Ledger, LedgerEntry};
use hugit_refstore::EventLog;
use serde_json::Value;

/// The lifecycle state of a PR projected off the log — mirrors the porcelain's
/// `pr_state` precedence (landed ≻ abandoned ≻ queued ≻ proposed). Read straight
/// off raw record kinds so the projection never re-hand-rolls authority.
fn pr_state(log: &EventLog, opened: &OpenedPr) -> PrState {
    if record_names_pr(log, PR_LANDED_KIND, &opened.pr_id) {
        PrState::Landed
    } else if record_names_pr(log, PR_ABANDONED_KIND, &opened.pr_id) {
        // No distinct Blocked source; an abandoned PR is terminal-not-landed.
        // The card surfaces it as Blocked (the "not advancing" column) — honest
        // mapping of the only terminal-non-landed signal we have.
        PrState::Blocked
    } else if all_pr_queued(log).iter().any(|q| q.pr_id == opened.pr_id) {
        PrState::Queued
    } else {
        PrState::Open
    }
}

/// Whether a record of `kind` names `pr_id` in its `pr_id` payload field.
fn record_names_pr(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// The recorded queue `order_index` of a PR, if it is actively queued.
fn queue_position(log: &EventLog, pr_id: &str) -> Option<u64> {
    all_pr_queued(log)
        .into_iter()
        .find(|q| q.pr_id == pr_id)
        .map(|q| q.order_index)
}

/// Map a PR's lifecycle state to the column it belongs in (pt-BR labels per the
/// §0 source map). Open/Queued → "Na fila"; queued-and-actively-tested has no
/// distinct local signal, so "Testando" stays empty by construction (honest:
/// no test-in-progress seam); Blocked (abandoned) → "Bloqueado"; Landed →
/// "Pousado hoje".
fn column_label(state: &PrState) -> &'static str {
    match state {
        PrState::Open | PrState::Queued | PrState::Draft => "Na fila",
        PrState::Testing => "Testando",
        PrState::Blocked => "Bloqueado",
        PrState::Landed => "Pousado hoje",
    }
}

/// The four landing columns, in display order.
const COLUMN_TITLES: [&str; 4] = ["Na fila", "Testando", "Bloqueado", "Pousado hoje"];

/// Read the PR-altitude + matching intent-altitude envelopes for a PR off the
/// log (read-side projection mirroring the porcelain's `envelopes_for_pr`).
/// `None` when no PR-altitude envelope was captured (→ honest-zero cost).
fn envelopes_for_pr(
    log: &EventLog,
    opened: &OpenedPr,
) -> Option<(ContextEnvelope, Vec<ContextEnvelope>)> {
    let bundle: std::collections::BTreeSet<&str> =
        opened.intent_ids.iter().map(String::as_str).collect();

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

    Some((pr, intents))
}

/// The REAL cost block for a PR via the F3 [`pr_record`] rollup — `usd` is the
/// total micro-USD divided to dollars; `tokens_total` + `model_breakdown` come
/// from the rollup. Honest-ZERO ([`zero_cost`]) when no PR-altitude envelope is
/// on the log (the typical/empty log) or the rollup rejects (e.g. a subagent
/// author) — NEVER faked.
fn cost_for(log: &EventLog, opened: &OpenedPr) -> CostVm {
    let Some((pr_env, intent_envs)) = envelopes_for_pr(log, opened) else {
        return zero_cost();
    };
    let landed: Vec<String> = opened.intent_ids.clone();
    let record = pr_record(
        &pr_env,
        "",
        &intent_envs,
        &landed,
        &[],
        CiCost {
            cache_hit: 0,
            exec: 0,
            cost_usd_micros: 0,
            saved_usd_micros: 0,
        },
        PrQueueInput::default(),
    );
    match record {
        Ok(rec) => CostVm {
            tokens_total: rec.cost.total.tokens,
            usd: rec.cost.total.cost_usd_micros as f64 / 1_000_000.0,
            // HONEST: the rollup carries no per-model token split, so emitting
            // `(model, 0)` would be a misleading stub-zero. Empty is honest.
            model_breakdown: vec![],
            cache_savings: String::new(), // STUB — no live-AC savings string seam
        },
        Err(_) => zero_cost(),
    }
}

/// The honest-ZERO cost block (no faked figures): used when no envelope/rollup
/// data is present on the log.
fn zero_cost() -> CostVm {
    CostVm {
        tokens_total: 0,
        usd: 0.0,
        model_breakdown: vec![],
        cache_savings: String::new(),
    }
}

/// REAL drawer intents for a PR: the ledger entries (`intent.landed` +
/// `verdict.recorded`) whose `intent_id` is in the PR's bundle. Charter →
/// `title`/`charter`; proven/rejected → `status`; recorded verdict → `verdicts`.
/// STUB per intent (no local seam): `diff` (no diffstat), `context_json`,
/// `envelope`, blame. Empty when the PR's intents are not on the ledger (empty
/// log).
fn drawer_intents(ledger: &Ledger, opened: &OpenedPr) -> Vec<IntentSummaryVm> {
    let bundle: std::collections::BTreeSet<&str> =
        opened.intent_ids.iter().map(String::as_str).collect();
    ledger
        .entries()
        .iter()
        .filter(|e| bundle.contains(e.intent_id.as_str()))
        .map(intent_summary)
        .collect()
}

/// Map one ledger entry to an [`IntentSummaryVm`] (honest defaults for the
/// no-seam fields).
fn intent_summary(entry: &LedgerEntry) -> IntentSummaryVm {
    let status = if entry.rejected {
        "REJECTED"
    } else if entry.proven {
        "PROVEN"
    } else {
        "LANDED"
    }
    .to_string();

    let verdicts: Vec<VerdictVm> = entry
        .verdict
        .iter()
        .map(|v| VerdictVm {
            verdict: v.outcome.clone(),
            reviewer: String::new(), // STUB — VerdictView carries no reviewer id
            summary: String::new(),  // STUB — no summary on the ledger view
            adversarial: false,      // STUB — panel-source not on the view
            lens: v.lens.clone(),    // REAL
            evidence_mono_terms: v.claims_checked.clone(), // REAL
        })
        .collect();

    IntentSummaryVm {
        id: entry.intent_id.clone(),    // REAL
        title: entry.charter.clone(),   // REAL (charter)
        status,                         // REAL (proven/rejected)
        charter: entry.charter.clone(), // REAL
        context_json: String::new(),    // STUB — no context snapshot seam
        diff: DiffVm {
            files: vec![],
            hunks: vec![],
        }, // STUB — no diffstat seam
        verdicts,                       // REAL (when a verdict is recorded)
        model: None,                    // STUB — not on the ledger view
        envelope: None,                 // STUB — no per-intent envelope ref seam
        blame_quote: None,              // STUB
        blame_proof: None,              // STUB
    }
}

/// Build one PR card from a projected PR.
fn build_card(
    log: &EventLog,
    ledger: &Ledger,
    opened: &OpenedPr,
    campaign_chip: Option<CampaignChipVm>,
) -> PrCardVm {
    let state = pr_state(log, opened);
    let pos = queue_position(log, &opened.pr_id);

    // PRESENTATION: list_badge from the queue position (the wedge's "na fila #N").
    // Display is 1-based ("na fila #1") to match the mock; the engine's
    // `order_index` is 0-based, hence the `p + 1`.
    let list_badge = match pos {
        Some(p) => format!("na fila #{}", p + 1),
        None => String::new(),
    };

    // PRESENTATION: date from the pr.opened record's recorded_at.
    let date = log
        .records()
        .iter()
        .find(|r| r.kind == PR_OPENED_KIND && r.seq == opened.seq)
        .map(|r| humanize_age(r.recorded_at))
        .unwrap_or_default();

    // number: the pr_id parsed as a number when it is numeric, else 0 (honest —
    // a content-address pr_id has no card number).
    let number = opened.pr_id.parse::<u64>().unwrap_or(0);

    PrCardVm {
        number,                                // REAL (when numeric)
        title: String::new(),                  // STUB — no PR title seam on the log
        author: String::new(),                 // STUB — no envelope authorship by default
        model: String::new(),                  // STUB — no envelope authorship by default
        campaign: campaign_chip,               // REAL (Option)
        intent_count: opened.intent_ids.len(), // REAL
        file_count: 0,                         // STUB — no diffstat seam
        checks: ChecksBadgeVm {
            passed: 0,
            total: 0,
            cache_hits: 0,
        }, // STUB — checks badge belongs to the checks read
        state,                                 // REAL
        stack: None,                           // STUB — no stack seam
        date,                                  // PRESENTATION
        list_badge,                            // PRESENTATION
        change_id: String::new(),              // STUB
        landed_ago: String::new(),             // STUB
        drawer: PrDrawerVm {
            union: UnionVm {
                batch: vec![],          // STUB/best-effort — no live union-oracle (P2)
                verdict: String::new(), // honest — no verdict without the oracle
                green: false,           // honest default
            },
            cost: cost_for(log, opened), // REAL (ledger rollup) / honest-zero
            mirror: MirrorVm {
                synced: false,         // STUB — P2 mirror seam
                detail: String::new(), // STUB — P2
            },
            intents: drawer_intents(ledger, opened), // REAL (from ledger)
            files: vec![],                           // STUB — no diffstat seam
            conflict_note: None,                     // STUB
            summary_diff: None,                      // STUB
        },
    }
}

/// Project the campaign chips off `campaign.opened` records (REAL).
fn campaign_chips(log: &EventLog) -> Vec<CampaignChipVm> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut chips: Vec<CampaignChipVm> = Vec::new();
    for r in log
        .records()
        .iter()
        .filter(|r| r.kind == CAMPAIGN_OPENED_KIND)
    {
        if let Ok(v) = serde_json::from_str::<Value>(&r.payload)
            && let Some(id) = v.get("campaign").and_then(Value::as_str)
            && seen.insert(id.to_string())
        {
            // P1 LEAK FIX: the campaign id is raw free-text from the log
            // envelope — scrub it before it reaches id/label/display_label so a
            // secret-shaped campaign id never echoes verbatim to the browser.
            let safe = scrub(id);
            chips.push(CampaignChipVm {
                id: safe.clone(),
                label: safe.clone(), // REAL id (scrubbed); no separate human label seam
                color_class: String::new(), // STUB — no kit color seam on the log
                display_label: safe,
            });
        }
    }
    chips
}

/// Project the latest `pr.opened` per id, in first-seen (open seq) order.
fn ordered_open_prs(log: &EventLog) -> Vec<OpenedPr> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut ordered: Vec<OpenedPr> = Vec::new();
    for r in log.records().iter().filter(|r| r.kind == PR_OPENED_KIND) {
        if let Ok(v) = serde_json::from_str::<Value>(&r.payload)
            && let Some(id) = v.get("pr_id").and_then(Value::as_str)
            && seen.insert(id.to_string())
            && let Some(latest) = find_pr_opened(log, id)
        {
            ordered.push(latest);
        }
    }
    ordered
}

/// The 1-based queue position of the first actively-queued PR (the "head of
/// the queue"), or `None` when the active queue is empty.
///
/// `all_pr_queued` returns only non-landed/non-abandoned PRs ordered by
/// `order_index`; the first entry is the PR that will land next. We expose
/// position 1 (the head) at the `LandingVm` level as a global queue signal.
fn landing_queue_position(log: &EventLog) -> Option<u32> {
    let queued = all_pr_queued(log);
    if queued.is_empty() {
        None
    } else {
        // The smallest order_index in the active queue is the head of the queue.
        // Display as 1-based (matches the per-card `list_badge` convention).
        Some(1)
    }
}

/// Build the landing view-model from a verified event log.
///
/// The `log` is ALREADY chain-verified by the caller — do NOT re-load or
/// re-verify. REAL fields are sourced from the PR projection / ledger / campaign
/// records; STUB fields have no local engine source and carry the honest default
/// (per master-plan §0/§5). Nothing is faked.
pub fn build_landing(log: &EventLog, repo: &str) -> LandingVm {
    let prs = ordered_open_prs(log);
    let chips = campaign_chips(log);
    let ledger = Ledger::from_records(log.records());

    // Build a card per PR, attaching its campaign chip when its campaign matches
    // a known campaign.opened (else None — honest).
    let mut cards_by_column: std::collections::BTreeMap<&'static str, Vec<LandingItemVm>> =
        std::collections::BTreeMap::new();

    let mut open_count = 0usize;
    let mut merged_count = 0usize;

    // P1 list cap: bound the total cards projected across all columns (fail-honest
    // — the VM IS the page). One PR yields exactly one card, so capping the
    // iterated PRs caps the emitted cards.
    for opened in prs.iter().take(PR_CARDS_CAP) {
        let state = pr_state(log, opened);
        match state {
            PrState::Landed => merged_count += 1,
            // open = opened-not-terminal (proposed/queued/testing).
            PrState::Open | PrState::Queued | PrState::Testing | PrState::Draft => open_count += 1,
            PrState::Blocked => {} // abandoned: terminal-not-landed, not "open"
        }

        // Chip ids are scrubbed (P1), so match against the scrubbed campaign id.
        let want = scrub(&opened.campaign);
        let chip = chips.iter().find(|c| c.id == want).cloned();
        let card = build_card(log, &ledger, opened, chip);
        let label = column_label(&state);
        cards_by_column
            .entry(label)
            .or_default()
            .push(LandingItemVm::Card(Box::new(card)));
    }

    // Emit all four columns in stable display order (empty columns kept so the
    // window renders the full board honestly).
    let columns: Vec<LandingColumnVm> = COLUMN_TITLES
        .iter()
        .map(|title| LandingColumnVm {
            title: title.to_string(),
            items: cards_by_column.remove(*title).unwrap_or_default(),
            orq_model: String::new(), // STUB — no fleet model seam
            window: String::new(),    // STUB — no window seam
            cost: String::new(),      // STUB — no per-column cost seam
        })
        .collect();

    // F5 — queue position: REAL from the active queue projection.
    let queue_position = landing_queue_position(log);

    LandingVm {
        repo: repo.to_string(),     // REAL
        main_green: false,          // STUB — NO local main-CI status seam
        main_status: String::new(), // STUB — NO local main-CI status seam
        open_count,                 // REAL
        merged_count,               // REAL
        draft_count: 0,             // STUB — no draft state in the engine
        columns,                    // REAL (PR cards grouped by state)
        campaigns: chips,           // REAL
        list_groups: vec![],        // STUB — no fleet grouping seam
        // honest column labels as filter pills (the 3 active board columns).
        filter_pills: vec![
            "Na fila".to_string(),
            "Bloqueado".to_string(),
            "Pousado hoje".to_string(),
        ],
        // F5 — queue wedge fields.
        queue_position,    // REAL — head of the active queue; None when empty
        eta_seconds: None, // HONEST-None — no timing estimator seam yet
    }
}
