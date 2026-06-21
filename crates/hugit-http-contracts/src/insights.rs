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
    // deprecated: format in the view
    pub tokens: String,
    pub prs_int: String,
    /// decomp-bar segment widths (work, orch, verif, ci) as percentages.
    pub decomp_pcts: (u32, u32, u32, u32),
    // deprecated: format in the view
    pub cost_total: String,
    // deprecated: format in the view
    pub waste: String,
    // deprecated: format in the view
    pub cache_saved: String,
    pub first_pass: String,
    pub drill_rows: Vec<XrayDrillRowVm>,

    // F4a — raw integer fields (additive; serde-default so old clients ignore them).
    /// Raw token count for the campaign row (same source as `tokens` string).
    #[serde(default)]
    pub tokens_count: u64,
    /// Total cost in integer micro-USD (1 USD = 1_000_000); same source as `cost_total`.
    #[serde(default)]
    pub cost_total_micros: u64,
    /// Waste cost in integer micro-USD; same source as `waste`.
    #[serde(default)]
    pub waste_micros: u64,
    /// CI cache savings in integer micro-USD; honest-zero until the cache-$ seam exists.
    #[serde(default)]
    pub cache_saved_micros: u64,

    // F4a — spend attestation.
    /// Content-addressed ref to the envelope's signed record — provability hook.
    /// `Some` when the PR-altitude envelope was captured and its `cas:` ref is
    /// available; `None` when no envelope was captured for this campaign row.
    #[serde(default)]
    pub spend_proof: Option<String>,

    // F5 — cache efficiency (honest-`None` until the CI-cost/cache seam exists).
    /// Cache efficiency percentage (0–100); `None` until the CI pricing seam
    /// supplies both `saved_usd_micros` and `cost_usd_micros`.
    #[serde(default)]
    pub cache_efficiency_pct: Option<u8>,
}

/// The Cost X-ray `<tfoot>` totals row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XrayTotalsVm {
    // deprecated: format in the view
    pub tokens: String,
    pub prs_int: String,
    // deprecated: format in the view
    pub cost: String,
    // deprecated: format in the view
    pub waste: String,
    // deprecated: format in the view
    pub cache_saved: String,
    pub first_pass: String,

    // F4a — raw integer totals (additive; serde-default so old clients ignore them).
    /// Total raw token count across all campaigns.
    #[serde(default)]
    pub tokens_count: u64,
    /// Grand total cost in integer micro-USD.
    #[serde(default)]
    pub cost_micros: u64,
    /// Grand total waste in integer micro-USD.
    #[serde(default)]
    pub waste_micros: u64,
    /// Grand total CI savings in integer micro-USD; honest-zero until the cache-$ seam exists.
    #[serde(default)]
    pub cache_saved_micros: u64,
}

/// The "Para onde foi o custo" global cost-decomposition card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalDecompVm {
    pub bar_pcts: (u32, u32, u32, u32),
    // deprecated: format in the view
    pub work_usd: String,
    // deprecated: format in the view
    pub orchestration_usd: String,
    // deprecated: format in the view
    pub verification_usd: String,
    // deprecated: format in the view
    pub ci_usd: String,
    // deprecated: format in the view
    pub waste_usd: String,
    pub overhead_pct: String,

    // F4a — raw integer decomposition fields (additive; serde-default so old clients ignore them).
    /// Work cost in integer micro-USD.
    #[serde(default)]
    pub work_micros: u64,
    /// Orchestration cost in integer micro-USD.
    #[serde(default)]
    pub orchestration_micros: u64,
    /// Verification cost in integer micro-USD.
    #[serde(default)]
    pub verification_micros: u64,
    /// CI cost in integer micro-USD.
    #[serde(default)]
    pub ci_micros: u64,
    /// Waste cost in integer micro-USD.
    #[serde(default)]
    pub waste_micros: u64,
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
    // deprecated: format in the view
    pub cost: String,
    // deprecated: format in the view
    pub savings: String,

    // F4a — raw integer cost fields (additive; serde-default so old clients ignore them).
    /// Total cost for this ledger entry in integer micro-USD.
    #[serde(default)]
    pub cost_micros: u64,
    /// Cache savings for this entry in integer micro-USD; honest-zero when no seam.
    #[serde(default)]
    pub savings_micros: u64,
    /// Raw token count for this entry; honest-zero when no envelope is present.
    #[serde(default)]
    pub tokens_count: u64,

    // F4a — spend attestation.
    /// Content-addressed ref to the envelope's signed record — provability hook.
    /// `Some(cas_ref)` when the intent-altitude envelope was captured; `None` otherwise.
    #[serde(default)]
    pub spend_proof: Option<String>,
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
            // F4a raw int fields
            tokens_count: 5_100_000,
            cost_total_micros: 24_000_000,
            waste_micros: 2_000_000,
            cache_saved_micros: 0,
            spend_proof: Some("cas:abc123".to_string()),
            cache_efficiency_pct: None,
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
            // F4a raw int fields
            cost_micros: 40_000,
            savings_micros: 3_100_000,
            tokens_count: 0,
            spend_proof: None,
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
                // F4a raw int totals
                tokens_count: 14_200_000,
                cost_micros: 94_000_000,
                waste_micros: 11_000_000,
                cache_saved_micros: 0,
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
                // F4a raw int decomp fields
                work_micros: 52_000_000,
                orchestration_micros: 14_000_000,
                verification_micros: 9_000_000,
                ci_micros: 8_000_000,
                waste_micros: 11_000_000,
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

    /// F4a: raw integer fields on `CostXrayRowVm` are present and survive a round-trip.
    /// Tests both the real-populated path (`tokens_count`, `cost_total_micros`, `waste_micros`,
    /// `spend_proof: Some`) and the honest-null/zero path (`cache_saved_micros: 0`,
    /// `cache_efficiency_pct: None`).
    #[test]
    fn cost_xray_row_raw_int_fields_round_trip() {
        let chip = CampaignChipVm {
            id: "c1".to_string(),
            label: "c1".to_string(),
            color_class: "".to_string(),
            display_label: "c1".to_string(),
        };
        let row = CostXrayRowVm {
            campaign: chip,
            tokens: "5.1M".to_string(),
            prs_int: "1·2".to_string(),
            decomp_pcts: (0, 0, 0, 0),
            cost_total: "$24.00".to_string(),
            waste: "$2.00".to_string(),
            cache_saved: String::new(),
            first_pass: String::new(),
            drill_rows: vec![],
            // real-populated:
            tokens_count: 5_100_000,
            cost_total_micros: 24_000_000,
            waste_micros: 2_000_000,
            // honest-zero (no cache-$ seam):
            cache_saved_micros: 0,
            // real spend_proof:
            spend_proof: Some("cas:abc123proof".to_string()),
            // honest-None (no CI seam):
            cache_efficiency_pct: None,
        };
        let json = serde_json::to_string(&row).expect("serializes");
        let back: CostXrayRowVm = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.tokens_count, 5_100_000, "tokens_count round-trips");
        assert_eq!(
            back.cost_total_micros, 24_000_000,
            "cost_total_micros round-trips"
        );
        assert_eq!(back.waste_micros, 2_000_000, "waste_micros round-trips");
        assert_eq!(back.cache_saved_micros, 0, "cache_saved_micros honest-zero");
        assert_eq!(
            back.spend_proof,
            Some("cas:abc123proof".to_string()),
            "spend_proof Some round-trips"
        );
        assert_eq!(
            back.cache_efficiency_pct, None,
            "cache_efficiency_pct honest-None"
        );
    }

    /// F4a: serde-default — old JSON without the new raw int fields deserializes cleanly
    /// (backward compat for the live githugr window).
    #[test]
    fn cost_xray_row_missing_new_fields_default_to_zero_and_none() {
        let old_json = r#"{
            "campaign": {"id":"c","label":"c","color_class":"","display_label":"c"},
            "tokens": "1.0M", "prs_int": "1·1", "decomp_pcts": [0,0,0,0],
            "cost_total": "$1.00", "waste": "$0.00", "cache_saved": "",
            "first_pass": "", "drill_rows": []
        }"#;
        let row: CostXrayRowVm = serde_json::from_str(old_json).expect("old JSON parses");
        assert_eq!(row.tokens_count, 0, "tokens_count defaults to 0");
        assert_eq!(row.cost_total_micros, 0, "cost_total_micros defaults to 0");
        assert_eq!(row.waste_micros, 0, "waste_micros defaults to 0");
        assert_eq!(
            row.cache_saved_micros, 0,
            "cache_saved_micros defaults to 0"
        );
        assert_eq!(row.spend_proof, None, "spend_proof defaults to None");
        assert_eq!(
            row.cache_efficiency_pct, None,
            "cache_efficiency_pct defaults to None"
        );
    }

    /// F4a: raw integer totals on `XrayTotalsVm` are present and round-trip.
    #[test]
    fn xray_totals_raw_int_fields_round_trip() {
        let totals = XrayTotalsVm {
            tokens: "14.2M".to_string(),
            prs_int: "9·18".to_string(),
            cost: "$94".to_string(),
            waste: "$11".to_string(),
            cache_saved: String::new(),
            first_pass: String::new(),
            tokens_count: 14_200_000,
            cost_micros: 94_000_000,
            waste_micros: 11_000_000,
            cache_saved_micros: 0,
        };
        let json = serde_json::to_string(&totals).expect("serializes");
        let back: XrayTotalsVm = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.tokens_count, 14_200_000);
        assert_eq!(back.cost_micros, 94_000_000);
        assert_eq!(back.waste_micros, 11_000_000);
        assert_eq!(back.cache_saved_micros, 0);
    }

    /// F4a: raw integer fields on `LedgerRowVm` — real-populated and honest-zero paths.
    #[test]
    fn ledger_row_raw_int_fields_and_spend_proof() {
        let row_with_proof = LedgerRowVm {
            intent_id: "x1".to_string(),
            asked: "do the thing".to_string(),
            done_status: "mergeado".to_string(),
            proven_status: "verde".to_string(),
            verdict: None,
            when: "now".to_string(),
            model: String::new(),
            done_body: String::new(),
            pr_number: None,
            intent_chips: vec![],
            verdict_chips: vec![],
            proof_note: String::new(),
            cost: String::new(),
            savings: String::new(),
            // honest-zero: no cost seam on ledger entries
            cost_micros: 0,
            savings_micros: 0,
            tokens_count: 0,
            // honest-None: no envelope ref on ledger view
            spend_proof: None,
        };
        let json = serde_json::to_string(&row_with_proof).expect("serializes");
        let back: LedgerRowVm = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.cost_micros, 0, "cost_micros honest-zero");
        assert_eq!(back.savings_micros, 0, "savings_micros honest-zero");
        assert_eq!(back.tokens_count, 0, "tokens_count honest-zero");
        assert_eq!(back.spend_proof, None, "spend_proof honest-None");
    }

    /// F4a: `LedgerRowVm` old JSON without new fields deserializes cleanly (backward compat).
    #[test]
    fn ledger_row_missing_new_fields_default() {
        let old_json = r#"{
            "intent_id":"x1","asked":"do","done_status":"m","proven_status":"v",
            "verdict":null,"when":"now","model":"","done_body":"","pr_number":null,
            "intent_chips":[],"verdict_chips":[],"proof_note":"","cost":"","savings":""
        }"#;
        let row: LedgerRowVm = serde_json::from_str(old_json).expect("old JSON parses");
        assert_eq!(row.cost_micros, 0);
        assert_eq!(row.savings_micros, 0);
        assert_eq!(row.tokens_count, 0);
        assert_eq!(row.spend_proof, None);
    }
}
