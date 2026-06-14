//! `GET /v1/repos/{repo}/campaigns/{name}` → [`CampaignVm`] (None = 404).
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: campaign.opened →
//! chip/operator/why; PRs in campaign → CampaignPrVm + cost; ledger → intents.
//! Honest defaults (no seam) for signing/session/model/window/born_from/docs/
//! envelope/seal/bundle_status/who/attestation/mirror. Every free-text field
//! passes `crate::fmt::scrub`. `None` → 404 (no existence leak).

use crate::fmt::{PR_CARDS_CAP, scrub};
use hugit_cli::pr::{
    CAMPAIGN_OPENED_KIND, INTENT_ENVELOPE_KIND, OpenedPr, PR_ABANDONED_KIND, PR_ENVELOPE_KIND,
    PR_LANDED_KIND, PR_OPENED_KIND, PR_QUEUED_KIND, find_pr_opened,
};
use hugit_contracts::context_envelope::{Altitude, CiCost, ContextEnvelope};
use hugit_http_contracts::CampaignVm;
use hugit_http_contracts::common::{CampaignChipVm, EnvelopeVm, MirrorVm};
use hugit_http_contracts::landing::{CampaignIntentChipVm, CampaignPrVm};
use hugit_http_contracts::pr_detail::{CostSplitVm, StatVm};
use hugit_ledger::Ledger;
use hugit_ledger::rollup::{PrQueueInput, pr_record};
use hugit_refstore::EventLog;
use serde_json::Value;

/// Build the campaign page view-model for `name`. `None` → 404 (no leak).
pub fn build_campaign(log: &EventLog, repo: &str, name: &str) -> Option<CampaignVm> {
    // REAL: must find a campaign.opened whose campaign==name (match raw key).
    let (owner, charter) = find_campaign_opened(log, name)?;

    let ledger = Ledger::from_records(log.records());
    let campaign_prs = prs_for_campaign(log, &ledger, name);
    let pr_count = campaign_prs.len();
    let intent_count: usize = campaign_prs.iter().map(|p| p.intent_count).sum();
    let landed_count = campaign_prs.iter().filter(|p| p.landed).count();

    let safe_name = scrub(name);
    let chip = CampaignChipVm {
        id: safe_name.clone(),
        label: safe_name.clone(),
        color_class: String::new(),
        display_label: safe_name.clone(),
    };

    let state_label = if pr_count == 0 {
        "sem PRs".to_string()
    } else if landed_count == pr_count {
        format!("bundle pousado · {landed_count} de {pr_count} pousaram")
    } else {
        format!("bundle em voo · {landed_count} de {pr_count} pousaram")
    };

    let cost = rollup_campaign_cost(log, name);
    let total_usd = cost.total_usd;
    let stats = vec![
        StatVm {
            value: format!("${total_usd:.2}"),
            label: "custo total".to_string(),
            sub: format!("{pr_count} PRs"),
        },
        StatVm {
            value: intent_count.to_string(),
            label: "intents".to_string(),
            sub: format!("{landed_count} pousaram"),
        },
    ];

    Some(CampaignVm {
        repo: repo.to_string(),
        name: safe_name.clone(),
        chip,
        state_label,
        operator: scrub(&owner),
        operator_signed: false,      // STUB — no signing seam off the log
        operator_sig: String::new(), // STUB
        session: String::new(),      // STUB
        model: String::new(),        // STUB
        window: String::new(),       // STUB
        pr_count,
        intent_count,
        why: scrub(&charter),
        acceptance_note: String::new(), // STUB
        born_from: vec![],              // STUB — no origin-ref seam
        docs: vec![],                   // STUB
        stats,
        prs: campaign_prs,
        envelope: EnvelopeVm::default(), // STUB — no campaign-level envelope seam
        seal_note: None,                 // STUB
        cost,
        bundle_status: vec![],            // STUB
        who: vec![],                      // STUB
        attested_label: String::new(),    // STUB
        envelope_cas: String::new(),      // STUB
        transcripts_label: String::new(), // STUB
        mirror: MirrorVm {
            synced: false,
            detail: String::new(),
        }, // STUB — P2 mirror
    })
}

fn find_campaign_opened(log: &EventLog, name: &str) -> Option<(String, String)> {
    log.records()
        .iter()
        .filter(|r| r.kind == CAMPAIGN_OPENED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .find(|v| v.get("campaign").and_then(Value::as_str) == Some(name))
        .map(|v| {
            (
                v.get("owner")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                v.get("charter")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
}

fn prs_for_campaign(log: &EventLog, ledger: &Ledger, name: &str) -> Vec<CampaignPrVm> {
    let mut seen = std::collections::BTreeSet::new();
    let mut pr_ids = Vec::new();
    for r in log.records().iter().filter(|r| r.kind == PR_OPENED_KIND) {
        if let Ok(v) = serde_json::from_str::<Value>(&r.payload)
            && v.get("campaign").and_then(Value::as_str) == Some(name)
            && let Some(id) = v.get("pr_id").and_then(Value::as_str)
            && seen.insert(id.to_string())
        {
            pr_ids.push(id.to_string());
        }
    }
    pr_ids.truncate(PR_CARDS_CAP); // cap per-request work (each PR → multiple log scans)
    pr_ids
        .into_iter()
        .filter_map(|pr_id| find_pr_opened(log, &pr_id).map(|o| build_campaign_pr(log, ledger, &o)))
        .collect()
}

fn build_campaign_pr(log: &EventLog, ledger: &Ledger, opened: &OpenedPr) -> CampaignPrVm {
    let number = opened.pr_id.parse::<u32>().unwrap_or(0);
    let n = opened.intent_ids.len();
    let title = if opened.campaign.is_empty() {
        format!("PR #{} — {} intents", opened.pr_id, n)
    } else {
        format!(
            "PR #{} ({}) — {} intents",
            opened.pr_id,
            scrub(&opened.campaign),
            n
        )
    };
    let landed = pr_has_event(log, PR_LANDED_KIND, &opened.pr_id);
    let state_label = if landed {
        "pousou ✓".to_string()
    } else if pr_has_event(log, PR_ABANDONED_KIND, &opened.pr_id) {
        "abandonado".to_string()
    } else if pr_has_event(log, PR_QUEUED_KIND, &opened.pr_id) {
        "na fila".to_string()
    } else {
        "proposto".to_string()
    };
    let cost_usd_micros = pr_cost_micros(log, opened);
    let why = pr_why(log, &opened.pr_id);
    let bundle: std::collections::BTreeSet<&str> =
        opened.intent_ids.iter().map(String::as_str).collect();
    let intents: Vec<CampaignIntentChipVm> = ledger
        .entries()
        .iter()
        .filter(|e| bundle.contains(e.intent_id.as_str()))
        .map(|e| CampaignIntentChipVm {
            id: e.intent_id.clone(),
            note: scrub(&e.charter),
        })
        .collect();
    CampaignPrVm {
        number,
        title,
        intent_count: n,
        cost_usd_micros,
        union_label: String::new(), // STUB — no live union oracle (P2)
        state_label,
        landed,
        why,
        intents,
    }
}

fn pr_has_event(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

fn pr_why(log: &EventLog, pr_id: &str) -> String {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == pr_id)
        .map(|e| scrub(&e.charter))
        .unwrap_or_default()
}

fn zero_ci() -> CiCost {
    CiCost {
        cache_hit: 0,
        exec: 0,
        cost_usd_micros: 0,
        saved_usd_micros: 0,
    }
}

fn intent_envs_for(log: &EventLog, opened: &OpenedPr) -> Vec<ContextEnvelope> {
    let bundle: std::collections::BTreeSet<&str> =
        opened.intent_ids.iter().map(String::as_str).collect();
    log.records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .filter(|e| e.altitude == Altitude::Intent && bundle.contains(e.intent_id.as_str()))
        .collect()
}

fn pr_env_for(log: &EventLog, pr_id: &str) -> Option<ContextEnvelope> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == pr_id)
}

fn pr_cost_micros(log: &EventLog, opened: &OpenedPr) -> u64 {
    let Some(env) = pr_env_for(log, &opened.pr_id) else {
        return 0;
    };
    let intent_envs = intent_envs_for(log, opened);
    pr_record(
        &env,
        "",
        &intent_envs,
        &opened.intent_ids,
        &[],
        zero_ci(),
        PrQueueInput::default(),
    )
    .map(|r| r.cost.total.cost_usd_micros)
    .unwrap_or(0)
}

fn rollup_campaign_cost(log: &EventLog, name: &str) -> CostSplitVm {
    let mut seen = std::collections::BTreeSet::new();
    let mut prs = Vec::new();
    for r in log.records().iter().filter(|r| r.kind == PR_OPENED_KIND) {
        if let Ok(v) = serde_json::from_str::<Value>(&r.payload)
            && v.get("campaign").and_then(Value::as_str) == Some(name)
            && let Some(id) = v.get("pr_id").and_then(Value::as_str)
            && seen.insert(id.to_string())
            && let Some(o) = find_pr_opened(log, id)
        {
            prs.push(o);
        }
    }
    prs.truncate(PR_CARDS_CAP); // cap per-request work (each PR → multiple log scans)

    let (mut work, mut orchestration, mut verification, mut ci, mut total, mut waste) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut intent_count_total = 0u64;
    for opened in &prs {
        let Some(env) = pr_env_for(log, &opened.pr_id) else {
            continue;
        };
        let intent_envs = intent_envs_for(log, opened);
        if let Ok(rec) = pr_record(
            &env,
            "",
            &intent_envs,
            &opened.intent_ids,
            &[],
            zero_ci(),
            PrQueueInput::default(),
        ) {
            work += rec.cost.work.cost_usd_micros;
            orchestration += rec.cost.orchestration.cost_usd_micros;
            verification += rec.cost.verification.cost_usd_micros;
            ci += rec.cost.ci.cost_usd_micros;
            total += rec.cost.total.cost_usd_micros;
            waste += rec.cost.waste.cost_usd_micros;
            intent_count_total += rec.intent_count;
        }
    }

    fn usd(micros: u64) -> f64 {
        micros as f64 / 1_000_000.0
    }
    CostSplitVm {
        author_line: String::new(),
        author_badge: None,
        author_suffix: None,
        work_usd: usd(work),
        work_note: format!("{intent_count_total} intents"),
        orchestration_usd: usd(orchestration),
        orchestration_note: String::new(),
        verification_usd: usd(verification),
        verification_note: String::new(),
        ci_usd: usd(ci),
        ci_note: String::new(),
        total_usd: usd(total),
        waste_usd: usd(waste),
        waste_note: String::new(),
        overhead_pct: 0,      // STUB — efficiency ratio not soundly aggregable
        cache_savings_pct: 0, // STUB
        time_note: String::new(),
    }
}
