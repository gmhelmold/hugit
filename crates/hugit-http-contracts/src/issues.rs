//! `GET /v1/repos/{repo}/issues` → `IssuesVm` and its issue-specific nested
//! types. Transcribed BYTE-FOR-FIELD from the canonical
//! `../githugr/crates/githugr-vm/src/provider.rs`.

use serde::{Deserialize, Serialize};

/// One triage-evidence chip on an issue row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueTriageChipVm {
    pub class: String,
    pub text: String,
}

/// One attachment chip in an issue drawer — a CAS blob scanned at the door.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueAttachmentVm {
    pub label: String,
    pub kind: String,
    pub note: String,
    pub thumb: String,
}

/// One comment in an issue drawer thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCommentVm {
    pub author: String,
    pub when: String,
    pub body: String,
}

/// One issue row. Which tab it renders under is decided by the list it
/// arrives in (`open` / `backlog` / `in_flight` / `closed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRowVm {
    pub number: u32,
    pub title: String,
    pub labels: Vec<(String, Option<String>)>,
    pub triage: Vec<IssueTriageChipVm>,
    pub priority: Option<String>,
    pub author: String,
    pub age: String,
    pub external: bool,
    pub comments: u32,
    pub assignee: Option<String>,
    pub flight_pr: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attachments: Vec<IssueAttachmentVm>,
    #[serde(default)]
    pub triage_evidence: Vec<String>,
    #[serde(default)]
    pub checks_href: Option<String>,
    #[serde(default)]
    pub thread: Vec<IssueCommentVm>,
    #[serde(default)]
    pub resolved_by_pr: Option<u32>,
    #[serde(default)]
    pub title_mono: String,
    #[serde(default)]
    pub triage_pending: bool,
    #[serde(default)]
    pub triage_meta: String,
    #[serde(default)]
    pub triage_evidence_icons: Vec<(String, String)>,
    #[serde(default)]
    pub drawer_note: Option<String>,
    #[serde(default)]
    pub backlog_suggested: bool,
}

/// The issues screen view-model; lists partition by tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuesVm {
    pub repo: String,
    pub doctrine: String,
    pub yours_count: usize,
    pub open: Vec<IssueRowVm>,
    pub backlog: Vec<IssueRowVm>,
    pub in_flight: Vec<IssueRowVm>,
    pub closed: Vec<IssueRowVm>,
    pub policy_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(number: u32, tab: &str) -> IssueRowVm {
        IssueRowVm {
            number,
            title: format!("Issue #{number} [{tab}]"),
            labels: vec![
                ("auth".to_string(), Some("c-auth".to_string())),
                ("bug".to_string(), None),
            ],
            triage: vec![
                IssueTriageChipVm {
                    class: "ok".to_string(),
                    text: "✓ repro".to_string(),
                },
                IssueTriageChipVm {
                    class: "warn".to_string(),
                    text: "≈ PR #129".to_string(),
                },
            ],
            priority: Some("P1".to_string()),
            author: "@ana".to_string(),
            age: "aberta há 3 h".to_string(),
            external: false,
            comments: 2,
            assignee: Some("@gustavo".to_string()),
            flight_pr: Some(125),
            description: Some("Descrição da issue no drawer.".to_string()),
            attachments: vec![
                IssueAttachmentVm {
                    label: "console.txt".to_string(),
                    kind: "file".to_string(),
                    note: "4 KB · 1 token → [REDACTED]".to_string(),
                    thumb: String::new(),
                },
                IssueAttachmentVm {
                    label: "painel.png".to_string(),
                    kind: "image".to_string(),
                    note: "12 KB · cas:7b2e… · ✓ scan".to_string(),
                    thumb: "painel.png".to_string(),
                },
            ],
            triage_evidence: vec![
                "repro confirmado".to_string(),
                "duplicata de #388".to_string(),
                "área: crates/auth".to_string(),
            ],
            checks_href: Some("/r/corelink/checks?ref=388".to_string()),
            thread: vec![IssueCommentVm {
                author: "@gustavo".to_string(),
                when: "há 2 h".to_string(),
                body: "Confirmado localmente.".to_string(),
            }],
            resolved_by_pr: None,
            title_mono: String::new(),
            triage_pending: false,
            triage_meta: "automática · sonnet-4.6 · $0.01 · há 2 h".to_string(),
            triage_evidence_icons: vec![
                ("✓".to_string(), "ok".to_string()),
                ("⊟".to_string(), String::new()),
                ("≈".to_string(), "warn".to_string()),
            ],
            drawer_note: Some("estado EXECUTING → preso na fila de união…".to_string()),
            backlog_suggested: false,
        }
    }

    /// `IssuesVm` fully populated round-trips losslessly.
    #[test]
    fn issues_vm_round_trips() {
        let vm = IssuesVm {
            repo: "humangr/corelink-server".to_string(),
            doctrine: "uma issue é um intent estacionado em PROPOSED.".to_string(),
            yours_count: 3,
            open: vec![make_row(412, "open")],
            backlog: vec![{
                let mut r = make_row(396, "backlog");
                r.backlog_suggested = true;
                r.flight_pr = None;
                r.priority = Some("P2".to_string());
                r
            }],
            in_flight: vec![{
                let mut r = make_row(125, "in_flight");
                r.flight_pr = Some(125);
                r.assignee = Some("opus-4.8".to_string());
                r
            }],
            closed: vec![{
                let mut r = make_row(386, "closed");
                r.resolved_by_pr = Some(119);
                r.age = "fechada há 6 d · resolvida pelo PR #119".to_string();
                r.flight_pr = None;
                r
            }],
            policy_note: "política: triagem auto · teto $0.05/issue".to_string(),
        };

        let json = serde_json::to_string(&vm).expect("IssuesVm serializes");
        let reparsed: IssuesVm = serde_json::from_str(&json).expect("IssuesVm deserializes");
        assert_eq!(vm, reparsed, "IssuesVm round-trip is lossless");
        assert_eq!(reparsed.open[0].labels[1], ("bug".to_string(), None));
        assert_eq!(reparsed.closed[0].resolved_by_pr, Some(119));
    }

    /// A `#[serde(default)]` field absent from JSON parses to its type default.
    #[test]
    fn serde_default_fields_absent_parse_to_default() {
        let minimal = r#"{
            "number": 1, "title": "minimal issue", "labels": [], "triage": [],
            "priority": null, "author": "@bot", "age": "há 1 min",
            "external": false, "comments": 0, "assignee": null, "flight_pr": null
        }"#;
        let row: IssueRowVm = serde_json::from_str(minimal).expect("minimal IssueRowVm parses");
        assert_eq!(row.description, None);
        assert!(row.attachments.is_empty());
        assert_eq!(row.title_mono, "");
        assert!(!row.backlog_suggested);
    }
}
