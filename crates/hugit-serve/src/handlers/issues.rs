//! `GET /v1/repos/{repo}/issues` → [`IssuesVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: the latest
//! `issue.transition` per `issue_id` gives each issue's current state; the FIRST
//! transition's `recorded_at` is the "opened" age; `priority` from the latest
//! transition that carried one. Issues partition into the VM's four tabs by state
//! (`open`/`backlog`/`dispatch`→in_flight/`closed`). The only echoed free-text is
//! `priority` (scrubbed at write + read boundary). All other fields honest-default.

use std::collections::BTreeMap;

use hugit_http_contracts::issues::{IssueRowVm, IssuesVm};
use hugit_refstore::EventLog;
use serde_json::Value;

use crate::fmt::{humanize_age, scrub};

const ISSUE_TRANSITION_KIND: &str = "issue.transition";
const STATE_OPEN: &str = "open";
const STATE_BACKLOG: &str = "backlog";
const STATE_DISPATCH: &str = "dispatch";
const STATE_CLOSED: &str = "closed";
const ISSUES_CAP: usize = 500;

struct IssueState {
    issue_id: u32,
    current_state: String,
    priority: Option<String>,
    first_recorded_at: u64,
}

fn fold_transitions(log: &EventLog) -> BTreeMap<u32, IssueState> {
    let mut map: BTreeMap<u32, IssueState> = BTreeMap::new();
    for record in log.records() {
        if record.kind != ISSUE_TRANSITION_KIND {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        let Some(issue_id) = v.get("issue_id").and_then(Value::as_u64).map(|n| n as u32) else {
            continue;
        };
        let Some(to) = v.get("to").and_then(Value::as_str) else {
            continue;
        };
        let priority: Option<String> = v.get("priority").and_then(Value::as_str).map(scrub);
        let at = record.recorded_at;
        match map.get_mut(&issue_id) {
            Some(existing) => {
                existing.current_state = to.to_string();
                if priority.is_some() {
                    existing.priority = priority;
                }
            }
            None => {
                map.insert(
                    issue_id,
                    IssueState {
                        issue_id,
                        current_state: to.to_string(),
                        priority,
                        first_recorded_at: at,
                    },
                );
            }
        }
    }
    map
}

fn issue_row(st: &IssueState) -> IssueRowVm {
    IssueRowVm {
        number: st.issue_id,
        title: format!("issue #{}", st.issue_id),
        labels: vec![],
        triage: vec![],
        priority: st.priority.clone(),
        author: String::new(),
        age: humanize_age(st.first_recorded_at),
        external: false,
        comments: 0,
        assignee: None,
        flight_pr: None,
        description: None,
        attachments: vec![],
        triage_evidence: vec![],
        checks_href: None,
        thread: vec![],
        resolved_by_pr: None,
        title_mono: String::new(),
        triage_pending: false,
        triage_meta: String::new(),
        triage_evidence_icons: vec![],
        drawer_note: None,
        backlog_suggested: false,
    }
}

/// Build the issues view-model (router calls `build_issues(log, repo)`).
pub fn build_issues(log: &EventLog, repo: &str) -> IssuesVm {
    let transitions = fold_transitions(log);
    let mut open = Vec::new();
    let mut backlog = Vec::new();
    let mut in_flight = Vec::new();
    let mut closed = Vec::new();
    let mut total = 0usize;
    for st in transitions.values() {
        if total >= ISSUES_CAP {
            break;
        }
        let row = issue_row(st);
        match st.current_state.as_str() {
            STATE_OPEN => open.push(row),
            STATE_BACKLOG => backlog.push(row),
            STATE_DISPATCH => in_flight.push(row),
            STATE_CLOSED => closed.push(row),
            _ => continue,
        }
        total += 1;
    }
    IssuesVm {
        repo: repo.to_string(),
        doctrine: "uma issue é um intent estacionado em PROPOSED.".to_string(),
        yours_count: 0,
        open,
        backlog,
        in_flight,
        closed,
        policy_note: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    fn append(log: &mut EventLog, issue_id: u32, to: &str, priority: Option<&str>, at: u64) {
        let pv = match priority {
            Some(p) => serde_json::json!({"issue_id":issue_id,"priority":p,"to":to}),
            None => serde_json::json!({"issue_id":issue_id,"to":to}),
        };
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ISSUE_TRANSITION_KIND,
            vec!["orchestrator:test".to_string()],
            pv.to_string(),
            at,
        )
        .expect("append");
    }

    #[test]
    fn empty_log_empty_vm() {
        let vm = build_issues(&EventLog::new(), "r");
        assert!(vm.open.is_empty() && vm.closed.is_empty());
        assert_eq!(
            vm.doctrine,
            "uma issue é um intent estacionado em PROPOSED."
        );
    }

    #[test]
    fn transitions_partition_by_state() {
        let mut log = EventLog::new();
        append(&mut log, 1, "open", None, 100);
        append(&mut log, 2, "backlog", None, 200);
        append(&mut log, 3, "dispatch", None, 300);
        append(&mut log, 4, "closed", None, 400);
        let vm = build_issues(&log, "r");
        assert_eq!(vm.open[0].number, 1);
        assert_eq!(vm.backlog[0].number, 2);
        assert_eq!(vm.in_flight[0].number, 3);
        assert_eq!(vm.closed[0].number, 4);
    }

    #[test]
    fn latest_state_wins() {
        let mut log = EventLog::new();
        append(&mut log, 10, "open", None, 100);
        append(&mut log, 10, "closed", None, 200);
        let vm = build_issues(&log, "r");
        assert!(vm.open.is_empty());
        assert_eq!(vm.closed[0].number, 10);
    }

    #[test]
    fn priority_sticky_when_omitted_later() {
        let mut log = EventLog::new();
        append(&mut log, 30, "open", Some("P0"), 100);
        append(&mut log, 30, "backlog", None, 200);
        let vm = build_issues(&log, "r");
        assert_eq!(vm.backlog[0].priority.as_deref(), Some("P0"));
    }

    #[test]
    fn priority_secret_redacted() {
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        append(&mut log, 77, "open", Some(pat), 100);
        let vm = build_issues(&log, "r");
        let p = vm.open[0].priority.as_deref().unwrap_or("");
        assert!(p != pat && p.contains("[REDACTED]"));
    }

    #[test]
    fn vm_round_trips() {
        let mut log = EventLog::new();
        append(&mut log, 1, "open", Some("P1"), 100);
        let vm = build_issues(&log, "r");
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<IssuesVm>(&j).unwrap());
    }
}
