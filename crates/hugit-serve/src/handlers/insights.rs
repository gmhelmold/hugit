//! `GET /v1/repos/{repo}/insights` → [`InsightsVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL backbone: the Ledger
//! (entries grouped by campaign → counts + rows) + cost X-ray (per-campaign
//! rollup via ContextEnvelope/`pr_record`) + landed-by-day + KPIs (derived from
//! ledger counts). Honest defaults (None/[]) for totals/decomp/contrib/timing
//! with no local seam. Presentation strings are DERIVED from real numbers,
//! never invented. Every free-text field passes `crate::fmt::scrub`.

use crate::fmt::{PR_CARDS_CAP, humanize_age, pct_u8, scrub};
use hugit_cli::pr::{
    CAMPAIGN_OPENED_KIND, INTENT_ENVELOPE_KIND, OpenedPr, PR_ENVELOPE_KIND, PR_OPENED_KIND,
    find_pr_opened,
};
use hugit_contracts::context_envelope::{Altitude, CiCost, ContextEnvelope};
use hugit_http_contracts::common::{KpiSubKind, KpiVm};
use hugit_http_contracts::insights::{
    CostXrayRowVm, LedgerCampaignVm, LedgerRowVm, LedgerViewVm, XrayDrillRowVm, XrayTotalsVm,
};
use hugit_http_contracts::{CampaignChipVm, InsightsVm};
use hugit_ledger::envelope::cold_ref_for;
use hugit_ledger::rollup::{PrQueueInput, pr_record};
use hugit_ledger::{Ledger, LedgerEntry};
use serde_json::Value;

// ── civil-day helpers (mirror commits.rs) ────────────────────────────────────

fn day_key_from_ms(unix_ms: u64) -> (i32, u32, u32) {
    let secs = (unix_ms / 1000) as i64; // safe: u64::MAX/1000 < i64::MAX
    // CLAMP, never wrap: `recorded_at` is excluded from the chain pre-image, so a
    // corrupt/forged far-future value can ride a chain-valid log. `as i32` would
    // wrap (release) / panic (debug); try_from clamps to a sentinel instead.
    let days = i32::try_from(secs.div_euclid(86400)).unwrap_or(i32::MAX);
    civil_date(days)
}

fn civil_date(z: i32) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn pt_br_month(month: u32) -> &'static str {
    match month {
        1 => "jan",
        2 => "fev",
        3 => "mar",
        4 => "abr",
        5 => "mai",
        6 => "jun",
        7 => "jul",
        8 => "ago",
        9 => "set",
        10 => "out",
        11 => "nov",
        12 => "dez",
        _ => "???",
    }
}

fn format_short_day_label(year: i32, month: u32, day: u32) -> String {
    format!("{day:02} {} {year}", pt_br_month(month))
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

// ── campaign chips (mirror landing.rs, scrubbed) ─────────────────────────────

fn campaign_chips(log: &EventLog) -> Vec<CampaignChipVm> {
    let mut seen = std::collections::BTreeSet::new();
    let mut chips = Vec::new();
    for r in log
        .records()
        .iter()
        .filter(|r| r.kind == CAMPAIGN_OPENED_KIND)
    {
        if let Ok(v) = serde_json::from_str::<Value>(&r.payload)
            && let Some(id) = v.get("campaign").and_then(Value::as_str)
            && seen.insert(id.to_string())
        {
            let safe = scrub(id);
            chips.push(CampaignChipVm {
                id: safe.clone(),
                label: safe.clone(),
                color_class: String::new(),
                display_label: safe,
            });
        }
    }
    chips
}

use hugit_refstore::EventLog;

fn ledger_campaign_ids(ledger: &Ledger) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut order = Vec::new();
    for e in ledger.entries() {
        if seen.insert(e.campaign.clone()) {
            order.push(e.campaign.clone());
        }
    }
    order
}

fn ordered_open_prs(log: &EventLog) -> Vec<OpenedPr> {
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();
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

fn envelopes_for_pr(
    log: &EventLog,
    opened: &OpenedPr,
) -> Option<(ContextEnvelope, String, Vec<ContextEnvelope>)> {
    let bundle: std::collections::BTreeSet<&str> =
        opened.intent_ids.iter().map(String::as_str).collect();
    let (pr, raw_payload) = log
        .records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| {
            let env = serde_json::from_str::<ContextEnvelope>(&r.payload).ok()?;
            Some((env, r.payload.clone()))
        })
        .rfind(|(e, _)| e.altitude == Altitude::Pr && e.intent_id == opened.pr_id)?;
    let intents = log
        .records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .filter(|e| e.altitude == Altitude::Intent && bundle.contains(e.intent_id.as_str()))
        // WP-COST read-time fold: an intent envelope whose cost is honest-zero is
        // augmented from its raw `ctx.usage` records BEFORE it reaches `pr_record`,
        // so the cost X-ray + spend_proof rows light with no re-land. The rollup
        // then rolls work (intents) + orchestration (pr) up naturally. Each
        // ctx.usage targets an intent XOR a pr, so the two never double-count the
        // same record. Double-count guard + complete-or-nothing live in the helper.
        .map(|e| {
            let id = e.intent_id.clone();
            super::usage_fold::envelope_with_usage(e, log, "intent", &id)
        })
        .collect();
    // The PR-altitude (orchestration) envelope gets the same zero-guarded augment.
    let pr = super::usage_fold::envelope_with_usage(pr, log, "pr", &opened.pr_id);
    Some((pr, raw_payload, intents))
}

fn zero_ci() -> CiCost {
    CiCost {
        cache_hit: 0,
        exec: 0,
        cost_usd_micros: 0,
        saved_usd_micros: 0,
    }
}

// ── cost X-ray (per campaign) ─────────────────────────────────────────────────

/// The cost X-ray outputs derived from one pass over the PR rollups: the table
/// rows, the per-campaign (chip, raw-tokens, cost) triples, and the grand totals.
struct XrayOut {
    rows: Vec<CostXrayRowVm>,
    tokens_by_campaign: Vec<(CampaignChipVm, u64, String)>,
    totals: Option<XrayTotalsVm>,
}

fn build_cost_xray(log: &EventLog, chips: &[CampaignChipVm]) -> XrayOut {
    let mut prs = ordered_open_prs(log);
    if prs.is_empty() || chips.is_empty() {
        return XrayOut {
            rows: vec![],
            tokens_by_campaign: vec![],
            totals: None,
        };
    }
    // Cap per-request work (DoS bound): each PR triggers two full log scans.
    prs.truncate(PR_CARDS_CAP);

    let mut by_campaign: std::collections::BTreeMap<String, Vec<&OpenedPr>> =
        std::collections::BTreeMap::new();
    for opened in &prs {
        by_campaign
            .entry(scrub(&opened.campaign))
            .or_default()
            .push(opened);
    }

    let mut rows = Vec::new();
    let mut tokens_by_campaign = Vec::new();
    let (mut g_tokens, mut g_micros, mut g_waste, mut g_prs, mut g_intents) =
        (0u64, 0u64, 0u64, 0usize, 0usize);

    for chip in chips {
        let Some(prs_for) = by_campaign.get(&chip.id) else {
            continue;
        };
        let mut total_tokens = 0u64;
        let mut total_micros = 0u64;
        let mut waste_micros = 0u64;
        let mut drill_rows = Vec::new();
        let mut pr_count = 0usize;
        let mut intent_count = 0usize;
        let mut spend_proof: Option<String> = None;
        for opened in prs_for {
            pr_count += 1;
            intent_count += opened.intent_ids.len();
            let Some((pr_env, raw_payload, intent_envs)) = envelopes_for_pr(log, opened) else {
                continue;
            };
            spend_proof = Some(cold_ref_for(raw_payload.as_bytes()));
            if let Ok(rec) = pr_record(
                &pr_env,
                "",
                &intent_envs,
                &opened.intent_ids,
                &[],
                zero_ci(),
                PrQueueInput::default(),
            ) {
                let micros = rec.cost.total.cost_usd_micros;
                total_tokens += rec.cost.total.tokens;
                total_micros += micros;
                waste_micros += rec.cost.waste.cost_usd_micros;
                // Per-PR first-pass yield is sound (no cross-PR aggregation).
                let fp = rec.efficiency.first_pass_yield;
                let first_pass = if fp >= 0.999 {
                    "✓ first-pass".to_string()
                } else {
                    format!("{}% first-pass", pct_u8(fp * 100.0))
                };
                drill_rows.push(XrayDrillRowVm {
                    pr_ref: scrub(&format!("#{}", opened.pr_id)),
                    pr_url: String::new(),
                    intent_label: format!("{} intents", opened.intent_ids.len()),
                    cost: format!("${:.2}", micros as f64 / 1_000_000.0),
                    // cache-$ saved needs CI cost data (not on the log) — honest "".
                    saving: String::new(),
                    first_pass, // REAL — per-PR yield
                    in_flight: false,
                });
            }
        }
        if drill_rows.is_empty() {
            continue;
        }
        let cost_total = format!("${:.2}", total_micros as f64 / 1_000_000.0);
        rows.push(CostXrayRowVm {
            campaign: chip.clone(),
            tokens: format_tokens(total_tokens),
            prs_int: format!("{pr_count}·{intent_count}"),
            decomp_pcts: (0, 0, 0, 0), // STUB — no decomp split seam
            cost_total: cost_total.clone(),
            // REAL — sum of per-PR waste (a sum is sound for dollars).
            waste: format!("${:.2}", waste_micros as f64 / 1_000_000.0),
            cache_saved: String::new(), // honest — no cache-$ seam (only %); never conflated
            first_pass: String::new(), // honest — cross-PR yield not soundly aggregable at row level
            drill_rows,
            // F4a — raw integer fields (REAL — same source as the string fields above).
            tokens_count: total_tokens,
            cost_total_micros: total_micros,
            waste_micros,
            cache_saved_micros: 0,      // HONEST-ZERO — no cache-$ seam yet
            spend_proof,                // REAL — cas: ref of the PR-altitude envelope raw payload
            cache_efficiency_pct: None, // HONEST-None — no CI-cost/cache seam yet
        });
        tokens_by_campaign.push((chip.clone(), total_tokens, cost_total));
        g_tokens += total_tokens;
        g_micros += total_micros;
        g_waste += waste_micros;
        g_prs += pr_count;
        g_intents += intent_count;
    }

    let totals = if rows.is_empty() {
        None
    } else {
        Some(XrayTotalsVm {
            tokens: format_tokens(g_tokens),
            prs_int: format!("{g_prs}·{g_intents}"),
            cost: format!("${:.2}", g_micros as f64 / 1_000_000.0),
            waste: format!("${:.2}", g_waste as f64 / 1_000_000.0),
            cache_saved: String::new(), // honest
            first_pass: String::new(),  // honest
            // F4a — raw integer totals (REAL — same source as the string fields above).
            tokens_count: g_tokens,
            cost_micros: g_micros,
            waste_micros: g_waste,
            cache_saved_micros: 0, // HONEST-ZERO — no cache-$ seam yet
        })
    };
    XrayOut {
        rows,
        tokens_by_campaign,
        totals,
    }
}

// ── ledger view ───────────────────────────────────────────────────────────────

fn ledger_row_from_entry(
    log: &EventLog,
    entry: &LedgerEntry,
    intent_spend_map: &std::collections::HashMap<String, String>,
) -> LedgerRowVm {
    let done_status = if entry.rejected {
        "rejected"
    } else {
        "mergeado"
    }
    .to_string();
    let proven_status = if entry.proven {
        "verde"
    } else if entry.rejected {
        "vermelho"
    } else {
        "pendente"
    }
    .to_string();
    let verdict = entry.verdict.as_ref().map(|v| scrub(&v.outcome));
    let verdict_chips: Vec<String> = entry
        .verdict
        .iter()
        .map(|v| scrub(&format!("{} {}", v.lens, v.outcome)))
        .collect();
    // WP-COST read-time fold — see the cost_micros/tokens_count fields below.
    let (cost_micros, tokens_count) =
        match super::usage_fold::priced_usage_for(log, "intent", &entry.intent_id) {
            Some(priced) => (priced.cost_usd_micros, priced.tokens.total),
            None => (0, 0),
        };
    LedgerRowVm {
        intent_id: entry.intent_id.clone(),
        asked: scrub(&entry.charter),
        done_status,
        proven_status,
        verdict,
        when: humanize_age(entry.recorded_at), // REAL — from the ledger entry timestamp
        model: String::new(),
        done_body: String::new(),
        pr_number: None,
        intent_chips: vec![entry.intent_id.clone()],
        verdict_chips,
        proof_note: String::new(),
        cost: String::new(),
        savings: String::new(),
        // F4a — raw integer cost fields.
        // The ledger view (`intent.landed` / `verdict.recorded`) carries no cost
        // figures of its own — cost lives in the PR-altitude envelope. WP-COST
        // read-time fold: price this intent's raw `ctx.usage` records so a posted
        // usage record lights the per-intent cost + token columns with no re-land.
        // Complete-or-nothing (any unknown model / malformed ⇒ None ⇒ stays 0);
        // this row is always honest-zero pre-fold, so there is nothing to stack on.
        cost_micros,
        savings_micros: 0,
        tokens_count,
        spend_proof: intent_spend_map.get(&entry.intent_id).cloned(), // REAL — cas: ref from matching intent.envelope record
    }
}

fn build_ledger_view(
    log: &EventLog,
    ledger: &Ledger,
    chips: &[CampaignChipVm],
    intent_spend_map: &std::collections::HashMap<String, String>,
) -> LedgerViewVm {
    let chip_lookup: std::collections::HashMap<&str, &CampaignChipVm> =
        chips.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut campaigns = Vec::new();
    for campaign_id in ledger_campaign_ids(ledger) {
        let safe_id = scrub(&campaign_id);
        let chip = chip_lookup
            .get(safe_id.as_str())
            .copied()
            .cloned()
            .unwrap_or_else(|| CampaignChipVm {
                id: safe_id.clone(),
                label: safe_id.clone(),
                color_class: String::new(),
                display_label: safe_id.clone(),
            });
        let rows: Vec<LedgerRowVm> = ledger
            .by_campaign(&campaign_id)
            .map(|e| ledger_row_from_entry(log, e, intent_spend_map))
            .collect();
        campaigns.push(LedgerCampaignVm {
            campaign: chip,
            asked: ledger.asked(&campaign_id),
            done: ledger.done(&campaign_id),
            proven: ledger.proven(&campaign_id),
            in_flight: 0,
            rows,
            owner: String::new(),
            session: String::new(),
            window: String::new(),
            bundle_note: String::new(),
            why: String::new(),
            acceptance_note: String::new(),
            envelope_refs: None,
            cost_rollup: String::new(),
            cache_savings: String::new(),
            seal_note: None,
        });
    }
    LedgerViewVm { campaigns }
}

fn build_landed_by_day(ledger: &Ledger) -> Vec<(String, u32)> {
    let mut day_counts: std::collections::BTreeMap<(i32, u32, u32), u32> =
        std::collections::BTreeMap::new();
    for entry in ledger.entries() {
        *day_counts
            .entry(day_key_from_ms(entry.recorded_at))
            .or_default() += 1;
    }
    day_counts
        .into_iter()
        .map(|((y, m, d), c)| (format_short_day_label(y, m, d), c))
        .collect()
}

fn kpi(label: &str, value: String, sub_text: &str) -> KpiVm {
    KpiVm {
        label: label.to_string(),
        value,
        delta: None,
        unit: String::new(),
        sub_kind: KpiSubKind::Plain,
        sub_text: sub_text.to_string(),
        label_has_period: false,
    }
}

fn build_kpis(ledger: &Ledger) -> Vec<KpiVm> {
    let total = ledger.entries().len();
    let proven = ledger.entries().iter().filter(|e| e.proven).count();
    let rejected = ledger.entries().iter().filter(|e| e.rejected).count();
    let campaigns = ledger_campaign_ids(ledger).len();
    let proven_pct = if total > 0 {
        format!("{:.0}%", 100.0 * proven as f64 / total as f64)
    } else {
        "—".to_string()
    };
    vec![
        kpi("Intents pousados", total.to_string(), "total"),
        kpi("Provados (APPROVE)", proven.to_string(), &proven_pct),
        kpi("Rejeitados", rejected.to_string(), "REJECT / FIX-FIRST"),
        kpi("Campanhas", campaigns.to_string(), ""),
    ]
}

/// Build the insights view-model from a verified event log.
pub fn build_insights(log: &EventLog, repo: &str) -> InsightsVm {
    let ledger = Ledger::from_records(log.records());
    let chips = campaign_chips(log);
    let xray = build_cost_xray(log, &chips);

    // Build intent_id → cas: ref map from the log's INTENT_ENVELOPE records
    // (altitude Intent) using the RAW stored payload bytes — avoids serde
    // field-order drift that would break the proof if the envelope were re-serialized.
    let intent_spend_map: std::collections::HashMap<String, String> = log
        .records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| {
            let env = serde_json::from_str::<ContextEnvelope>(&r.payload).ok()?;
            if env.altitude == Altitude::Intent {
                Some((env.intent_id.clone(), cold_ref_for(r.payload.as_bytes())))
            } else {
                None
            }
        })
        .collect();

    InsightsVm {
        repo: repo.to_string(),
        kpis: build_kpis(&ledger),
        landed_by_day: build_landed_by_day(&ledger),
        tokens_by_campaign: xray.tokens_by_campaign, // REAL — per-campaign raw tokens
        cost_xray: xray.rows,
        cost_xray_totals: xray.totals, // REAL — grand totals (sound sums)
        tokens_by_model: vec![],       // HONEST-DEFAULT — no per-model seam
        tokens_by_model_legend: String::new(), // HONEST-DEFAULT
        global_decomp: None,           // HONEST-DEFAULT — no decomp seam
        contrib: vec![],               // HONEST-DEFAULT — no contributor seam
        landing_times: None,           // HONEST-DEFAULT — no timing seam
        ci_checks: None,               // HONEST-DEFAULT — no CI-card seam
        ledger: build_ledger_view(log, &ledger, &chips, &intent_spend_map),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_cli::ctx::CTX_USAGE_KIND;

    fn push(log: &mut EventLog, kind: &str, payload: serde_json::Value, seq: u64) {
        log.append_for_test(kind, vec!["t".to_string()], payload.to_string(), seq);
    }

    fn land(log: &mut EventLog, id: &str, seq: u64) {
        push(
            log,
            "intent.landed",
            serde_json::json!({"intent_id": id, "campaign": "c3", "charter": "do it", "ref": "refs/heads/x", "target": "0".repeat(40)}),
            seq,
        );
    }

    fn ledger_row<'a>(vm: &'a InsightsVm, id: &str) -> &'a LedgerRowVm {
        vm.ledger
            .campaigns
            .iter()
            .flat_map(|c| c.rows.iter())
            .find(|r| r.intent_id == id)
            .expect("row present")
    }

    #[test]
    fn ctx_usage_lights_the_ledger_row_cost_and_tokens() {
        // WP-COST read-time fold (site B): a hand-appended `ctx.usage` record on a
        // landed intent lights the ledger row's cost_micros + tokens_count — no re-land.
        let mut log = EventLog::new();
        land(&mut log, "a31", 1);
        push(
            &mut log,
            CTX_USAGE_KIND,
            serde_json::json!({
                "target_id": "a31",
                "target_kind": "intent",
                "source": "provider_usage",
                "model": "claude-opus-4-8",
                "recorded_at": 0,
                "tokens": {"input": 1000, "output": 200, "cache_read": 0, "cache_write": 0, "total": 1200},
            }),
            2,
        );
        let vm = build_insights(&log, "hugit");
        let row = ledger_row(&vm, "a31");
        // opus-4-8: $5/MTok input + $25/MTok output = 5000 + 5000 = 10_000 micro-USD.
        assert_eq!(row.cost_micros, 10_000);
        assert_eq!(row.tokens_count, 1200);
    }

    #[test]
    fn no_ctx_usage_keeps_ledger_row_honest_zero() {
        let mut log = EventLog::new();
        land(&mut log, "a31", 1);
        let vm = build_insights(&log, "hugit");
        let row = ledger_row(&vm, "a31");
        assert_eq!(row.cost_micros, 0);
        assert_eq!(row.tokens_count, 0);
    }

    #[test]
    fn foreign_target_ctx_usage_does_not_leak_into_row() {
        // A ctx.usage for a DIFFERENT intent must not attribute cost to this row.
        let mut log = EventLog::new();
        land(&mut log, "a31", 1);
        push(
            &mut log,
            CTX_USAGE_KIND,
            serde_json::json!({
                "target_id": "OTHER",
                "target_kind": "intent",
                "source": "provider_usage",
                "model": "claude-opus-4-8",
                "recorded_at": 0,
                "tokens": {"input": 9999, "output": 9999, "cache_read": 0, "cache_write": 0, "total": 19998},
            }),
            2,
        );
        let vm = build_insights(&log, "hugit");
        let row = ledger_row(&vm, "a31");
        assert_eq!(row.cost_micros, 0);
        assert_eq!(row.tokens_count, 0);
    }
}
