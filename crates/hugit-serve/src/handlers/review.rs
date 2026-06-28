//! `GET /v1/repos/{repo}/prs/{n}/review` → [`ReviewVm`] (None = 404).
//!
//! The `log` is ALREADY chain-verified by the caller. REAL backbone:
//! `verdict.recorded` records (a clean `VerdictObject` per Wave-2 write_verdict)
//! → `verdicts`; `pr.comment` records → timeline. PR identity from `pr.opened` +
//! lifecycle. Every other field is an honest default — the diff/thread/qa/reviewer
//! seams are P2. Every echoed free-text field passes `crate::fmt::scrub`.

use std::sync::Arc;

use crate::fmt::{humanize_age, scrub};
use hugit_cli::pr::{PR_ABANDONED_KIND, PR_LANDED_KIND, PR_QUEUED_KIND, find_pr_opened};
use hugit_cli::verdict::VERDICT_RECORDED_KIND;
use hugit_contracts::VerdictObject;
use hugit_http_contracts::common::{
    CampaignChipVm, DiffLineKind, DiffLineVm, FileRowVm, HunkVm, VerdictVm, decision_of,
};
use hugit_http_contracts::review::{ReviewEventVm, ReviewReplyVm, ReviewThreadVm, ReviewVm};
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;
use serde_json::Value;

use super::diff::diff_vm;

const PR_COMMENT_KIND: &str = "pr.comment";

/// The git object source threaded in for the PR diff seam. `None` =
/// no `HUGIT_SERVE_GIT_DIR` → an honest `file_count: 0` diffstat.
type GitSrc<'a> = Option<&'a Arc<dyn hugit_proto::ObjectSource + Send + Sync>>;

/// Build the review view-model for PR `pr_number`. `None` → 404 (no existence leak).
///
/// `src` is the deploy-gated git object source; when present the `file_count` /
/// `added` / `removed` diffstat is REAL (the union of the PR's intents' commits
/// vs. their parents). `None` → the honest `0` diffstat.
pub fn build_review(
    log: &EventLog,
    repo: &str,
    pr_number: u32,
    src: GitSrc<'_>,
) -> Option<ReviewVm> {
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

    // REAL (review-legibility ①): the PR diffstat — the union of its intents'
    // commits vs. their parents, walked over the git source. Honest `(0,0,0)`
    // when there is no git source / no resolvable commit (matches `file_count: 0`).
    let (file_count, added, removed) = pr_diffstat(log, &opened.intent_ids, src);

    Some(ReviewVm {
        repo: repo.to_string(),
        number: pr_number,
        title,
        state_label,
        author: String::new(),
        source_branch: String::new(),
        target_branch: String::new(),
        intent_count,
        file_count,
        added,
        removed,
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

/// Max intents whose diffs are summed into one PR diffstat. Each intent diff is a
/// CAS tree-walk (`tree_diff`, itself wall-clock-bounded); this caps how many of
/// them ONE request runs so a PR with pathologically many intents can't multiply
/// the per-diff budget into a single-threaded-engine stall (the diffstat is
/// informational — a partial sum is acceptable, and `tree_diff`'s own budget is
/// the load-bearing latency guard).
const MAX_REVIEW_DIFF_INTENTS: usize = 64;

/// The PR's aggregate diffstat: `(file_count, added, removed)` over the union of
/// its intents' commits vs. their parents. Honest `(0, 0, 0)` when there is no
/// git source or no intent commit resolves. A path touched by ≥2 intents is
/// counted once (union by path) — `file_count` is the distinct changed-file set.
fn pr_diffstat(log: &EventLog, intent_ids: &[String], src: GitSrc<'_>) -> (usize, u32, u32) {
    let Some(src) = src else {
        return (0, 0, 0);
    };
    let Ok(intent_log) = intents_from_log(log) else {
        return (0, 0, 0);
    };
    // Union the per-intent file rows by path; sum added/removed per path.
    let mut by_path: std::collections::BTreeMap<String, (u32, u32)> =
        std::collections::BTreeMap::new();
    for id in intent_ids.iter().take(MAX_REVIEW_DIFF_INTENTS) {
        let Some(intent) = intent_log.by_id(id) else {
            continue;
        };
        let diff = intent_commit_diff(src, &intent.target);
        for FileRowVm {
            path,
            added,
            removed,
        } in diff.files
        {
            let e = by_path.entry(path).or_insert((0, 0));
            e.0 += added;
            e.1 += removed;
        }
    }
    let file_count = by_path.len();
    let added = by_path.values().map(|(a, _)| *a).sum();
    let removed = by_path.values().map(|(_, r)| *r).sum();
    (file_count, added, removed)
}

/// The diff for one intent commit vs. its first parent (the review.rs-local twin
/// of `intent_detail::intent_diff`; kept here to avoid a cross-handler dep).
fn intent_commit_diff(
    src: &Arc<dyn hugit_proto::ObjectSource + Send + Sync>,
    target_hex: &str,
) -> hugit_http_contracts::common::DiffVm {
    use super::diff::empty_diff;
    let Ok(commit) = gix_hash::ObjectId::from_hex(target_hex.as_bytes()) else {
        return empty_diff();
    };
    let new_tree = match hugit_proto::commit_root_tree(src.as_ref(), &commit) {
        Ok(Some(t)) => t,
        _ => return empty_diff(),
    };
    let parent_tree = src
        .get(&commit)
        .ok()
        .flatten()
        .filter(|o| o.kind == hugit_proto::ObjectKind::Commit)
        .and_then(|o| {
            gix_object::CommitRefIter::from_bytes(&o.data)
                .parent_ids()
                .next()
        })
        .and_then(|p| {
            hugit_proto::commit_root_tree(src.as_ref(), &p)
                .ok()
                .flatten()
        });
    let Some(parent_tree) = parent_tree else {
        return empty_diff();
    };
    diff_vm(Some(src), Some(&parent_tree), Some(&new_tree))
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
    // REAL (review-legibility ②): the reviewer IS the model that produced the
    // verdict (the VerdictObject's `model` field). Scrubbed — a model id is an
    // identifier but a smuggled secret-shaped value must not echo raw.
    let reviewer = scrub(&vo.model);
    let evidence_mono_terms: Vec<String> = vo.claims_checked.iter().map(|c| scrub(c)).collect();
    let evidence_prose = if evidence_mono_terms.is_empty() {
        String::new()
    } else {
        scrub(&evidence_mono_terms.join(" · "))
    };
    // REAL (review-legibility ②): a 1-line summary derived from the claims
    // checked — "<lens> · <outcome> · N claim(s) checked". No fabricated prose;
    // every term is a real verdict field.
    let summary = if evidence_mono_terms.is_empty() {
        format!("{lens} · {outcome_str}")
    } else {
        format!(
            "{lens} · {outcome_str} · {} claim(s) checked",
            evidence_mono_terms.len()
        )
    };
    let vm = VerdictVm {
        verdict: outcome_str.to_string(),
        reviewer,
        summary,
        // Invariant (not a fabricated per-verdict signal): every `verdict.recorded`
        // VerdictObject in hugit is produced by the adversarial review panel — that
        // is the forge's review model, and the VerdictObject carries no contrary
        // flag. Matches the contract's canonical examples (common.rs `adversarial`).
        adversarial: true,
        lens,
        evidence_mono_terms,
        decision: decision_of(outcome_str), // REAL — structured, same outcome source
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
        assert!(build_review(&EventLog::new(), "hugit", 99, None).is_none());
    }

    #[test]
    fn empty_pr_is_honest_and_round_trips() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "wave-test", &["i-1"]);
        let vm = build_review(&log, "hugit", 1, None).expect("present");
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
        let vm = build_review(&log, "hugit", 7, None).expect("present");
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
        assert!(
            build_review(&log, "hugit", 1, None)
                .unwrap()
                .verdicts
                .is_empty()
        );
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
        let vm = build_review(&log, "hugit", 5, None).expect("present");
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
        let j = serde_json::to_string(&build_review(&log, "hugit", 42, None).unwrap()).unwrap();
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
        let j = serde_json::to_string(&build_review(&log, "hugit", 11, None).unwrap()).unwrap();
        assert!(!j.contains(PAT) && j.contains("[REDACTED]"));
    }

    // ── review-legibility ② : reviewer == the verdict model ──────────────────
    #[test]
    fn reviewer_is_the_verdict_model() {
        let mut log = EventLog::new();
        open_pr(&mut log, "7", "c", &["i-1"]);
        // A verdict whose model is "opus-4.8".
        push(
            &mut log,
            VERDICT_RECORDED_KIND,
            serde_json::json!({"intent":"7","tree_hash":"","lens":"correctness","model":"opus-4.8","prompt_digest":"0".repeat(64),"verdict":"approve","claims_checked":["iat ok","exp ok"],"evidence_refs":[]}),
            2,
        );
        let vm = build_review(&log, "hugit", 7, None).expect("present");
        let (v, _prose) = &vm.verdicts[0];
        assert_eq!(v.reviewer, "opus-4.8", "reviewer is the verdict model");
        assert!(v.adversarial, "panel-sourced verdict");
        assert!(v.summary.contains("correctness") && v.summary.contains("2 claim"));
    }

    // ── review-legibility ① : a populated git source projects a REAL diffstat ─
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind, ObjectSource};
    use hugit_refstore::intent::INTENT_LANDED_KIND;
    use std::sync::Arc;

    fn blob(src: &mut CasObjectSource, body: &str) -> gix_hash::ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(
        src: &mut CasObjectSource,
        mut entries: Vec<(&str, &str, gix_hash::ObjectId)>,
    ) -> gix_hash::ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }
    fn commit(
        src: &mut CasObjectSource,
        tree_oid: gix_hash::ObjectId,
        parent: Option<gix_hash::ObjectId>,
    ) -> gix_hash::ObjectId {
        let parent_line = parent.map(|p| format!("parent {p}\n")).unwrap_or_default();
        let body = format!(
            "tree {tree_oid}\n{parent_line}author a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nm\n"
        );
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    #[test]
    fn populated_git_source_projects_real_diffstat() {
        let mut src = CasObjectSource::new();
        // parent commit: f.txt = "a\nb\n"; child commit: f.txt = "a\nb\nc\n".
        let old = blob(&mut src, "a\nb\n");
        let new = blob(&mut src, "a\nb\nc\n");
        let parent_tree = tree(&mut src, vec![("100644", "f.txt", old)]);
        let child_tree = tree(&mut src, vec![("100644", "f.txt", new)]);
        let parent_commit = commit(&mut src, parent_tree, None);
        let child_commit = commit(&mut src, child_tree, Some(parent_commit));
        let target = child_commit.to_string();

        let mut log = EventLog::new();
        open_pr(&mut log, "9", "c", &["i-1"]);
        push(
            &mut log,
            INTENT_LANDED_KIND,
            serde_json::json!({"intent_id":"i-1","ref":"refs/heads/x","target":target,"charter":"add c"}),
            2,
        );
        let arc: Arc<dyn ObjectSource + Send + Sync> = Arc::new(src);
        let vm = build_review(&log, "hugit", 9, Some(&arc)).expect("present");
        assert_eq!(vm.file_count, 1, "one changed file");
        assert_eq!((vm.added, vm.removed), (1, 0), "+1 line, -0");
    }

    #[test]
    fn no_git_source_is_honest_zero_diffstat() {
        let mut log = EventLog::new();
        open_pr(&mut log, "9", "c", &["i-1"]);
        push(
            &mut log,
            INTENT_LANDED_KIND,
            serde_json::json!({"intent_id":"i-1","ref":"r","target":"0".repeat(40),"charter":"x"}),
            2,
        );
        let vm = build_review(&log, "hugit", 9, None).expect("present");
        assert_eq!((vm.file_count, vm.added, vm.removed), (0, 0, 0));
    }
}
