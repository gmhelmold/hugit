//! `GET /v1/repos/{repo}/commit/{sha}` → `CommitDetailVm`.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

use crate::common::DiffVm;

/// A raw/external commit — the git-native view. Fleet commits live on their
/// intent page; this covers manual/external pushes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetailVm {
    pub repo: String,
    pub sha: String,
    pub parent_sha: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub age: String,
    /// True = manual push (the "externa" tag + provenance line).
    pub external: bool,
    pub provenance_note: String,
    /// Fleet commit → its intent: the page defers to the intent view.
    pub intent_id: Option<String>,
    pub checks_ok: bool,
    pub checks_summary: String,
    pub checks_detail: String,
    /// Reused wave-1 diff family.
    pub diff: DiffVm,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DiffLineKind, DiffLineVm, FileRowVm, HunkVm};

    #[test]
    fn commit_detail_vm_round_trips() {
        let vm = CommitDetailVm {
            repo: "humangr/corelink-server".to_string(),
            sha: "f3a91c".to_string(),
            parent_sha: "a31f9c".to_string(),
            title: "fix: sessão expira cedo no refresh".to_string(),
            description: "A expiração usava now() sem o TTL.".to_string(),
            author: "ana".to_string(),
            age: "commitou há 2 h".to_string(),
            external: true,
            provenance_note: "push manual de @ana — mudança externa.".to_string(),
            intent_id: None,
            checks_ok: true,
            checks_summary: "8 checks passaram".to_string(),
            checks_detail: "· 7 do cache · 41 s".to_string(),
            diff: DiffVm {
                files: vec![FileRowVm {
                    path: "crates/auth/src/token.rs".to_string(),
                    added: 1,
                    removed: 1,
                }],
                hunks: vec![HunkVm {
                    file: "crates/auth/src/token.rs".to_string(),
                    header: "@@ -41,1 +41,1 @@".to_string(),
                    lines: vec![DiffLineVm {
                        kind: DiffLineKind::Add,
                        text: "+    let exp = now() + TTL;".to_string(),
                        ln: "42".to_string(),
                    }],
                }],
            },
        };
        let json = serde_json::to_string(&vm).expect("CommitDetailVm serializes");
        let reparsed: CommitDetailVm = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(vm, reparsed, "CommitDetailVm round-trip is lossless");
        let mut with_intent = vm.clone();
        with_intent.intent_id = Some("a31".to_string());
        let j2 = serde_json::to_string(&with_intent).unwrap();
        let r2: CommitDetailVm = serde_json::from_str(&j2).unwrap();
        assert_eq!(with_intent, r2);
    }
}
