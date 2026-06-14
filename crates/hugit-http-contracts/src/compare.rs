//! `GET /v1/repos/{repo}/compare/{base}/{head}` → `CompareVm`.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All-`Eq` (no floats).

use serde::{Deserialize, Serialize};

use crate::commits::CommitRowVm;
use crate::common::DiffVm;

/// `GET /v1/repos/{repo}/compare/{base}/{head}` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompareVm {
    pub repo: String,
    pub base: String,
    pub head: String,
    pub branches: Vec<String>,
    pub generated_branches: Vec<String>,
    pub can_merge: bool,
    pub stats_note: String,
    /// Commits between the refs — reused wave-2 row.
    pub commits: Vec<CommitRowVm>,
    pub commits_note: String,
    /// Reused wave-1 diff family (files + hunks).
    pub diff: DiffVm,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DiffLineKind, DiffLineVm, FileRowVm, HunkVm};

    #[test]
    fn compare_vm_round_trips() {
        let commit = CommitRowVm {
            message: "fix: sessão expira cedo no refresh".to_string(),
            author: "opus-4.8".to_string(),
            avatar_class: "opus".to_string(),
            age: "há 38 min".to_string(),
            intent_id: Some("a31".to_string()),
            sha: "a31f9c".to_string(),
            checks_ok: true,
        };
        let diff = DiffVm {
            files: vec![FileRowVm {
                path: "crates/auth/src/token.rs".to_string(),
                added: 1,
                removed: 1,
            }],
            hunks: vec![HunkVm {
                file: "crates/auth/src/token.rs".to_string(),
                header: "@@ -40,7 +40,7 @@".to_string(),
                lines: vec![DiffLineVm {
                    kind: DiffLineKind::KeyAdd,
                    text: "+   let exp = now() + TTL;".to_string(),
                    ln: "41".to_string(),
                }],
            }],
        };
        let vm = CompareVm {
            repo: "hugit".to_string(),
            base: "main".to_string(),
            head: "feat/sessions".to_string(),
            branches: vec!["main".to_string(), "feat/sessions".to_string()],
            generated_branches: vec!["intent/a31".to_string()],
            can_merge: true,
            stats_note: "3 commits · 2 arquivos · +38 −9".to_string(),
            commits: vec![commit],
            commits_note: "3 commits · push manual de @ana.".to_string(),
            diff,
        };
        let json = serde_json::to_string(&vm).expect("CompareVm serializes");
        let reparsed: CompareVm = serde_json::from_str(&json).expect("CompareVm round-trips");
        assert_eq!(vm, reparsed, "CompareVm round-trip is lossless");
    }
}
