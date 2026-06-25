//! `GET /v1/repos/{repo}/blob/{*path}` → `BlobVm` and its blob-specific nested
//! types. Transcribed BYTE-FOR-FIELD from the canonical
//! companion web frontend's view-model definitions.

use serde::{Deserialize, Serialize};

/// One source line. NO syntax highlight this wave — `text` is raw mono.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobLineVm {
    pub number: u32,
    pub text: String,
    pub hl: bool,
    /// The intent that last wrote this line; None = external/manual authorship.
    pub intent_id: Option<String>,
}

/// One symbol of the simple outline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineItemVm {
    pub kind: String,
    pub name: String,
    pub line: u32,
    #[serde(default)]
    pub active: bool,
}

/// One inline segment of a why-blame `reason` prose. Internally tagged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlobReasonSegVm {
    Text { text: String },
    Bold { text: String },
    Mono { text: String },
}

/// One why-blame entry — the deep `hugit why` card for a contiguous range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobBlameVm {
    pub line_start: u32,
    pub line_end: u32,
    pub intent_id: String,
    pub charter: String,
    pub href: String,
    pub principal: String,
    pub cost: String,
    pub verdict: String,
    pub proof: String,
    pub attestation: String,
    pub reason: String,
    #[serde(default)]
    pub proof_ok: bool,
    #[serde(default)]
    pub cost_tokens: String,
    #[serde(default)]
    pub cost_usd_str: String,
    #[serde(default)]
    pub cost_duration: String,
    #[serde(default)]
    pub principal_segments: Vec<(String, bool)>,
    #[serde(default)]
    pub attestation_ok: bool,
    #[serde(default)]
    pub reason_segments: Vec<BlobReasonSegVm>,
}

/// One node of the blob sidebar's "Arquivos" file tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobTreeRowVm {
    pub name: String,
    /// Repo-relative path for navigation, e.g. "crates/auth/src/token.rs". A
    /// directory carries a trailing slash, e.g. "crates/auth/". The consumer
    /// builds `/r/{repo}/blob/{path}` links from it, so it is REQUIRED on the
    /// wire (a missing `path` is a hard decode error for the window). Scrubbed
    /// at the read boundary like `name`. `#[serde(default)]` only covers older
    /// pre-`path` fixtures on the producer side; the engine always populates it.
    #[serde(default)]
    pub path: String,
    pub depth: u32,
    pub is_dir: bool,
    pub current: bool,
}

/// The blob view-model: one file at a ref with why-blame, outline, tree sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobVm {
    pub repo: String,
    pub path: String,
    pub size: String,
    pub lang: Option<String>,
    pub via_intent: Option<String>,
    pub via_model: Option<String>,
    pub lines: Vec<BlobLineVm>,
    pub blame: Vec<BlobBlameVm>,
    pub outline: Vec<OutlineItemVm>,
    pub actions: Vec<String>,
    #[serde(default)]
    pub tree: Vec<BlobTreeRowVm>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BlobVm` round-trips losslessly (all nested types, every enum variant).
    #[test]
    fn blob_vm_round_trips() {
        let vm = BlobVm {
            repo: "acme/myrepo".into(),
            path: "crates/auth/src/token.rs".into(),
            size: "2.1 KB".into(),
            lang: Some("rust".into()),
            via_intent: Some("a31".into()),
            via_model: Some("opus-4.8".into()),
            lines: vec![
                BlobLineVm {
                    number: 1,
                    text: "pub fn refresh(token: &str) -> Result<Token> {".into(),
                    hl: false,
                    intent_id: Some("a31".into()),
                },
                BlobLineVm {
                    number: 2,
                    text: "    let exp = now() + TTL;".into(),
                    hl: true,
                    intent_id: Some("a31".into()),
                },
                BlobLineVm {
                    number: 3,
                    text: "}".into(),
                    hl: false,
                    intent_id: None,
                },
            ],
            blame: vec![BlobBlameVm {
                line_start: 1,
                line_end: 3,
                intent_id: "a31".into(),
                charter: "o refresh deve reemitir um TTL completo — rotate-on-use".into(),
                href: "/r/acme/myrepo/intent/a31".into(),
                principal: "você → orquestrador opus-4.8 → implementer opus-4.8".into(),
                cost: "18.4k tokens · $0.21 · 2m 12s".into(),
                verdict: "✓ correctness · ✓ security · ✓ repro".into(),
                proof: "test_refresh_full_ttl ✓".into(),
                attestation: "tree 9f2e1a4 · runner box-A · sig ed25519:7c… ✓".into(),
                reason: "esta função existe porque o refresh precisa da expiração atual".into(),
                proof_ok: true,
                cost_tokens: "18.4k".into(),
                cost_usd_str: "$0.21".into(),
                cost_duration: "2m 12s".into(),
                principal_segments: vec![
                    ("você → orquestrador ".into(), false),
                    ("opus-4.8".into(), true),
                ],
                attestation_ok: true,
                reason_segments: vec![
                    BlobReasonSegVm::Text {
                        text: "esta função existe porque o ".into(),
                    },
                    BlobReasonSegVm::Bold {
                        text: "refresh".into(),
                    },
                    BlobReasonSegVm::Text {
                        text: " precisa de ".into(),
                    },
                    BlobReasonSegVm::Mono { text: "exp".into() },
                ],
            }],
            outline: vec![
                OutlineItemVm {
                    kind: "fn".into(),
                    name: "refresh".into(),
                    line: 1,
                    active: true,
                },
                OutlineItemVm {
                    kind: "c".into(),
                    name: "TTL".into(),
                    line: 42,
                    active: false,
                },
            ],
            actions: vec![
                "Raw".into(),
                "Copiar".into(),
                "✎ Editar".into(),
                "Blame".into(),
            ],
            tree: vec![
                BlobTreeRowVm {
                    name: "auth/".into(),
                    path: "crates/auth/".into(),
                    depth: 0,
                    is_dir: true,
                    current: false,
                },
                BlobTreeRowVm {
                    name: "token.rs".into(),
                    path: "crates/auth/token.rs".into(),
                    depth: 1,
                    is_dir: false,
                    current: true,
                },
            ],
        };

        let json = serde_json::to_string(&vm).expect("BlobVm serializes");
        let reparsed: BlobVm = serde_json::from_str(&json).expect("BlobVm deserializes");
        assert_eq!(vm, reparsed, "BlobVm round-trip is lossless");
        assert!(json.contains("\"kind\":\"bold\""));
        assert!(json.contains("\"kind\":\"mono\""));

        let no_active = r#"{"kind":"fn","name":"foo","line":1}"#;
        let item: OutlineItemVm = serde_json::from_str(no_active).expect("OutlineItemVm parses");
        assert!(!item.active);
    }
}
