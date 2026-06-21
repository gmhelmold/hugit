//! `GET /v1/repos/{repo}/knowledge` → [`KnowledgeVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. The knowledge-index seam
//! (vector search, BYOK synthesis, citability graph) is an explicitly gated P2
//! feature (`engine_note` below). This handler returns an HONEST-DEFAULT stub:
//! no data is fabricated, no fields are guessed. Every honest-default is
//! documented at the field site with `// HONEST-DEFAULT — P2 seam`.
//!
//! REAL backbone:
//! - `repo` — the router path param, echoed verbatim (structural, not scrubbed).
//!
//! HONEST-DEFAULT (P2 knowledge-index seam — no local source):
//! - `areas`     = `[]` — knowledge areas come from the vector index; no seam yet.
//! - `faq`       = `[]` — FAQ rows come from the citability graph; no seam yet.
//! - `answer.paragraphs` = `[]` — synthesis is BYOK; the index is P2.
//! - `engine_note`       = `"índice de conhecimento: seam P2"` — honest disclosure.
//!
//! All other static fields carry minimal honest defaults (empty strings / empty vecs).

use hugit_http_contracts::knowledge::{
    AnswerSegmentVm, KnowledgeAnswerVm, KnowledgeAreaVm, KnowledgeFaqVm, KnowledgeVm,
    ModelOptionVm, TrailStepVm,
};
use hugit_refstore::EventLog;

/// The honest-default note that surfaces to the client, identifying the P2 seam.
const ENGINE_NOTE: &str = "índice de conhecimento: seam P2";

/// Build the knowledge view-model (router calls `build_knowledge(log, repo)`).
///
/// The log is accepted to keep the signature consistent with all other
/// collection-read handlers (`build_<screen>(log, repo)`). It is intentionally
/// unused here — the P2 index seam means there are no log records to fold.
/// The `#[allow(unused_variables)]` below prevents a compiler warning without
/// renaming the parameter (the caller API is frozen).
#[allow(unused_variables)]
pub fn build_knowledge(log: &EventLog, repo: &str) -> KnowledgeVm {
    KnowledgeVm {
        repo: repo.to_string(),         // REAL — router path param
        lead: String::new(),            // HONEST-DEFAULT — no copy seam
        ask_placeholder: String::new(), // HONEST-DEFAULT
        model: String::new(),           // HONEST-DEFAULT — no BYOK model selected
        key_label: String::new(),       // HONEST-DEFAULT — no key seam
        cost_est: String::new(),        // HONEST-DEFAULT (#[serde(default)] field)
        keyline: String::new(),         // HONEST-DEFAULT
        models: vec![],                 // HONEST-DEFAULT — no model-option seam
        examples: vec![],               // HONEST-DEFAULT — no example seam
        answer: KnowledgeAnswerVm {
            question: String::new(),      // HONEST-DEFAULT
            paragraphs: vec![],           // HONEST-DEFAULT — P2 synthesis seam
            trail: vec![],                // HONEST-DEFAULT — P2 citability seam
            anchored_note: String::new(), // HONEST-DEFAULT
            cost_note: String::new(),     // HONEST-DEFAULT
        },
        areas: vec![], // HONEST-DEFAULT — P2 knowledge-index seam
        faq: vec![],   // HONEST-DEFAULT — P2 citability-graph seam
        engine_note: ENGINE_NOTE.to_string(), // honest disclosure of the P2 gate
    }
}

// ── unused-type suppressors (keep dead-code warnings away from re-exported VM ─
// The contract types below are imported by the contracts crate and used in the
// populated test; the compiler would warn about "unused import" without them.
const _: fn() = || {
    let _ = AnswerSegmentVm::Text {
        text: String::new(),
    };
    let _: TrailStepVm;
    let _: KnowledgeAreaVm;
    let _: KnowledgeFaqVm;
    let _: ModelOptionVm;
};

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_http_contracts::knowledge::KnowledgeVm;

    // ── empty-log test (the canonical "honest stub" baseline) ────────────────

    #[test]
    fn empty_log_returns_honest_defaults() {
        let vm = build_knowledge(&EventLog::new(), "humangr/hugit");

        // REAL field.
        assert_eq!(vm.repo, "humangr/hugit");

        // P2-gated seams: must be empty, never fabricated.
        assert!(vm.areas.is_empty(), "areas must be empty (P2 seam)");
        assert!(vm.faq.is_empty(), "faq must be empty (P2 seam)");
        assert!(
            vm.answer.paragraphs.is_empty(),
            "answer.paragraphs must be empty (P2 synthesis seam)"
        );
        assert!(
            vm.answer.trail.is_empty(),
            "answer.trail must be empty (P2 seam)"
        );

        // Engine note discloses the gate honestly.
        assert_eq!(vm.engine_note, ENGINE_NOTE);

        // Other honest-defaults (non-fabricated).
        assert!(vm.models.is_empty());
        assert!(vm.examples.is_empty());
        assert!(vm.model.is_empty());
        assert!(vm.key_label.is_empty());
        assert!(vm.cost_est.is_empty());
    }

    // ── populated test (round-trip fidelity over the full VM shape) ──────────

    #[test]
    fn populated_vm_round_trips() {
        let vm = KnowledgeVm {
            repo: "acme/myrepo".to_string(),
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
                        href: "/r/acme/myrepo/intent/a31".to_string(),
                    },
                    AnswerSegmentVm::Mono {
                        text: "iat".to_string(),
                    },
                ]],
                trail: vec![TrailStepVm {
                    icon: "○".to_string(),
                    label: "intent a31".to_string(),
                    href: "/r/acme/myrepo/intent/a31".to_string(),
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

        // Spot-check the internally-tagged enum encoding.
        assert!(json.contains("\"kind\":\"cite\""));
        assert!(json.contains("\"kind\":\"mono\""));
    }
}
