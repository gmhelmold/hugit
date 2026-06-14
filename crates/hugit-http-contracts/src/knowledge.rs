//! `GET /v1/repos/{repo}/knowledge` → `KnowledgeVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

/// One segment of an answer paragraph: plain prose, a navigable citation, or an
/// inline monospace token. Internally tagged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerSegmentVm {
    Text { text: String },
    Cite { label: String, href: String },
    Mono { text: String },
}

/// One step of the evidence trail under the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrailStepVm {
    pub icon: String,
    pub label: String,
    pub href: String,
}

/// The rendered answer card — pre-answered this wave (read-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeAnswerVm {
    pub question: String,
    pub paragraphs: Vec<Vec<AnswerSegmentVm>>,
    pub trail: Vec<TrailStepVm>,
    pub anchored_note: String,
    pub cost_note: String,
}

/// One BYOK model option of the chip menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOptionVm {
    pub name: String,
    pub est: String,
    pub selected: bool,
}

/// One "O que o repo sabe" area card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeAreaVm {
    pub name: String,
    pub color_class: String,
    pub owner: String,
    pub lines: Vec<(String, String)>,
}

/// One FAQ row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeFaqVm {
    pub question: String,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeVm {
    pub repo: String,
    pub lead: String,
    pub ask_placeholder: String,
    pub model: String,
    pub key_label: String,
    #[serde(default)]
    pub cost_est: String,
    pub keyline: String,
    pub models: Vec<ModelOptionVm>,
    pub examples: Vec<String>,
    pub answer: KnowledgeAnswerVm,
    pub areas: Vec<KnowledgeAreaVm>,
    pub faq: Vec<KnowledgeFaqVm>,
    pub engine_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_vm_round_trips() {
        let vm = KnowledgeVm {
            repo: "humangr/corelink-server".to_string(),
            lead: "Pergunte ao repo.".to_string(),
            ask_placeholder: "pergunte qualquer coisa…".to_string(),
            model: "opus-4.8".to_string(),
            key_label: "anth-•••8k2".to_string(),
            cost_est: "~$0.02/pergunta".to_string(),
            keyline: "sua chave: anth-•••8k2".to_string(),
            models: vec![ModelOptionVm {
                name: "opus-4.8".to_string(),
                est: "~$0.02/pergunta".to_string(),
                selected: true,
            }],
            examples: vec!["por que o refresh tolera ±2s de skew?".to_string()],
            answer: KnowledgeAnswerVm {
                question: "por que o refresh tolera ±2s de skew?".to_string(),
                paragraphs: vec![vec![
                    AnswerSegmentVm::Text {
                        text: "O TTL conta do recebimento — veja ".to_string(),
                    },
                    AnswerSegmentVm::Cite {
                        label: "a31".to_string(),
                        href: "/r/humangr/corelink-server/intent/a31".to_string(),
                    },
                    AnswerSegmentVm::Mono {
                        text: "iat".to_string(),
                    },
                ]],
                trail: vec![TrailStepVm {
                    icon: "○".to_string(),
                    label: "intent a31".to_string(),
                    href: "/r/humangr/corelink-server/intent/a31".to_string(),
                }],
                anchored_note: "ancorado em proveniência assinada".to_string(),
                cost_note: "custou $0.013 · opus-4.8 · sua chave (BYOK)".to_string(),
            },
            areas: vec![KnowledgeAreaVm {
                name: "auth".to_string(),
                color_class: "c-auth".to_string(),
                owner: "auth-hardening · gustavo".to_string(),
                lines: vec![(
                    "TTL conta do recebimento".to_string(),
                    "desde a31".to_string(),
                )],
            }],
            faq: vec![KnowledgeFaqVm {
                question: "qual é o modelo de custo?".to_string(),
                tag: "auth · 2 citações".to_string(),
            }],
            engine_note: "índice citável construído no land · síntese BYOK".to_string(),
        };

        let json = serde_json::to_string(&vm).expect("serializes");
        let reparsed: KnowledgeVm = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(vm, reparsed, "KnowledgeVm round-trip is lossless");
        assert!(json.contains("\"kind\":\"cite\""));
        let no_cost = r#"{"repo":"r","lead":"l","ask_placeholder":"p","model":"opus-4.8","key_label":"k","keyline":"kl","models":[],"examples":[],"answer":{"question":"q","paragraphs":[],"trail":[],"anchored_note":"an","cost_note":"cn"},"areas":[],"faq":[],"engine_note":"en"}"#;
        let r: KnowledgeVm = serde_json::from_str(no_cost).expect("parses without cost_est");
        assert_eq!(r.cost_est, "");
    }
}
