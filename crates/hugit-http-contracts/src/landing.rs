//! `GET /v1/repos/{repo}/landing` → `LandingVm` and its landing-specific nested
//! types. Shared atoms (`CampaignChipVm`, `UnionVm`, `CostVm`, `MirrorVm`,
//! `IntentSummaryVm`, `FileRowVm`, `DiffVm`) come from [`crate::common`].
//! Transcribed BYTE-FOR-FIELD from the canonical source; derives copied verbatim
//! (composites embedding `CostVm`'s `f64` are `PartialEq`-only, never `Eq`).

use serde::{Deserialize, Serialize};

use crate::common::{
    CampaignChipVm, CostVm, DiffVm, EnvelopeVm, FileRowVm, IntentSummaryVm, MirrorVm, UnionVm,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    Open,
    Queued,
    Testing,
    Blocked,
    Landed,
    Draft,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecksBadgeVm {
    pub passed: usize,
    pub total: usize,
    pub cache_hits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackVm {
    /// Position label, e.g. "1/2".
    pub position: String,
    pub base: String,
    pub top: String,
    pub is_top: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginRefVm {
    pub label: String,
    pub detail: String,
    #[serde(default)]
    pub scanned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignIntentChipVm {
    pub id: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignPrVm {
    pub number: u32,
    pub title: String,
    pub intent_count: usize,
    #[serde(rename = "cost_usd_micros")]
    pub cost_usd_micros: u64,
    pub union_label: String,
    pub state_label: String,
    pub landed: bool,
    pub why: String,
    pub intents: Vec<CampaignIntentChipVm>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignDrawerVm {
    pub name: String,
    pub pill: String,
    pub why: String,
    pub acceptance_note: String,
    pub progress_note: String,
    pub window: String,
    pub owner: String,
    pub session: String,
    pub model: String,
    pub volume: String,
    pub first_pass_pct: String,
    pub union_note: String,
    pub signature: String,
    pub issues_linked: Vec<OriginRefVm>,
    pub cost_rollup: String,
    pub cost_rows: Vec<(String, String, String)>,
    pub cache_savings: String,
    pub seal_note: Option<String>,
    pub envelope: EnvelopeVm,
    pub prs_section: String,
    pub prs: Vec<CampaignPrVm>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrDrawerVm {
    pub union: UnionVm,
    pub cost: CostVm,
    pub mirror: MirrorVm,
    pub intents: Vec<IntentSummaryVm>,
    pub files: Vec<FileRowVm>,
    #[serde(default)]
    pub conflict_note: Option<String>,
    #[serde(default)]
    pub summary_diff: Option<DiffVm>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrCardVm {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub model: String,
    pub campaign: Option<CampaignChipVm>,
    pub intent_count: usize,
    pub file_count: usize,
    pub checks: ChecksBadgeVm,
    pub state: PrState,
    pub stack: Option<StackVm>,
    pub date: String,
    pub list_badge: String,
    #[serde(default)]
    pub change_id: String,
    #[serde(default)]
    pub landed_ago: String,
    pub drawer: PrDrawerVm,
}

/// A column item: a lone PR card or a campaign bundle of cards. Externally
/// tagged (`{"Card": {…}}` / `{"Bundle": {…}}`) — matches the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LandingItemVm {
    Card(Box<PrCardVm>),
    Bundle {
        campaign: CampaignChipVm,
        cards: Vec<PrCardVm>,
        drawer: Box<CampaignDrawerVm>,
        #[serde(default)]
        bsub: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandingColumnVm {
    pub title: String,
    pub items: Vec<LandingItemVm>,
    #[serde(default)]
    pub orq_model: String,
    #[serde(default)]
    pub window: String,
    #[serde(default)]
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandingListGroupVm {
    pub name: String,
    pub color_class: String,
    pub owner: String,
    pub orq_model: String,
    pub subtitle: String,
    pub window: String,
    pub landed: u32,
    pub total: u32,
    pub cost: String,
    pub collapsed: bool,
    pub items: Vec<LandingItemVm>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandingVm {
    pub repo: String,
    pub main_green: bool,
    pub main_status: String,
    pub open_count: usize,
    #[serde(default)]
    pub merged_count: usize,
    #[serde(default)]
    pub draft_count: usize,
    pub columns: Vec<LandingColumnVm>,
    pub campaigns: Vec<CampaignChipVm>,
    #[serde(default)]
    pub list_groups: Vec<LandingListGroupVm>,
    #[serde(default)]
    pub filter_pills: Vec<String>,

    // F5 — queue wedge fields (additive; serde-default so old clients ignore them).
    /// The 1-based queue position of the first actively-queued PR on this log.
    /// REAL: populated from the landing/queue projection (`pr.queued` order_index)
    /// when at least one PR is in the active queue. `None` when the queue is empty.
    #[serde(default)]
    pub queue_position: Option<u32>,
    /// Estimated seconds until the first queued PR lands.
    /// Honest-`None` until a real estimator exists (no timing seam yet).
    #[serde(default)]
    pub eta_seconds: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical Appendix-A `landing` JSON round-trips losslessly through
    /// `LandingVm` (every nested atom resolves; externally-tagged `Card` parses).
    #[test]
    fn landing_vm_round_trips_canonical_json() {
        let canonical = r##"{
          "repo": "hugit",
          "main_green": true,
          "main_status": "main verde · 12 pousos hoje",
          "open_count": 24, "merged_count": 0, "draft_count": 3,
          "columns": [{
            "title": "Na fila",
            "items": [{ "Card": {
              "number": 128, "title": "fix: refresh expira cedo",
              "author": "gustavo", "model": "opus-4.8",
              "campaign": { "id": "auth-hardening", "label": "Auth Hardening", "color_class": "c-auth", "display_label": "auth" },
              "intent_count": 3, "file_count": 7,
              "checks": { "passed": 11, "total": 14, "cache_hits": 11 },
              "state": "Queued", "stack": null, "date": "9 jun",
              "list_badge": "na fila #1", "change_id": "zxqkmrtp", "landed_ago": "",
              "drawer": {
                "union": { "batch": ["#128", "#129"], "verdict": "verde", "green": true },
                "cost": { "tokens_total": 425000, "usd": 0.34, "model_breakdown": [["opus-4.8", 280000]], "cache_savings": "cache poupou $3.10/14min" },
                "mirror": { "synced": true, "detail": "GitHub #128 ✓" },
                "intents": [{ "id": "a31", "title": "refresh TTL", "status": "LANDED", "charter": "o refresh deve reemitir um TTL completo", "context_json": "{...}", "diff": { "files": [{ "path": "auth/token.rs", "added": 12, "removed": 3 }], "hunks": [] }, "verdicts": [{ "verdict": "APPROVE", "reviewer": "opus-4.8", "summary": "correctness confirmed", "adversarial": true, "lens": "correctness", "evidence_mono_terms": ["iat", "exp"] }], "model": "opus-4.8", "envelope": { "context_cas": "cas:7e1a…", "context_size": "4.2 KB", "compact_context_ref": "cas:2c91… · 2 KB", "bundle_ref": "cas:8d40… · 1.6 MB", "compact_transcript_ref": "cas:55c1… · 9 KB" }, "blame_quote": null, "blame_proof": null }],
                "files": [{ "path": "auth/token.rs", "added": 12, "removed": 3 }],
                "conflict_note": null, "summary_diff": null
              }
            }}],
            "orq_model": "opus-4.8", "window": "7–9 jun", "cost": "$0.83"
          }],
          "campaigns": [{ "id": "auth-hardening", "label": "Auth Hardening", "color_class": "c-auth", "display_label": "auth" }],
          "list_groups": [], "filter_pills": []
        }"##;
        let vm: LandingVm =
            serde_json::from_str(canonical).expect("landing JSON parses into LandingVm");
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.open_count, 24);
        let LandingItemVm::Card(card) = &vm.columns[0].items[0] else {
            panic!("first item must be a Card");
        };
        assert_eq!(card.number, 128);
        assert_eq!(card.state, PrState::Queued);
        assert_eq!(card.drawer.cost.usd, 0.34);
        assert_eq!(card.drawer.intents[0].verdicts[0].lens, "correctness");
        let reparsed: LandingVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "LandingVm round-trip is lossless");
        // F5: canonical JSON has no queue_position/eta_seconds — they must default to None.
        assert_eq!(
            vm.queue_position, None,
            "queue_position defaults to None when absent from JSON"
        );
        assert_eq!(
            vm.eta_seconds, None,
            "eta_seconds defaults to None when absent from JSON"
        );
    }

    /// F5: `LandingVm.queue_position` is `Some` when a queued PR exists,
    /// and `eta_seconds` stays `None` (no estimator seam). Exercises the
    /// real-populated path and the honest-None path in the same fixture.
    #[test]
    fn landing_vm_queue_position_some_eta_seconds_none() {
        let json_with_queue = r#"{
          "repo": "hugit",
          "main_green": false,
          "main_status": "",
          "open_count": 1,
          "columns": [],
          "campaigns": [],
          "queue_position": 1,
          "eta_seconds": null
        }"#;
        let vm: LandingVm =
            serde_json::from_str(json_with_queue).expect("LandingVm with queue_position parses");
        assert_eq!(
            vm.queue_position,
            Some(1),
            "queue_position Some(1) round-trips"
        );
        assert_eq!(vm.eta_seconds, None, "eta_seconds None round-trips");
        let back: LandingVm = serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(back.queue_position, Some(1));
        assert_eq!(back.eta_seconds, None);
    }
}
