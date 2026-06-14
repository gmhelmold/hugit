//! `GET /v1/repos/{repo}/prs/{n}` → `PrDetailVm` and its pr-specific nested
//! types. Shared atoms (`CampaignChipVm`, `CheckRowVm`, `DiffVm`, `EnvelopeVm`,
//! `IntentSummaryVm`, `MirrorVm`, `UnionVm`) come from [`crate::common`].
//! Transcribed BYTE-FOR-FIELD; derives copied verbatim (`CostSplitVm`/`PrDetailVm`
//! carry `f64` → `PartialEq`-only, never `Eq`).

use serde::{Deserialize, Serialize};

use crate::common::{
    CampaignChipVm, CheckRowVm, DiffVm, EnvelopeVm, IntentSummaryVm, MirrorVm, UnionVm,
};

/// One stat tile in a metrics strip; `value` is display-formatted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatVm {
    pub value: String,
    pub label: String,
    pub sub: String,
}

/// The Impacto card — build-graph blast radius (hugit impact).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactVm {
    pub crates_touched: Vec<String>,
    pub direct_dependents: Vec<String>,
    pub transitive_dependents: Vec<String>,
    pub critical_paths_note: String,
    #[serde(default)]
    pub critical_paths_target: String,
    #[serde(default)]
    pub critical_paths_anchor: String,
    pub critical_paths_safe: bool,
    pub checks_selected: u32,
    pub checks_total: u32,
    pub source_note: String,
}

/// Decomposed cost (work / orchestration / verification / ci + waste).
/// `PartialEq` only (contains `f64`) — matches the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostSplitVm {
    pub author_line: String,
    #[serde(default)]
    pub author_badge: Option<String>,
    #[serde(default)]
    pub author_suffix: Option<String>,
    pub work_usd: f64,
    pub work_note: String,
    pub orchestration_usd: f64,
    pub orchestration_note: String,
    pub verification_usd: f64,
    pub verification_note: String,
    pub ci_usd: f64,
    pub ci_note: String,
    pub total_usd: f64,
    pub waste_usd: f64,
    pub waste_note: String,
    pub overhead_pct: u8,
    pub cache_savings_pct: u8,
    pub time_note: String,
}

/// The Q&A card ("Conversa") — one anchored exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrConversationVm {
    pub asked_by: String,
    pub question: String,
    pub answer: String,
}

/// One reviewer row in the rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrReviewerVm {
    pub name: String,
    pub state: String,
    pub approved: bool,
}

/// One chip in the PR's "better than GitHub" strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BetterChipVm {
    pub prefix: String,
    pub bold: String,
    pub suffix: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrDetailVm {
    pub repo: String,
    pub number: u32,
    pub title: String,
    pub state_label: String,
    pub author: String,
    pub session: String,
    pub source_branch: String,
    pub target_branch: String,
    pub models_note: String,
    pub file_count: usize,
    pub added: u32,
    pub removed: u32,
    pub better_chips: Vec<BetterChipVm>,
    pub regen_note: Option<String>,
    pub why: String,
    pub acceptance_note: String,
    pub stats: Vec<StatVm>,
    pub envelope: EnvelopeVm,
    pub campaign: Option<CampaignChipVm>,
    pub impact: ImpactVm,
    pub cost: CostSplitVm,
    pub conversation: Vec<PrConversationVm>,
    pub intents: Vec<IntentSummaryVm>,
    pub union: UnionVm,
    pub check_rows: Vec<CheckRowVm>,
    pub checks_cost_note: String,
    #[serde(default)]
    pub checks_cache_note: String,
    pub diff: DiffVm,
    pub landing_status: Vec<(String, String)>,
    pub reviewers: Vec<PrReviewerVm>,
    pub panel_note: String,
    pub assignee: String,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub milestone_progress: Option<(u32, u32)>,
    pub provenance_campaign: String,
    pub stack: Option<String>,
    pub change_id: String,
    pub attested_label: String,
    pub envelope_cas: String,
    pub transcripts_label: String,
    pub mirror: MirrorVm,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CostSplitVm` (the f64-bearing, default-Option-bearing pr_detail atom)
    /// round-trips losslessly — the most transcription-error-prone local type.
    #[test]
    fn cost_split_vm_round_trips() {
        let canonical = r#"{
          "author_line": "aberto pelo orquestrador orq-014",
          "author_badge": "nunca subagent",
          "author_suffix": "— o trabalho é dos intents",
          "work_usd": 0.21, "work_note": "3 intents",
          "orchestration_usd": 0.03, "orchestration_note": "coordenação",
          "verification_usd": 0.06, "verification_note": "painel adversarial",
          "ci_usd": 0.04, "ci_note": "checks executados",
          "total_usd": 0.34, "waste_usd": 0.0, "waste_note": "zero re-trabalho",
          "overhead_pct": 15, "cache_savings_pct": 98,
          "time_note": "22m relógio · 38m agente"
        }"#;
        let vm: CostSplitVm = serde_json::from_str(canonical).expect("CostSplitVm parses");
        assert_eq!(vm.total_usd, 0.34);
        assert_eq!(vm.overhead_pct, 15);
        assert_eq!(vm.author_badge.as_deref(), Some("nunca subagent"));
        let reparsed: CostSplitVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "CostSplitVm round-trip is lossless");
    }

    /// `ImpactVm` round-trips (the build-graph blast-radius card).
    #[test]
    fn impact_vm_round_trips() {
        let canonical = r#"{
          "crates_touched": ["crates/auth"],
          "direct_dependents": ["crates/edge"],
          "transitive_dependents": [],
          "critical_paths_note": "✓ 0 caminhos pra billing/",
          "critical_paths_safe": true,
          "checks_selected": 5, "checks_total": 14,
          "source_note": "fonte: build-graph do hugit"
        }"#;
        let vm: ImpactVm = serde_json::from_str(canonical).expect("ImpactVm parses");
        assert_eq!(vm.checks_selected, 5);
        assert!(vm.critical_paths_safe);
        assert_eq!(vm.critical_paths_target, ""); // serde(default) absent → ""
        let reparsed: ImpactVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "ImpactVm round-trip is lossless");
    }
}
