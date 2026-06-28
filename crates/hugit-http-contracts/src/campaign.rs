//! `GET /v1/repos/{repo}/campaigns/{name}` → `CampaignVm` and its campaign-
//! specific nested types. `CampaignWhoVm` is module-local; shared atoms come
//! from their defining sibling modules (re-exported flat by `lib.rs`).
//! Transcribed BYTE-FOR-FIELD; `CampaignVm` derives `PartialEq` only (embeds
//! `CostSplitVm` which holds `f64`) — `Eq` is deliberately absent.

use serde::{Deserialize, Serialize};

use crate::common::{CampaignChipVm, EnvelopeVm, MirrorVm};
use crate::landing::{CampaignPrVm, OriginRefVm};
use crate::pr_detail::{CostSplitVm, StatVm};

/// The structured role of a [`CampaignWhoVm`] rail row, beside the localized
/// `role` prose. The variants mirror the engine's real principal classes
/// (`PrincipalClass::{Human, Orchestrator, Worker, Model}` in
/// `hugit-refstore/authz`) — NOT invented roles. `#[default] Unknown` is the
/// honest value for an absent/forward-compat payload (no role asserted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WhoRole {
    #[default]
    Unknown,
    /// The human stakeholder — decides, approves, interrogates.
    Human,
    /// The orchestrator (tech-lead seat) — plans, dispatches, lands.
    Orchestrator,
    /// A worker agent — executes one intent within a campaign.
    Worker,
    /// A model — a first-class identity with permissions/budgets/attribution.
    Model,
}

/// The structured status of a [`CampaignWhoVm`] rail row, beside the localized
/// `status` prose. `#[default] Unknown` is the honest value for an absent payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WhoStatus {
    #[default]
    Unknown,
    Active,
    Pending,
    Done,
}

/// One "Quem" rail row (operator / orchestrator / reviewer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignWhoVm {
    pub name: String,
    pub role: String,
    pub status: String,
    /// Structured role discriminant beside the localized `role` prose (githugr
    /// styles from this). Mirrors the engine's principal classes. Additive +
    /// forward-compat: an old payload without it defaults to [`WhoRole::Unknown`].
    /// NOTE: no live builder constructs `CampaignWhoVm` yet (the campaign `who`
    /// rail is a STUB — `build_campaign` emits `who: vec![]`); these structured
    /// fields are carried on the contract so githugr can light them the moment a
    /// builder lands. The prose `role`/`status` stay.
    #[serde(default)]
    pub role_kind: WhoRole,
    /// Structured status discriminant beside the localized `status` prose.
    /// Additive + forward-compat → [`WhoStatus::Unknown`] when absent.
    #[serde(default)]
    pub status_kind: WhoStatus,
}

/// The campaign page view-model. `PartialEq` only — embeds `CostSplitVm` (f64).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignVm {
    pub repo: String,
    pub name: String,
    pub chip: CampaignChipVm,
    pub state_label: String,
    pub operator: String,
    /// True renders the ✓ — the campaign does not open unsigned (fail-closed).
    pub operator_signed: bool,
    pub operator_sig: String,
    pub session: String,
    pub model: String,
    pub window: String,
    pub pr_count: usize,
    pub intent_count: usize,
    pub why: String,
    pub acceptance_note: String,
    pub born_from: Vec<OriginRefVm>,
    pub docs: Vec<OriginRefVm>,
    pub stats: Vec<StatVm>,
    pub prs: Vec<CampaignPrVm>,
    pub envelope: EnvelopeVm,
    pub seal_note: Option<String>,
    pub cost: CostSplitVm,
    pub bundle_status: Vec<(String, String)>,
    pub who: Vec<CampaignWhoVm>,
    pub attested_label: String,
    pub envelope_cas: String,
    pub transcripts_label: String,
    pub mirror: MirrorVm,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landing::CampaignIntentChipVm;

    /// `CampaignVm` (the f64-bearing campaign page VM) round-trips losslessly.
    #[test]
    fn campaign_vm_round_trips() {
        let vm = CampaignVm {
            repo: "acme/myrepo".to_string(),
            name: "auth-hardening".to_string(),
            chip: CampaignChipVm {
                id: "auth".to_string(),
                label: "auth-hardening".to_string(),
                color_class: "c-auth".to_string(),
                display_label: "auth".to_string(),
            },
            state_label: "bundle em voo · 2 de 3 pousaram".to_string(),
            operator: "gustavo".to_string(),
            operator_signed: true,
            operator_sig: "ed25519:9f2c…".to_string(),
            session: "orq-014".to_string(),
            model: "opus-4.8".to_string(),
            window: "7–9 jun".to_string(),
            pr_count: 3,
            intent_count: 5,
            why: "reforçar a camada de auth contra clock-skew e replay".to_string(),
            acceptance_note: "aceite: zero findings repetidos no re-teste".to_string(),
            born_from: vec![OriginRefVm {
                label: "#412".to_string(),
                detail: "refresh expira cedo em clock skew".to_string(),
                scanned: false,
            }],
            docs: vec![OriginRefVm {
                label: "pentest-maio.pdf".to_string(),
                detail: "cas:91c2…".to_string(),
                scanned: true,
            }],
            stats: vec![StatVm {
                value: "$0.34".to_string(),
                label: "custo total".to_string(),
                sub: "3 PRs".to_string(),
            }],
            prs: vec![CampaignPrVm {
                number: 128,
                title: "fix: refresh expira cedo".to_string(),
                intent_count: 2,
                cost_usd_micros: 60_000,
                union_label: "verde".to_string(),
                state_label: "pousou ✓".to_string(),
                landed: true,
                why: "o refresh deve reemitir um TTL completo".to_string(),
                intents: vec![CampaignIntentChipVm {
                    id: "a31".to_string(),
                    note: "refresh · opus-4.8".to_string(),
                }],
                state: crate::landing::PrState::Landed,
            }],
            envelope: EnvelopeVm {
                session: "orq-014".to_string(),
                model: "opus-4.8".to_string(),
                window: "7–9 jun".to_string(),
                headline: "planejou a wave auth em 3 PRs".to_string(),
                transcript_complete: true,
                session_summary: "planejou e selou o bundle".to_string(),
                compact_transcript_ref: "cas:b81c… · 41 KB".to_string(),
                raw_transcript_ref: "cas:9d4f… · 2.1 MB".to_string(),
                snapshot_files: vec!["notes/wave-auth.md".to_string()],
                context_json: "{}".to_string(),
                context_cas: "cas:a7e0…".to_string(),
                compact_context_ref: "cas:6d28…".to_string(),
                compact_json_note: "4 KB".to_string(),
                bundle_note: "4.1 MB · cas:5fc2…".to_string(),
            },
            seal_note: None,
            cost: CostSplitVm {
                author_line: "aberto pelo orquestrador orq-014".to_string(),
                author_badge: Some("nunca subagent".to_string()),
                author_suffix: Some("— a coordenação é da sessão da campanha".to_string()),
                work_usd: 0.21,
                work_note: "3 intents".to_string(),
                orchestration_usd: 0.03,
                orchestration_note: "coordenação".to_string(),
                verification_usd: 0.06,
                verification_note: "painel adversarial".to_string(),
                ci_usd: 0.04,
                ci_note: "checks executados".to_string(),
                total_usd: 0.34,
                waste_usd: 0.0,
                waste_note: "zero re-trabalho".to_string(),
                overhead_pct: 15,
                cache_savings_pct: 98,
                time_note: "2.1d span · 1h08m soma de agente".to_string(),
            },
            bundle_status: vec![
                ("união".to_string(), "verde".to_string()),
                ("selada".to_string(), "09 jun 16:40".to_string()),
            ],
            who: vec![CampaignWhoVm {
                name: "gustavo".to_string(),
                role: "operador humano".to_string(),
                status: "assinou ✓".to_string(),
                role_kind: WhoRole::Human,
                status_kind: WhoStatus::Done,
            }],
            attested_label: "5/5 assinados".to_string(),
            envelope_cas: "cas:a7e0… ✓".to_string(),
            transcripts_label: "100% · pra sempre".to_string(),
            mirror: MirrorVm {
                synced: true,
                detail: "GitHub #128 ✓".to_string(),
            },
        };

        let json = serde_json::to_string(&vm).expect("CampaignVm serializes");
        let reparsed: CampaignVm = serde_json::from_str(&json).expect("CampaignVm deserializes");
        assert_eq!(vm, reparsed, "CampaignVm round-trip is lossless");
        // The structured discriminants round-trip beside the prose.
        assert_eq!(reparsed.who[0].role_kind, WhoRole::Human);
        assert_eq!(reparsed.who[0].status_kind, WhoStatus::Done);
        assert_eq!(reparsed.prs[0].state, crate::landing::PrState::Landed);
    }

    /// `CampaignWhoVm`'s structured `role_kind`/`status_kind` default to `Unknown`
    /// (forward-compat) when absent from an old payload — the prose stays.
    #[test]
    fn campaign_who_vm_structured_fields_default_unknown() {
        let legacy = r#"{ "name": "ana", "role": "orquestrador", "status": "ativo" }"#;
        let who: CampaignWhoVm = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            who.role_kind,
            WhoRole::Unknown,
            "absent role_kind → Unknown"
        );
        assert_eq!(
            who.status_kind,
            WhoStatus::Unknown,
            "absent status_kind → Unknown"
        );
        assert_eq!(who.role, "orquestrador", "prose role preserved");
        // Round-trip with the structured fields populated.
        let full = CampaignWhoVm {
            name: "orq-014".to_string(),
            role: "orquestrador".to_string(),
            status: "ativo".to_string(),
            role_kind: WhoRole::Orchestrator,
            status_kind: WhoStatus::Active,
        };
        let reparsed: CampaignWhoVm =
            serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(full, reparsed);
    }
}
