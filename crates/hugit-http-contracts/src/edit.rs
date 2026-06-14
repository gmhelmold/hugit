//! `GET /v1/repos/{repo}/edit/{*path}` → `EditVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

use crate::common::HunkVm;

/// One editor line; `edited` lines carry the ● gutter marker + highlight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditLineVm {
    pub number: u32,
    pub text: String,
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditVm {
    pub repo: String,
    pub path: String,
    pub workspace_note: String,
    pub lines: Vec<EditLineVm>,
    pub changed_note: String,
    #[serde(default)]
    pub changed_count: String,
    /// The Preview diff — reused wave-1 hunk.
    pub preview: HunkVm,
    pub honesty_note: String,
    pub commit_title: String,
    pub commit_description: String,
    pub direct_main_label: String,
    pub direct_main_reason: String,
    pub branch_label: String,
    pub branch_note: String,
    pub authorship_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DiffLineKind, DiffLineVm};

    #[test]
    fn edit_vm_round_trips() {
        let vm = EditVm {
            repo: "humangr/corelink-server".to_string(),
            path: "crates/auth/src/token.rs".to_string(),
            workspace_note: "Workspace efêmero, só seu.".to_string(),
            lines: vec![
                EditLineVm {
                    number: 41,
                    text: "    let exp = now();".to_string(),
                    edited: false,
                },
                EditLineVm {
                    number: 42,
                    text: "    let exp = now() + TTL;".to_string(),
                    edited: true,
                },
            ],
            changed_note: "2 linhas alteradas".to_string(),
            changed_count: "2 linhas".to_string(),
            preview: HunkVm {
                file: "crates/auth/src/token.rs".to_string(),
                header: "@@ -41,2 +41,2 @@".to_string(),
                lines: vec![
                    DiffLineVm {
                        kind: DiffLineKind::Del,
                        text: "-    let exp = now();".to_string(),
                        ln: "41".to_string(),
                    },
                    DiffLineVm {
                        kind: DiffLineKind::Add,
                        text: "+    let exp = now() + TTL;".to_string(),
                        ln: "42".to_string(),
                    },
                ],
            },
            honesty_note: "propor vira um commit SEU (mudança externa assinada).".to_string(),
            commit_title: "fix: sessão expira cedo no refresh".to_string(),
            commit_description: "A expiração usava now() sem o TTL.".to_string(),
            direct_main_label: "commit direto na main".to_string(),
            direct_main_reason: "main é single-writer.".to_string(),
            branch_label: "criar branch gustavo/edit-token-rs e abrir PR".to_string(),
            branch_note: "branch criada a partir de main · PR rascunho.".to_string(),
            authorship_note: "A autoria da mudança é sua — assinada com sua identidade HuGR."
                .to_string(),
        };
        let json = serde_json::to_string(&vm).expect("EditVm serializes");
        let reparsed: EditVm = serde_json::from_str(&json).expect("EditVm deserializes");
        assert_eq!(vm, reparsed, "EditVm round-trip is lossless");
        let minimal = r#"{"repo":"r","path":"p","workspace_note":"w","lines":[],
            "changed_note":"c","preview":{"file":"f","header":"h","lines":[]},
            "honesty_note":"h","commit_title":"t","commit_description":"d",
            "direct_main_label":"l","direct_main_reason":"r","branch_label":"b",
            "branch_note":"n","authorship_note":"a"}"#;
        let m: EditVm = serde_json::from_str(minimal).expect("minimal EditVm parses");
        assert_eq!(m.changed_count, "");
    }
}
