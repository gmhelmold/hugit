//! `GET /v1/repos/{repo}/insights` → `InsightsVm` and all nested types for
//! the Insights + Ledger screen. Transcribed BYTE-FOR-FIELD from the canonical
//! companion web frontend's view-model definitions; derives copied verbatim.

use serde::{Deserialize, Serialize};

use crate::common::{CampaignChipVm, KpiVm};

/// One per-PR drill-down row inside a Cost X-ray campaign row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XrayDrillRowVm {
    pub pr_ref: String,
    pub pr_url: String,
    pub intent_label: String,
    pub cost: String,
    pub saving: String,
    pub first_pass: String,
    pub in_flight: bool,
}

/// One campaign row of the Cost X-ray table (7-column schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostXrayRowVm {
    pub campaign: CampaignChipVm,
    pub tokens: String,
    pub prs_int: String,
    /// decomp-bar segment widths (work, orch, verif, ci) as percentages.
    pub decomp_pcts: (u32, u32, u32, u32),
    pub cost_total: String,
    pub waste: String,
    pub cache_saved: String,
    pub first_pass: String,
    pub drill_rows: Vec<XrayDrillRowVm>,
}

/// The Cost X-ray `<tfoot>` totals row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XrayTotalsVm {
    pub tokens: String,
    pub prs_int: String,
    pub cost: String,
    pub waste: String,
    pub cache_saved: String,
    pub first_pass: String,
}

/// The "Para onde foi o custo" global cost-decomposition card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalDecompVm {
    pub bar_pcts: (u32, u32, u32, u32),
    pub work_usd: String,
    pub orchestration_usd: String,
    pub verification_usd: String,
    pub ci_usd: String,
    pub waste_usd: String,
    pub overhead_pct: String,
}

/// One contributor row in the full-width Contribuição card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContribRowVm {
    pub name: String,
    pub kind: String,
    pub bar_pct: u32,
    pub bar_accent: bool,
    pub stat: String,
    pub pct: String,
    pub pct_color: String,
    pub drill: Vec<(CampaignChipVm, String)>,
}

/// One k/v secondary-metric row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricRowVm {
    pub label: String,
    pub value: String,
    pub value_suffix: String,
    pub value_color: String,
}

/// A secondary-metrics card (Tempo até landing · Checks CI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricCardVm {
    pub title: String,
    pub sub: String,
    pub rows: Vec<MetricRowVm>,
}

/// One ledger row (a single intent/PR entry in a campaign's ledger).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRowVm {
    pub intent_id: String,
    pub asked: String,
    pub done_status: String,
    pub proven_status: String,
    pub verdict: Option<String>,
    pub when: String,
    pub model: String,
    pub done_body: String,
    pub pr_number: Option<u32>,
    pub intent_chips: Vec<String>,
    pub verdict_chips: Vec<String>,
    pub proof_note: String,
    pub cost: String,
    pub savings: String,
}

/// One campaign block in the Ledger view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCampaignVm {
    pub campaign: CampaignChipVm,
    pub asked: usize,
    pub done: usize,
    pub proven: usize,
    #[serde(default)]
    pub in_flight: usize,
    pub rows: Vec<LedgerRowVm>,
    pub owner: String,
    pub session: String,
    pub window: String,
    pub bundle_note: String,
    pub why: String,
    pub acceptance_note: String,
    pub envelope_refs: Option<LedgerEnvelopeRefVm>,
    pub cost_rollup: String,
    pub cache_savings: String,
    pub seal_note: Option<String>,
}

/// The campaign envelope refs surfaced in the Ledger (ADR-0001 §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LedgerEnvelopeRefVm {
    pub header: String,
    pub context_cas: String,
    pub pr_envelopes: Vec<(String, String)>,
    pub compact_context_ref: String,
    pub bundle_ref: String,
    #[serde(default)]
    pub trajectory_summary: String,
    #[serde(default)]
    pub compact_transcript_ref: String,
    #[serde(default)]
    pub raw_transcript_ref: String,
    #[serde(default)]
    pub snapshot_files: Vec<String>,
    #[serde(default)]
    pub context_json: String,
    #[serde(default)]
    pub context_json_fn: String,
    #[serde(default)]
    pub compact_json_note: String,
    #[serde(default)]
    pub bundle_note: String,
}

/// The flat ledger view (all campaigns, in order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerViewVm {
    pub campaigns: Vec<LedgerCampaignVm>,
}

/// Top-level insights view-model. `PartialEq` only — matches the canonical
/// source derive (no `Eq` even though all fields are scalar — copy verbatim).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightsVm {
    pub repo: String,
    pub kpis: Vec<KpiVm>,
    pub landed_by_day: Vec<(String, u32)>,
    pub tokens_by_campaign: Vec<(CampaignChipVm, u64, String)>,
    pub cost_xray: Vec<CostXrayRowVm>,
    #[serde(default)]
    pub cost_xray_totals: Option<XrayTotalsVm>,
    pub tokens_by_model: Vec<(String, u64)>,
    #[serde(default)]
    pub tokens_by_model_legend: String,
    #[serde(default)]
    pub global_decomp: Option<GlobalDecompVm>,
    #[serde(default)]
    pub contrib: Vec<ContribRowVm>,
    #[serde(default)]
    pub landing_times: Option<MetricCardVm>,
    #[serde(default)]
    pub ci_checks: Option<MetricCardVm>,
    pub ledger: LedgerViewVm,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::KpiSubKind;

    /// `InsightsVm` round-trips losslessly.
    #[test]
    fn insights_vm_round_trips() {
        let kpi = KpiVm {
            label: "Tokens (30d)".to_string(),
            value: "14.2".to_string(),
            delta: Some("↑ 18%".to_string()),
            unit: "M".to_string(),
            sub_kind: KpiSubKind::DeltaUp,
            sub_text: "vs. período ant.".to_string(),
            label_has_period: true,
        };
        let drill_row = XrayDrillRowVm {
            pr_ref: "#129".to_string(),
            pr_url: "/r/corelink-server/pr/129".to_string(),
            intent_label: "3 intents".to_string(),
            cost: "$0.02".to_string(),
            saving: "poupou $2.05".to_string(),
            first_pass: "✓ first-pass".to_string(),
            in_flight: false,
        };
        let chip = CampaignChipVm {
            id: "auth-hardening".to_string(),
            label: "auth-hardening".to_string(),
            color_class: "c-auth".to_string(),
            display_label: "auth".to_string(),
        };
        let xray_row = CostXrayRowVm {
            campaign: chip.clone(),
            tokens: "5.1M".to_string(),
            prs_int: "2·2".to_string(),
            decomp_pcts: (63, 17, 12, 8),
            cost_total: "$24".to_string(),
            waste: "$2".to_string(),
            cache_saved: "$161".to_string(),
            first_pass: "90%".to_string(),
            drill_rows: vec![drill_row],
        };
        let ledger_row = LedgerRowVm {
            intent_id: "a31".to_string(),
            asked: "rate-limit por tenant no edge".to_string(),
            done_status: "mergeado".to_string(),
            proven_status: "verde".to_string(),
            verdict: Some("APPROVE".to_string()),
            when: "09 jun · 14:22".to_string(),
            model: "opus-4.8".to_string(),
            done_body: "PR #128 mergeado em main · 3 intents".to_string(),
            pr_number: Some(128),
            intent_chips: vec!["a31".to_string(), "a2f".to_string()],
            verdict_chips: vec!["correctness APPROVE".to_string()],
            proof_note: "checks 11 verdes · byte-idêntico ×3".to_string(),
            cost: "$0.04".to_string(),
            savings: "cache poupou $3.10".to_string(),
        };
        let envelope_ref = LedgerEnvelopeRefVm {
            header: "sessão da campanha orq-014 · opus-4.8 · 7–9 jun".to_string(),
            context_cas: "cas:a7e0…".to_string(),
            pr_envelopes: vec![("cas:c2e7… (#128)".to_string(), "/r/cs/pr/128".to_string())],
            compact_context_ref: "cas:b3a4… · 6 KB".to_string(),
            bundle_ref: "cas:77e1… · 9.8 MB".to_string(),
            trajectory_summary: "planejou a wave auth em 3 PRs".to_string(),
            compact_transcript_ref: "cas:e22a… · 87 KB".to_string(),
            raw_transcript_ref: "cas:f31d… · 5.4 MB".to_string(),
            snapshot_files: vec!["notes/wave-auth.md".to_string()],
            context_json: "{}".to_string(),
            context_json_fn: "auth-hardening.context.json".to_string(),
            compact_json_note: "6 KB".to_string(),
            bundle_note: "9.8 MB".to_string(),
        };
        let ledger_campaign = LedgerCampaignVm {
            campaign: chip.clone(),
            asked: 3,
            done: 3,
            proven: 3,
            in_flight: 0,
            rows: vec![ledger_row],
            owner: "gustavo".to_string(),
            session: "orq-014".to_string(),
            window: "7–9 jun".to_string(),
            bundle_note: "bundle de 3 PRs".to_string(),
            why: "hardening do auth layer".to_string(),
            acceptance_note: "aceite: zero findings repetidos no re-teste".to_string(),
            envelope_refs: Some(envelope_ref),
            cost_rollup: "$0.34".to_string(),
            cache_savings: "cache poupou $8.40".to_string(),
            seal_note: Some("selada 09 jun 16:40".to_string()),
        };
        let vm = InsightsVm {
            repo: "corelink-server".to_string(),
            kpis: vec![kpi],
            landed_by_day: vec![("09 jun".to_string(), 4u32), ("10 jun".to_string(), 7u32)],
            tokens_by_campaign: vec![(chip.clone(), 5_100_000u64, "$24".to_string())],
            cost_xray: vec![xray_row],
            cost_xray_totals: Some(XrayTotalsVm {
                tokens: "14.2M".to_string(),
                prs_int: "9·18".to_string(),
                cost: "$94".to_string(),
                waste: "$11".to_string(),
                cache_saved: "$312".to_string(),
                first_pass: "88%".to_string(),
            }),
            tokens_by_model: vec![("opus-4.8".to_string(), 9_100_000u64)],
            tokens_by_model_legend: "14.2M total · cache hit-rate 88%".to_string(),
            global_decomp: Some(GlobalDecompVm {
                bar_pcts: (55, 15, 10, 8),
                work_usd: "$52".to_string(),
                orchestration_usd: "$14".to_string(),
                verification_usd: "$9".to_string(),
                ci_usd: "$8".to_string(),
                waste_usd: "$11".to_string(),
                overhead_pct: "17%".to_string(),
            }),
            contrib: vec![ContribRowVm {
                name: "opus-4.8".to_string(),
                kind: "agente".to_string(),
                bar_pct: 65,
                bar_accent: false,
                stat: "834 intents · 71 PRs".to_string(),
                pct: "65%".to_string(),
                pct_color: "".to_string(),
                drill: vec![(chip.clone(), "71 PRs".to_string())],
            }],
            landing_times: Some(MetricCardVm {
                title: "Tempo até landing".to_string(),
                sub: "· mediana + P95".to_string(),
                rows: vec![MetricRowVm {
                    label: "PR simples (1 intent)".to_string(),
                    value: "mediana 11min".to_string(),
                    value_suffix: "91%".to_string(),
                    value_color: "g".to_string(),
                }],
            }),
            ci_checks: Some(MetricCardVm {
                title: "Checks CI".to_string(),
                sub: "· 30 dias".to_string(),
                rows: vec![MetricRowVm {
                    label: "cache hit-rate".to_string(),
                    value: "88%".to_string(),
                    value_suffix: "".to_string(),
                    value_color: "g".to_string(),
                }],
            }),
            ledger: LedgerViewVm {
                campaigns: vec![ledger_campaign],
            },
        };

        let json = serde_json::to_string(&vm).expect("InsightsVm serializes");
        let reparsed: InsightsVm = serde_json::from_str(&json).expect("InsightsVm deserializes");
        assert_eq!(vm, reparsed, "InsightsVm round-trip is lossless");
    }
}
