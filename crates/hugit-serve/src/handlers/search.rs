//! `GET /v1/repos/{repo}/search?q=` → [`SearchVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. This is a text search over
//! the event log — there is NO code/blob index here (that is the P2 CAS source
//! seam), so `code`/`commits` are honest-empty by construction, never faked.
//!
//! REAL backbone (each matched + echoed field is scrubbed at this read boundary):
//! - `prs` — folds `pr.opened` (`hugit_cli::pr` projection): a PR matches when its
//!   corpus (PR number string · campaign key · intent ids) contains `q`
//!   (case-insensitive). `number`/`open`/state-label/`campaign` are REAL; the
//!   `title` is a DERIVED label (`"PR #N — K intents"`) — there is no PR-title
//!   seam on `pr.opened`, so we synthesize one (mirrors `review.rs`), never a fake
//!   free-text title.
//! - `issues` — reuses the `issues.rs` fold (latest `issue.transition` per
//!   `issue_id` → number · current state · first-seen age · priority). A match is
//!   on the issue NUMBER string only (there is no real issue title; `"issue #N"`
//!   is a derived label). `number`/state/age/`priority` are REAL.
//! - `intents` — folds `intent.landed` via `intents_from_log`: a match is over
//!   `intent_id` + charter. `id`/`charter` (first line, scrubbed)/`age` are REAL;
//!   `model`/`pr`/`status` are honest-defaults (`intent.landed` carries none).
//!
//! HONEST-DEFAULT (no local seam — never fabricated): `code`/`code_total` (no blob
//! index), `commits` (no git-commit-message records), `people_count` (no people
//! index; the principal chain is a redacted seam, not a directory), and the
//! `SearchIntentVm` `model`/`pr`/`status` fields. An empty/whitespace-only `q`
//! short-circuits to all-empty result lists (the `repo`/`q`/static notes still
//! populate) — an honest no-op, not a "match everything" dump.

use std::collections::BTreeMap;

use hugit_cli::pr::{
    PR_ABANDONED_KIND, PR_LANDED_KIND, PR_OPENED_KIND, find_pr_opened, find_pr_queued,
};
use hugit_http_contracts::search::{SearchIntentVm, SearchRefVm, SearchVm};
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;
use serde_json::Value;

use crate::fmt::{humanize_age, scrub};

const ISSUE_TRANSITION_KIND: &str = "issue.transition";
const STATE_OPEN: &str = "open";
/// Bound the per-issue fold itself (mirrors `issues.rs`).
const ISSUES_CAP: usize = 500;
/// Per-bucket result cap — the VM IS the page; fail-honest beyond it.
const RESULT_CAP: usize = 200;
/// Static UI caption (a fixed doctrine string, not log-derived, not fabricated data).
const INTENTS_NOTE: &str = "intents carregam o porquê.";
/// Static caption — NOT a timing (there is no real timer; never echo a fake "0,04s").
const INDEX_NOTE: &str = "índice do log de eventos";

/// Case-insensitive substring match over a SCRUB-SAFE corpus. An empty/whitespace
/// `q` never matches (the caller short-circuits, but this stays correct standalone).
fn corpus_matches(corpus: &str, q_lower: &str) -> bool {
    !q_lower.is_empty() && corpus.to_lowercase().contains(q_lower)
}

/// Return the PR's open flag and Portuguese state label.
///
/// Uses `all_pr_queued` (via `find_pr_queued`) so that a PR that was queued
/// then landed is correctly classified as "pousou", not "na fila". Terminal
/// events (`pr.landed`, `pr.abandoned`) are checked first; the queue projection
/// is consulted last and already excludes settled PRs by construction.
fn pr_lifecycle(log: &EventLog, pr_id: &str) -> (bool, &'static str) {
    if has_pr_event(log, PR_LANDED_KIND, pr_id) {
        (false, "pousou")
    } else if has_pr_event(log, PR_ABANDONED_KIND, pr_id) {
        (false, "abandonado")
    } else if find_pr_queued(log, pr_id).is_some() {
        (true, "na fila")
    } else {
        (true, "proposto")
    }
}

fn has_pr_event(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// Fold `pr.opened` → matching PR hits. The match corpus is the PR number string,
/// the campaign key, and the intent ids (all caller-supplied identifiers / keys);
/// the campaign is the only echoed free-text and is scrubbed where interpolated.
///
/// Age provenance: `log.records().get(opened.seq as usize).recorded_at` — the
/// seq-indexed record timestamp, not the iterator record's timestamp. `opened.seq`
/// is the log position of the canonical `pr.opened` event for this PR.
fn search_prs(log: &EventLog, q_lower: &str) -> Vec<SearchRefVm> {
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for record in log.records().iter().filter(|r| r.kind == PR_OPENED_KIND) {
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        let Some(pr_id) = v.get("pr_id").and_then(Value::as_str) else {
            continue;
        };
        if seen.contains(pr_id) {
            continue;
        }
        // Project the canonical opened record (latest-wins) via the shared fold.
        let Some(opened) = find_pr_opened(log, pr_id) else {
            continue;
        };
        let mut corpus = String::new();
        corpus.push_str(&opened.pr_id);
        corpus.push(' ');
        corpus.push_str(&opened.campaign);
        corpus.push(' ');
        corpus.push_str(&opened.intent_ids.join(" "));
        if !corpus_matches(&corpus, q_lower) {
            continue;
        }
        if out.len() >= RESULT_CAP {
            break;
        }
        // Mark seen only once the PR is actually emitted (after the cap break),
        // so a capped-out PR never suppresses a later same-id match.
        seen.insert(pr_id.to_string());
        let (open, label) = pr_lifecycle(log, &opened.pr_id);
        // Age via the seq-indexed record, not the iterator record.
        let age = log
            .records()
            .get(opened.seq as usize)
            .map(|r| humanize_age(r.recorded_at))
            .unwrap_or_default();
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
        out.push(SearchRefVm {
            number: opened.pr_id.parse::<u32>().unwrap_or(0),
            title,
            open,
            meta: format!("{label} · {age}"),
            extra: scrub(&opened.campaign),
        });
    }
    out
}

struct IssueState {
    issue_id: u32,
    current_state: String,
    priority: Option<String>,
    first_recorded_at: u64,
}

/// Latest `issue.transition` per `issue_id` → folded state (mirrors `issues.rs`,
/// same u32 bound + `ISSUES_CAP`). `priority` is scrubbed at the read boundary.
fn fold_transitions(log: &EventLog) -> BTreeMap<u32, IssueState> {
    let mut map: BTreeMap<u32, IssueState> = BTreeMap::new();
    for record in log.records() {
        if record.kind != ISSUE_TRANSITION_KIND {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        let Some(issue_id) = v
            .get("issue_id")
            .and_then(Value::as_u64)
            .filter(|n| *n <= u32::MAX as u64)
            .map(|n| n as u32)
        else {
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
                if map.len() >= ISSUES_CAP {
                    continue;
                }
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

/// Fold issues → matching hits. Match is on the issue NUMBER string only (there is
/// no real issue title); `title` is the derived `"issue #N"` label.
fn search_issues(log: &EventLog, q_lower: &str) -> Vec<SearchRefVm> {
    let mut out = Vec::new();
    for st in fold_transitions(log).values() {
        if out.len() >= RESULT_CAP {
            break;
        }
        let number_str = st.issue_id.to_string();
        if !corpus_matches(&number_str, q_lower) {
            continue;
        }
        let open = st.current_state == STATE_OPEN;
        let label = if open { "aberto" } else { "fechado" };
        let age = humanize_age(st.first_recorded_at);
        out.push(SearchRefVm {
            number: st.issue_id,
            title: format!("issue #{}", st.issue_id),
            open,
            meta: format!("{label} · {age}"),
            extra: st.priority.clone().unwrap_or_default(),
        });
    }
    out
}

/// Fold `intent.landed` → matching hits. Match corpus is `intent_id` + charter;
/// `id`/`charter` (first line, scrubbed)/`age` are REAL, the rest honest-default.
fn search_intents(log: &EventLog, q_lower: &str) -> Vec<SearchIntentVm> {
    let Ok(intent_log) = intents_from_log(log) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for intent in intent_log.intents() {
        if out.len() >= RESULT_CAP {
            break;
        }
        let corpus = format!("{} {}", intent.intent_id, intent.charter);
        if !corpus_matches(&corpus, q_lower) {
            continue;
        }
        out.push(SearchIntentVm {
            id: intent.intent_id.clone(), // structural — not scrubbed
            charter: scrub(intent.charter.lines().next().unwrap_or("")),
            model: String::new(),  // honest-default: no model on intent.landed
            pr: 0,                 // honest-default: no PR back-link on the record
            status: String::new(), // honest-default: intent.landed carries no status
            age: humanize_age(intent.recorded_at),
        });
    }
    out
}

/// Build the search view-model (router calls `build_search(log, repo, q)`).
pub fn build_search(log: &EventLog, repo: &str, q: &str) -> SearchVm {
    let q_echo = scrub(q); // caller free-text round-trips into the VM — scrub it
    let q_lower = q.trim().to_lowercase();

    // Empty/whitespace-only q → honest no-op: repo/q/static notes only.
    let (prs, issues, intents) = if q_lower.is_empty() {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        (
            search_prs(log, &q_lower),
            search_issues(log, &q_lower),
            search_intents(log, &q_lower),
        )
    };

    SearchVm {
        repo: repo.to_string(),
        q: q_echo,
        index_note: INDEX_NOTE.to_string(),
        code: vec![], // honest-default: no blob/code index (P2 CAS seam)
        prs,
        issues,
        intents_note: INTENTS_NOTE.to_string(),
        intents,
        commits: vec![], // honest-default: no git-commit-message records
        people_count: 0, // honest-default: no people/identity index
        code_total: 0,   // honest-default: paired with empty code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn append(log: &mut EventLog, kind: &str, payload: Value, at: u64) {
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            kind,
            vec!["o".into()],
            payload.to_string(),
            at,
        )
        .expect("append");
    }

    fn open_pr(log: &mut EventLog, pr_id: &str, campaign: &str, intents: &[&str], at: u64) {
        append(
            log,
            PR_OPENED_KIND,
            serde_json::json!({
                "author_kind": "orchestrator",
                "campaign": campaign,
                "intent_ids": intents,
                "pr_id": pr_id
            }),
            at,
        );
    }

    fn queue_pr(log: &mut EventLog, pr_id: &str, item_id: &str, order_index: u64, at: u64) {
        use hugit_cli::pr::PR_QUEUED_KIND;
        append(
            log,
            PR_QUEUED_KIND,
            serde_json::json!({
                "pr_id": pr_id,
                "item_id": item_id,
                "order_index": order_index
            }),
            at,
        );
    }

    fn land_pr(log: &mut EventLog, pr_id: &str, at: u64) {
        append(log, PR_LANDED_KIND, serde_json::json!({"pr_id": pr_id}), at);
    }

    fn issue(log: &mut EventLog, id: u32, to: &str, priority: Option<&str>, at: u64) {
        let pv = match priority {
            Some(p) => serde_json::json!({"issue_id": id, "priority": p, "to": to}),
            None => serde_json::json!({"issue_id": id, "to": to}),
        };
        append(log, ISSUE_TRANSITION_KIND, pv, at);
    }

    fn intent(log: &mut EventLog, id: &str, charter: &str, at: u64) {
        append(
            log,
            "intent.landed",
            serde_json::json!({
                "intent_id": id,
                "ref": "refs/heads/main",
                "target": "0".repeat(64),
                "charter": charter
            }),
            at,
        );
    }

    // ── empty-log ────────────────────────────────────────────────────────────

    #[test]
    fn empty_log_honest_defaults() {
        let vm = build_search(&EventLog::new(), "r", "anything");
        assert!(vm.prs.is_empty());
        assert!(vm.issues.is_empty());
        assert!(vm.intents.is_empty());
        // Honest-default seams: never fabricated.
        assert!(vm.code.is_empty());
        assert_eq!(vm.code_total, 0);
        assert!(vm.commits.is_empty());
        assert_eq!(vm.people_count, 0);
        assert_eq!(vm.intents_note, INTENTS_NOTE);
        assert_eq!(vm.index_note, INDEX_NOTE);
        assert_eq!(vm.q, "anything");
        assert_eq!(vm.repo, "r");
    }

    #[test]
    fn empty_query_is_honest_no_op() {
        let mut log = EventLog::new();
        open_pr(&mut log, "7", "wave-x", &["i-1"], 100);
        issue(&mut log, 7, "open", None, 100);
        intent(&mut log, "i-1", "fix the thing", 100);
        // Whitespace-only q matches nothing but still populates repo/q/notes.
        let vm = build_search(&log, "r", "   ");
        assert!(vm.prs.is_empty() && vm.issues.is_empty() && vm.intents.is_empty());
        assert_eq!(vm.q, "   ");
        assert_eq!(vm.intents_note, INTENTS_NOTE);
    }

    // ── populated ────────────────────────────────────────────────────────────

    #[test]
    fn real_projection_pr_match_by_campaign() {
        let mut log = EventLog::new();
        open_pr(&mut log, "128", "auth-wave", &["a31", "a32"], 100);
        open_pr(&mut log, "999", "other", &["z9"], 200);
        let vm = build_search(&log, "r", "auth-wave");
        assert_eq!(vm.prs.len(), 1);
        assert_eq!(vm.prs[0].number, 128);
        assert!(vm.prs[0].open);
        assert!(vm.prs[0].title.contains("PR #128"));
        assert!(vm.prs[0].title.contains("2 intents"));
        assert_eq!(vm.prs[0].extra, "auth-wave");
        assert!(vm.prs[0].meta.contains("proposto"));
    }

    #[test]
    fn real_projection_pr_match_by_intent_id_and_state() {
        let mut log = EventLog::new();
        open_pr(&mut log, "42", "c", &["needle-intent"], 100);
        append(
            &mut log,
            PR_LANDED_KIND,
            serde_json::json!({"pr_id": "42"}),
            200,
        );
        let vm = build_search(&log, "r", "needle-intent");
        assert_eq!(vm.prs.len(), 1);
        assert_eq!(vm.prs[0].number, 42);
        assert!(!vm.prs[0].open);
        assert!(vm.prs[0].meta.contains("pousou"));
    }

    #[test]
    fn real_projection_issue_match_by_number() {
        let mut log = EventLog::new();
        issue(&mut log, 412, "open", Some("P0"), 100);
        issue(&mut log, 7, "closed", None, 100);
        let vm = build_search(&log, "r", "412");
        assert_eq!(vm.issues.len(), 1);
        assert_eq!(vm.issues[0].number, 412);
        assert!(vm.issues[0].open);
        assert_eq!(vm.issues[0].title, "issue #412");
        assert_eq!(vm.issues[0].extra, "P0");
        assert!(vm.issues[0].meta.contains("aberto"));
    }

    #[test]
    fn real_projection_intent_match_by_charter() {
        let mut log = EventLog::new();
        intent(
            &mut log,
            "a31",
            "fix: refresh reusava o iat antigo\nmore detail",
            100,
        );
        intent(&mut log, "b99", "unrelated change", 200);
        let vm = build_search(&log, "r", "refresh");
        assert_eq!(vm.intents.len(), 1);
        assert_eq!(vm.intents[0].id, "a31");
        // Charter is FIRST LINE only.
        assert_eq!(vm.intents[0].charter, "fix: refresh reusava o iat antigo");
        // Honest-defaults.
        assert_eq!(vm.intents[0].model, "");
        assert_eq!(vm.intents[0].pr, 0);
        assert_eq!(vm.intents[0].status, "");
    }

    #[test]
    fn case_insensitive_match() {
        let mut log = EventLog::new();
        open_pr(&mut log, "5", "SkewFix", &[], 100);
        let vm = build_search(&log, "r", "skewfix");
        assert_eq!(vm.prs.len(), 1);
    }

    /// Regression: a PR that was queued and then landed must show "pousou"
    /// (open=false), NOT "na fila". `all_pr_queued` excludes settled PRs, so
    /// `find_pr_queued` returns `None` after `pr.landed` — the old `has_pr_event`
    /// path on `PR_QUEUED_KIND` was divergent from that contract.
    #[test]
    fn queued_then_landed_shows_pousou_not_na_fila() {
        let mut log = EventLog::new();
        open_pr(&mut log, "7", "wave-a", &["i-1"], 1000);
        queue_pr(&mut log, "7", "item-001", 1, 2000);
        land_pr(&mut log, "7", 3000);
        let vm = build_search(&log, "r", "wave-a");
        assert_eq!(vm.prs.len(), 1);
        assert!(!vm.prs[0].open, "landed PR must not be open");
        assert!(
            vm.prs[0].meta.contains("pousou"),
            "expected 'pousou' in meta, got: {}",
            vm.prs[0].meta
        );
    }

    #[test]
    fn queued_pr_shows_na_fila_while_active() {
        let mut log = EventLog::new();
        open_pr(&mut log, "9", "wave-b", &["i-2"], 1000);
        queue_pr(&mut log, "9", "item-002", 1, 2000);
        let vm = build_search(&log, "r", "wave-b");
        assert_eq!(vm.prs.len(), 1);
        assert!(vm.prs[0].open, "queued PR must still be open");
        assert!(
            vm.prs[0].meta.contains("na fila"),
            "expected 'na fila' in meta, got: {}",
            vm.prs[0].meta
        );
    }

    #[test]
    fn redaction_pat_in_campaign_not_leaked() {
        let mut log = EventLog::new();
        open_pr(&mut log, "13", &format!("camp-{PAT}"), &["i-1"], 100);
        // q must match the corpus so the PR surfaces and the campaign is echoed.
        let vm = build_search(&log, "r", "camp-");
        assert_eq!(vm.prs.len(), 1);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT), "PAT must not appear in the VM JSON");
        assert!(j.contains("[REDACTED]"));
    }

    #[test]
    fn redaction_pat_in_charter_not_leaked() {
        let mut log = EventLog::new();
        intent(&mut log, "needle", &format!("secret {PAT} here"), 100);
        let vm = build_search(&log, "r", "needle");
        assert_eq!(vm.intents.len(), 1);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT));
    }

    #[test]
    fn redaction_pat_in_issue_priority_not_leaked() {
        let mut log = EventLog::new();
        issue(&mut log, 77, "open", Some(PAT), 100);
        let vm = build_search(&log, "r", "77");
        assert_eq!(vm.issues.len(), 1);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT));
    }

    #[test]
    fn redaction_pat_in_query_echo_not_leaked() {
        let q = format!("find {PAT}");
        let vm = build_search(&EventLog::new(), "r", &q);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT));
    }

    #[test]
    fn result_cap_bounds_prs() {
        let mut log = EventLog::new();
        for n in 0..(RESULT_CAP as u32 + 25) {
            open_pr(&mut log, &n.to_string(), "needle", &[], 100 + n as u64);
        }
        let vm = build_search(&log, "r", "needle");
        assert_eq!(vm.prs.len(), RESULT_CAP);
    }

    #[test]
    fn vm_round_trips() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "wave-test", &["i-1"], 100);
        issue(&mut log, 1, "open", Some("P1"), 100);
        intent(&mut log, "i-1", "charter line one", 100);
        let vm = build_search(&log, "humangr/hugit", "1");
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<SearchVm>(&j).unwrap());
    }
}
