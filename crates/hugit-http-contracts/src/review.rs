//! `GET /v1/repos/{repo}/prs/{n}/review` → `ReviewVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical
//! companion web frontend's view-model definitions.

use serde::{Deserialize, Serialize};

use crate::common::{CampaignChipVm, HunkVm, VerdictVm};
use crate::pr_detail::PrReviewerVm;

/// Citation kind of a Q&A source chip — drives the leading glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewSourceKind {
    Ref,
    Proof,
    Context,
}

/// One Q&A source chip — a typed citation with a per-kind icon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSourceVm {
    pub label: String,
    pub kind: ReviewSourceKind,
    #[serde(default)]
    pub label_mono_terms: Vec<String>,
}

/// One pre-answered interrogation, read-only this wave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewQaVm {
    pub question: String,
    pub answer: String,
    pub sources: Vec<String>,
    #[serde(default)]
    pub answer_mono_terms: Vec<String>,
    #[serde(default)]
    pub answer_lnk_terms: Vec<String>,
    /// When non-empty this is authoritative and `sources` is ignored.
    #[serde(default)]
    pub typed_sources: Vec<ReviewSourceVm>,
}

/// One event of the conversation timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewEventVm {
    pub class: String,
    pub icon: String,
    pub text: String,
    pub age: String,
    /// Some = a comment card (the body).
    pub body: Option<String>,
    #[serde(default)]
    pub text_id_terms: Vec<String>,
    #[serde(default)]
    pub text_bold_terms: Vec<String>,
    #[serde(default)]
    pub text_meta_terms: Vec<String>,
}

/// One reply of the inline diff thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewReplyVm {
    pub who: String,
    pub meta: String,
    pub message: String,
    #[serde(default)]
    pub message_mono_terms: Vec<String>,
}

/// The inline thread anchored on a diff line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewThreadVm {
    pub hunk: HunkVm,
    pub anchor: String,
    pub replies: Vec<ReviewReplyVm>,
}

/// The review surface of a PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewVm {
    pub repo: String,
    pub number: u32,
    pub title: String,
    pub state_label: String,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub intent_count: usize,
    pub file_count: usize,
    pub added: u32,
    pub removed: u32,
    pub conversation_count: usize,
    pub qa: Vec<ReviewQaVm>,
    pub qa_note: String,
    pub timeline: Vec<ReviewEventVm>,
    pub thread: ReviewThreadVm,
    /// Adversarial panel: verdict + expanded evidence prose (tuple).
    pub verdicts: Vec<(VerdictVm, String)>,
    pub action_labels: Vec<String>,
    pub action_note: String,
    pub reviewers: Vec<PrReviewerVm>,
    pub assignee: String,
    pub assignee_note: String,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub milestone_progress: Option<(u32, u32)>,
    pub campaign: Option<CampaignChipVm>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DiffLineKind, DiffLineVm};

    /// `ReviewVm` (fully populated) round-trips losslessly through JSON.
    #[test]
    fn review_vm_round_trips() {
        let hunk = HunkVm {
            file: "auth/token.rs".to_string(),
            header: "@@ -40,6 +40,6 @@".to_string(),
            lines: vec![
                DiffLineVm {
                    kind: DiffLineKind::Context,
                    text: "    let ttl = TTL;".to_string(),
                    ln: "40".to_string(),
                },
                DiffLineVm {
                    kind: DiffLineKind::KeyAdd,
                    text: "+   let exp = now() + ttl;".to_string(),
                    ln: "41".to_string(),
                },
            ],
        };
        let vm = ReviewVm {
            repo: "acme/myrepo".to_string(),
            number: 128,
            title: "fix: sessão expira cedo no refresh".to_string(),
            state_label: "na fila de landing".to_string(),
            author: "opus-4.8".to_string(),
            source_branch: "feat/session-refresh".to_string(),
            target_branch: "main".to_string(),
            intent_count: 3,
            file_count: 2,
            added: 12,
            removed: 9,
            conversation_count: 4,
            qa: vec![ReviewQaVm {
                question: "O token usa o instante atual para `exp`?".to_string(),
                answer: "Sim — auth/token.rs:42 usa now() + TTL.".to_string(),
                sources: vec!["auth/token.rs:42 · intent a31".to_string()],
                answer_mono_terms: vec!["now()".to_string(), "TTL".to_string()],
                answer_lnk_terms: vec!["auth/token.rs:42 — ver intent a31".to_string()],
                typed_sources: vec![ReviewSourceVm {
                    label: "auth/token.rs:42 · intent a31".to_string(),
                    kind: ReviewSourceKind::Ref,
                    label_mono_terms: vec!["auth/token.rs:42".to_string()],
                }],
            }],
            qa_note: "ancorado em proveniência assinada".to_string(),
            timeline: vec![ReviewEventVm {
                class: "ok".to_string(),
                icon: "●".to_string(),
                text: "intent a31 adicionado · auth/token.rs · +12 −9".to_string(),
                age: "há 14 min".to_string(),
                body: None,
                text_id_terms: vec!["a31".to_string()],
                text_bold_terms: vec!["fix sessão refresh".to_string()],
                text_meta_terms: vec!["· auth/token.rs · +12 −9".to_string()],
            }],
            thread: ReviewThreadVm {
                hunk: hunk.clone(),
                anchor: "token.rs:42".to_string(),
                replies: vec![ReviewReplyVm {
                    who: "opus-4.8".to_string(),
                    meta: "há 1 min · resposta ancorada".to_string(),
                    message: "usa now() — o instante correto.".to_string(),
                    message_mono_terms: vec!["now()".to_string()],
                }],
            },
            verdicts: vec![(
                VerdictVm {
                    verdict: "APPROVE".to_string(),
                    reviewer: "correctness".to_string(),
                    summary: "iat/exp corretos.".to_string(),
                    adversarial: true,
                    lens: "correctness".to_string(),
                    evidence_mono_terms: vec![
                        "iat".to_string(),
                        "test_refresh_full_ttl".to_string(),
                    ],
                    decision: crate::common::VerdictDecision::Pass,
                },
                "O campo `exp` é derivado do instante atual — prova byte-idêntica ×3.".to_string(),
            )],
            action_labels: vec![
                "✓ Aprovar".to_string(),
                "✎ Pedir mudanças".to_string(),
                "✦ Interrogar…".to_string(),
            ],
            action_note: "union-test verde · 3/3 atestados · sem conflito".to_string(),
            reviewers: vec![PrReviewerVm {
                name: "gustavo".to_string(),
                state: "✓ aprovou".to_string(),
                approved: true,
            }],
            assignee: "opus-4.8".to_string(),
            assignee_note: "· orquestrador".to_string(),
            labels: vec!["auth".to_string()],
            milestone: Some("v1.0".to_string()),
            milestone_progress: Some((2, 3)),
            campaign: Some(CampaignChipVm {
                id: "auth-hardening".to_string(),
                label: "auth-hardening".to_string(),
                color_class: "c-auth".to_string(),
                display_label: "auth".to_string(),
            }),
        };

        let json = serde_json::to_string(&vm).expect("ReviewVm serializes");
        let reparsed: ReviewVm = serde_json::from_str(&json).expect("ReviewVm deserializes");
        assert_eq!(vm, reparsed, "ReviewVm round-trip is lossless");
        assert_eq!(reparsed.verdicts[0].0.verdict, "APPROVE");
        assert!(reparsed.qa[0].typed_sources[0].kind == ReviewSourceKind::Ref);
    }
}
