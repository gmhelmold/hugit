//! `GET /v1/me/attention` → `AttentionVm` and its nested types.
//! Shared atom (`VerdictVm`) comes from [`crate::common`]. `AttentionDecisionVm`
//! and `AttentionVm` derive `PartialEq` only — the source containers omit `Eq`.

use serde::{Deserialize, Serialize};

use crate::common::VerdictVm;

/// One evidence section inside a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionEvidenceVm {
    pub section: String,
    pub lines: Vec<String>,
}

/// One action button on a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionActionVm {
    pub label: String,
    /// Navigation target when the action is a link; None = write action.
    pub href: Option<String>,
}

/// One ranked decision. `PartialEq` only (source container omits `Eq`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionDecisionVm {
    pub kind: String,
    pub type_label: String,
    pub signal_class: String,
    pub title: String,
    pub repo: String,
    pub age: String,
    pub why: String,
    pub verdicts: Vec<VerdictVm>,
    pub evidence: Vec<AttentionEvidenceVm>,
    pub actions: Vec<AttentionActionVm>,
    pub note: Option<String>,
    /// The repo the decision's land/verdict write actions POST against.
    pub target_repo: Option<String>,
    /// The PR number the land/verdict write actions POST against.
    pub target_pr: Option<u64>,
}

/// `PartialEq` only (source container omits `Eq`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionVm {
    /// Display order = rank.
    pub decisions: Vec<AttentionDecisionVm>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_vm_round_trips() {
        let vm = AttentionVm {
            decisions: vec![AttentionDecisionVm {
                kind: "land".into(),
                type_label: "LAND".into(),
                signal_class: "g".into(),
                title: "PR #128 pronto pra land".into(),
                repo: "corelink-server".into(),
                age: "há 38 min".into(),
                why: "política: land-ready · alcance: main · confiança: 100%".into(),
                verdicts: vec![VerdictVm {
                    verdict: "APPROVE".into(),
                    reviewer: "opus-4.8".into(),
                    summary: "correctness verificada".into(),
                    adversarial: true,
                    lens: "correctness".into(),
                    evidence_mono_terms: vec!["iat".into()],
                }],
                evidence: vec![AttentionEvidenceVm {
                    section: "Prova".into(),
                    lines: vec!["checks 11 verdes · byte-idêntico ×3".into()],
                }],
                actions: vec![
                    AttentionActionVm {
                        label: "Aprovar e land →".into(),
                        href: None,
                    },
                    AttentionActionVm {
                        label: "Ver PR".into(),
                        href: Some("/r/corelink-server/pr/128".into()),
                    },
                ],
                note: Some("aprovar não pula a fila.".into()),
                target_repo: Some("corelink-server".into()),
                target_pr: Some(128),
            }],
        };
        let json = serde_json::to_string(&vm).expect("AttentionVm serializes");
        let reparsed: AttentionVm = serde_json::from_str(&json).expect("AttentionVm deserializes");
        assert_eq!(vm, reparsed, "AttentionVm round-trip is lossless");
    }
}
