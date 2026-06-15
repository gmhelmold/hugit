//! `GET /v1/repos/{repo}/prs/{n}/review` → [`ReviewVm`] (None = 404).
//!
//! The `log` is ALREADY chain-verified by the caller. REAL backbone:
//! `verdict.recorded` records (a clean `VerdictObject` per Wave-2 write_verdict)
//! → `verdicts`; `pr.comment` records → timeline. PR identity from `pr.opened` +
//! lifecycle. Every other field is an honest default — the diff/thread/qa/reviewer
//! seams are P2. Every echoed free-text field passes `crate::fmt::scrub`.

use crate::fmt::{humanize_age, scrub};
use hugit_cli::pr::{PR_ABANDONED_KIND, PR_LANDED_KIND, PR_QUEUED_KIND, find_pr_opened};
use hugit_cli::verdict::VERDICT_RECORDED_KIND;
use hugit_contracts::VerdictObject;
use hugit_http_contracts::common::{CampaignChipVm, DiffLineKind, DiffLineVm, HunkVm, VerdictVm};
use hugit_http_contracts::review::{ReviewEventVm, ReviewReplyVm, ReviewThreadVm, ReviewVm};
use hugit_refstore::EventLog;
use serde_json::Value;

const PR_COMMENT_KIND: &str = "pr.comment";

/// Build the review view-model for PR `pr_number`. `None` → 404 (no existence leak).
pub fn build_review(log: &EventLog, repo: &str, pr_number: u32) -> Option<ReviewVm> {
    let pr_id = pr_number.to_string();
    let opened = find_pr_opened(log, &pr_id)?;
    let state_label = pr_state_label(log, &pr_id).to_string();
    let title = if opened.campaign.is_empty() {
        format!("PR #{} — {} intents", opened.pr_id, opened.intent_ids.len())
    } else {
        format!(
            "PR #{} ({}) — {} intents",
            opened.pr_id,
            scrub(&opened.campaign),
            opened.intent_ids.len()
        )
    };
    let intent_count = opened.intent_ids.len();
    let campaign: Option<CampaignChipVm> = if opened.campaign.is_empty() {
        None
    } else {
        let s = scrub(&opened.campaign);
        Some(CampaignChipVm {
            id: s.clone(),
            label: s.clone(),
            color_class: String::new(),
            display_label: s,
        })
    };
    let verdicts = build_verdicts(log, &pr_id);
    let timeline = build_timeline(log, &pr_id);
    let conversation_count = count_comments(log, &pr_id);

    Some(ReviewVm {
        repo: repo.to_string(),
        number: pr_number,
        title,
        state_label,
        author: String::new(),
        source_branch: String::new(),
        target_branch: String::new(),
        intent_count,
        file_count: 0,
        added: 0,
        removed: 0,
        conversation_count,
        qa: vec![],
        qa_note: String::new(),
        timeline,
        thread: stub_thread(),
        verdicts,
        action_labels: vec![
            "✓ Aprovar".to_string(),
            "✎ Pedir mudanças".to_string(),
            "✦ Interrogar…".to_string(),
        ],
        action_note: String::new(),
        reviewers: vec![],
        assignee: String::new(),
        assignee_note: String::new(),
        labels: vec![],
        milestone: None,
        milestone_progress: None,
        campaign,
    })
}

fn pr_state_label(log: &EventLog, pr_id: &str) -> &'static str {
    if has_pr_event(log, PR_LANDED_KIND, pr_id) {
        "landed"
    } else if has_pr_event(log, PR_ABANDONED_KIND, pr_id) {
        "abandoned"
    } else if has_pr_event(log, PR_QUEUED_KIND, pr_id) {
        "queued"
    } else {
        "proposed"
    }
}

fn has_pr_event(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// `pr.comment` matches the PR by `pr_id` as either a string OR a u64 (the write
/// verb serializes it as an integer).
fn comment_matches_pr(v: &Value, pr_id: &str) -> bool {
    let by_str = v.get("pr_id").and_then(Value::as_str) == Some(pr_id);
    let by_int = pr_id
        .parse::<u64>()
        .ok()
        .map(|n| v.get("pr_id").and_then(Value::as_u64) == Some(n))
        .unwrap_or(false);
    by_str || by_int
}

fn count_comments(log: &EventLog, pr_id: &str) -> usize {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_COMMENT_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter(|v| comment_matches_pr(v, pr_id))
        .count()
}

fn build_verdicts(log: &EventLog, pr_id: &str) -> Vec<(VerdictVm, String)> {
    log.records()
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .filter(|vo| vo.intent == pr_id)
        .map(|vo| verdict_pair(&vo))
        .collect()
}

fn verdict_pair(vo: &VerdictObject) -> (VerdictVm, String) {
    use hugit_contracts::verdict_object::Verdict;
    let outcome_str = match vo.verdict {
        Verdict::Approve => "APPROVE",
        Verdict::FixFirst => "FIX-FIRST",
        Verdict::Reject => "REJECT",
    };
    let lens = scrub(&vo.lens);
    let evidence_mono_terms: Vec<String> = vo.claims_checked.iter().map(|c| scrub(c)).collect();
    let evidence_prose = if evidence_mono_terms.is_empty() {
        String::new()
    } else {
        scrub(&evidence_mono_terms.join(" · "))
    };
    let vm = VerdictVm {
        verdict: outcome_str.to_string(),
        reviewer: String::new(),
        summary: format!("{lens}: {outcome_str}"),
        // Invariant (not a fabricated per-verdict signal): every `verdict.recorded`
        // VerdictObject in hugit is produced by the adversarial review panel — that
        // is the forge's review model, and the VerdictObject carries no contrary
        // flag. Matches the contract's canonical examples (common.rs `adversarial`).
        adversarial: true,
        lens,
        evidence_mono_terms,
    };
    (vm, evidence_prose)
}

fn build_timeline(log: &EventLog, pr_id: &str) -> Vec<ReviewEventVm> {
    use hugit_contracts::verdict_object::Verdict;
    let mut events: Vec<(u64, ReviewEventVm)> = Vec::new();
    for r in log.records() {
        if r.kind == VERDICT_RECORDED_KIND {
            if let Ok(vo) = serde_json::from_str::<VerdictObject>(&r.payload) {
                if vo.intent != pr_id {
                    continue;
                }
                let outcome = match vo.verdict {
                    Verdict::Approve => "APPROVE",
                    Verdict::FixFirst => "FIX-FIRST",
                    Verdict::Reject => "REJECT",
                };
                let (class, icon) = match vo.verdict {
                    Verdict::Approve => ("ok", "●"),
                    Verdict::FixFirst => ("warn", "◑"),
                    Verdict::Reject => ("err", "○"),
                };
                let lens = scrub(&vo.lens);
                events.push((
                    r.seq,
                    ReviewEventVm {
                        class: class.to_string(),
                        icon: icon.to_string(),
                        text: format!("veredito {outcome} · lens {lens} · PR #{pr_id}"),
                        age: humanize_age(r.recorded_at),
                        body: None,
                        text_id_terms: vec![pr_id.to_string()],
                        text_bold_terms: vec![outcome.to_string()],
                        text_meta_terms: vec![format!("· lens {lens}")],
                    },
                ));
            }
        } else if r.kind == PR_COMMENT_KIND {
            let Ok(v) = serde_json::from_str::<Value>(&r.payload) else {
                continue;
            };
            if !comment_matches_pr(&v, pr_id) {
                continue;
            }
            let safe_body = scrub(v.get("body").and_then(Value::as_str).unwrap_or(""));
            let anchor = v
                .get("anchor")
                .and_then(Value::as_str)
                .map(scrub)
                .unwrap_or_default();
            let text = if anchor.is_empty() {
                format!("comentário · PR #{pr_id}")
            } else {
                format!("comentário em {anchor} · PR #{pr_id}")
            };
            events.push((
                r.seq,
                ReviewEventVm {
                    class: "comment".to_string(),
                    icon: "💬".to_string(),
                    text,
                    age: humanize_age(r.recorded_at),
                    body: Some(safe_body),
                    text_id_terms: vec![pr_id.to_string()],
                    text_bold_terms: vec![],
                    text_meta_terms: vec![],
                },
            ));
        }
    }
    events.sort_by_key(|(seq, _)| *seq);
    events.into_iter().map(|(_, ev)| ev).collect()
}

fn stub_thread() -> ReviewThreadVm {
    ReviewThreadVm {
        hunk: HunkVm {
            file: String::new(),
            header: String::new(),
            lines: vec![DiffLineVm {
                kind: DiffLineKind::Context,
                text: String::new(),
                ln: String::new(),
            }],
        },
        anchor: String::new(),
        replies: vec![ReviewReplyVm {
            who: String::new(),
            meta: String::new(),
            message: String::new(),
            message_mono_terms: vec![],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_cli::pr::PR_OPENED_KIND;
    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn push(log: &mut EventLog, kind: &str, payload: serde_json::Value, seq: u64) {
        log.append_for_test(kind, vec!["test".to_string()], payload.to_string(), seq);
    }
    fn open_pr(log: &mut EventLog, pr_id: &str, campaign: &str, intents: &[&str]) {
        push(
            log,
            PR_OPENED_KIND,
            serde_json::json!({"author_kind":"orchestrator","campaign":campaign,"intent_ids":intents,"pr_id":pr_id}),
            1,
        );
    }
    fn verdict(intent: &str, lens: &str, v: &str, claims: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"intent":intent,"tree_hash":"","lens":lens,"model":"m","prompt_digest":"0".repeat(64),"verdict":v,"claims_checked":claims,"evidence_refs":[]})
    }

    #[test]
    fn absent_pr_returns_none() {
        assert!(build_review(&EventLog::new(), "hugit", 99).is_none());
    }

    #[test]
    fn empty_pr_is_honest_and_round_trips() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "wave-test", &["i-1"]);
        let vm = build_review(&log, "hugit", 1).expect("present");
        assert!(vm.verdicts.is_empty() && vm.timeline.is_empty());
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<ReviewVm>(&j).unwrap());
    }

    #[test]
    fn verdict_recorded_projects() {
        let mut log = EventLog::new();
        open_pr(&mut log, "7", "c", &["i-1"]);
        push(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict(
                "7",
                "correctness",
                "approve",
                serde_json::json!(["correctness:approve"]),
            ),
            2,
        );
        let vm = build_review(&log, "hugit", 7).expect("present");
        assert_eq!(vm.verdicts.len(), 1);
        assert_eq!(vm.verdicts[0].0.verdict, "APPROVE");
    }

    #[test]
    fn other_pr_verdict_not_included() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "c", &["i"]);
        open_pr(&mut log, "2", "c", &["i"]);
        push(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict("2", "x", "reject", serde_json::json!([])),
            2,
        );
        assert!(build_review(&log, "hugit", 1).unwrap().verdicts.is_empty());
    }

    #[test]
    fn comment_in_timeline() {
        let mut log = EventLog::new();
        open_pr(&mut log, "5", "c", &[]);
        push(
            &mut log,
            PR_COMMENT_KIND,
            serde_json::json!({"body":"lgtm","pr_id":5u64}),
            2,
        );
        let vm = build_review(&log, "hugit", 5).expect("present");
        assert_eq!(vm.conversation_count, 1);
        assert_eq!(vm.timeline[0].class, "comment");
    }

    #[test]
    fn pat_in_claims_is_redacted() {
        let mut log = EventLog::new();
        open_pr(&mut log, "42", "", &["i"]);
        push(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict(
                "42",
                "correctness",
                "approve",
                serde_json::json!([format!("c:{PAT}")]),
            ),
            2,
        );
        let j = serde_json::to_string(&build_review(&log, "hugit", 42).unwrap()).unwrap();
        assert!(!j.contains(PAT) && j.contains("[REDACTED]"));
    }

    #[test]
    fn pat_in_comment_body_is_redacted() {
        let mut log = EventLog::new();
        open_pr(&mut log, "11", "", &[]);
        push(
            &mut log,
            PR_COMMENT_KIND,
            serde_json::json!({"body":PAT,"pr_id":11u64}),
            2,
        );
        let j = serde_json::to_string(&build_review(&log, "hugit", 11).unwrap()).unwrap();
        assert!(!j.contains(PAT) && j.contains("[REDACTED]"));
    }
}
