//! `GET /v1/me/import` → `ImportVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

/// One candidate row of "Ou escolha dos seus".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRepoVm {
    pub org: String,
    pub name: String,
    pub stack: String,
    pub counts: String,
    pub visibility: String,
    /// True = "✓ já vive aqui" (already imported; no button).
    pub imported: bool,
}

/// One step of the import pipeline strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportStepVm {
    pub title: String,
    pub detail: String,
    pub sub: Option<String>,
}

/// Import page — ALL render data, no Option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportVm {
    pub lead: String,
    pub url_placeholder: String,
    pub fidelity_note: String,
    pub candidates: Vec<ImportRepoVm>,
    pub deep_link_note: String,
    pub steps: Vec<ImportStepVm>,
    pub done_note: String,
    pub exit_note: String,
    #[serde(default)]
    pub url_value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_vm_round_trips() {
        let vm = ImportVm {
            lead: "Um comando: histórico, issues, PRs.".into(),
            url_placeholder: "github.com/org/repo — cole a URL e pronto".into(),
            fidelity_note: "público ou privado · LFS incluso.".into(),
            candidates: vec![ImportRepoVm {
                org: "humangr".into(),
                name: "hugr-wallet".into(),
                stack: "Rust · 1.9 GB".into(),
                counts: "97 PRs · 41 issues".into(),
                visibility: "privado".into(),
                imported: false,
            }],
            deep_link_note: "o resto continua no GitHub — a githugr deep-linka.".into(),
            steps: vec![
                ImportStepVm {
                    title: "Clone via protocolo git".into(),
                    detail: "histórico completo 2 941 commits · 1.9 GB".into(),
                    sub: Some("3 fora de contrato — marcados como externos".into()),
                },
                ImportStepVm {
                    title: "Issues e PRs".into(),
                    detail: "97 PRs · 41 issues · threads".into(),
                    sub: None,
                },
            ],
            done_note: "O GitHub continua sendo a casa — o espelho é vivo.".into(),
            exit_note: "Sair é um recurso, por escrito: o export completo.".into(),
            url_value: "github.com/humangr/hugr-wallet".into(),
        };
        let json = serde_json::to_string(&vm).expect("serialize");
        let reparsed: ImportVm = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(vm, reparsed, "ImportVm round-trip is lossless");
        let no_url = r#"{"lead":"l","url_placeholder":"p","fidelity_note":"f","candidates":[],"deep_link_note":"d","steps":[],"done_note":"dn","exit_note":"en"}"#;
        let parsed: ImportVm = serde_json::from_str(no_url).expect("missing url_value");
        assert_eq!(parsed.url_value, "");
    }
}
