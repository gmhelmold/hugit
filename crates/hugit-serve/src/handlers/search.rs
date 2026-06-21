//! `GET /v1/repos/{repo}/search?q=` → [`SearchVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. This is a text search over
//! the event log AND (when a git source is wired) over git blob contents.
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
//! - `code` (F4c interim) — when a git source is wired (`git_source` + `root_tree`
//!   are `Some`), a REAL grep over the git tree's blob contents. Bounded: at most
//!   `CODE_FILE_CAP` files scanned, `CODE_MATCH_CAP` total hits, and blobs larger
//!   than `CODE_BLOB_BYTES_CAP` are skipped (mirrors the existing `RESULT_CAP` /
//!   `MAX_RECORDS_TO_SCAN` guards). Results are secret-scrubbed at the read boundary
//!   (same `scrub` applied to every `code` hit line and path). `code_total` is the
//!   true match count before the cap (may exceed `code.len()`). When no git source
//!   is wired `code`/`code_total` remain honest-empty (never faked).
//!
//! HONEST-DEFAULT (no local seam — never fabricated): `commits` (no
//! git-commit-message records), `people_count` (no people index; the principal
//! chain is a redacted seam, not a directory), and the `SearchIntentVm`
//! `model`/`pr`/`status` fields. An empty/whitespace-only `q` short-circuits to
//! all-empty result lists (the `repo`/`q`/static notes still populate) — an honest
//! no-op, not a "match everything" dump.

use std::collections::BTreeMap;
use std::sync::Arc;

use gix_hash::ObjectId;
use hugit_cli::pr::{
    PR_ABANDONED_KIND, PR_LANDED_KIND, PR_OPENED_KIND, find_pr_opened, find_pr_queued,
};
use hugit_http_contracts::search::{SearchCodeVm, SearchIntentVm, SearchRefVm, SearchVm};
use hugit_proto::{ObjectKind, ObjectSource};
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
/// Maximum log records scanned per search query. Without this short-circuit a
/// single query against a huge log can stall the single-threaded server for
/// seconds (DoS fix). This is a scan-side guard; the RESULT_CAP is the
/// output-side guard — both must hold.
const MAX_RECORDS_TO_SCAN: usize = 50_000;
/// Static UI caption (a fixed doctrine string, not log-derived, not fabricated data).
const INTENTS_NOTE: &str = "intents carregam o porquê.";
/// Static caption — NOT a timing (there is no real timer; never echo a fake "0,04s").
const INDEX_NOTE: &str = "índice do log de eventos";

// ── F4c interim code search bounds ───────────────────────────────────────────

/// Maximum number of tree files visited per code search (depth-first, BFS order).
/// Mirrors the existing `MAX_RECORDS_TO_SCAN` discipline: a single search must
/// never stall the single-threaded server. 2000 files bounds the CAS budget.
const CODE_FILE_CAP: usize = 2_000;
/// Maximum number of line hits emitted in `code` before the result cap kicks in.
/// `code_total` continues counting (the caller can page). Mirrors `RESULT_CAP`.
const CODE_MATCH_CAP: usize = 200;
/// Maximum blob size (bytes) that is read + searched inline. A blob larger than
/// 50 KiB is skipped (counted as 0 hits for that file). This prevents a single
/// huge generated file from dominating the search budget.
const CODE_BLOB_BYTES_CAP: usize = 50 * 1024;
/// Maximum number of matching lines PER FILE emitted into `code`. Keeps one huge
/// file from consuming the whole result budget at the expense of other files.
const CODE_LINES_PER_FILE_CAP: usize = 10;

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
    for record in log
        .records()
        .iter()
        .take(MAX_RECORDS_TO_SCAN)
        .filter(|r| r.kind == PR_OPENED_KIND)
    {
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
    for record in log.records().iter().take(MAX_RECORDS_TO_SCAN) {
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
    for intent in intent_log.intents().iter().take(MAX_RECORDS_TO_SCAN) {
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

// ── F4c interim code search (git tree grep) ──────────────────────────────────

/// One stack frame for the iterative DFS tree walk used by `search_code`.
struct TreeFrame {
    /// The oid of this tree object.
    oid: ObjectId,
    /// The repo-relative path prefix for this directory (e.g. `"src/handlers"`).
    /// Empty string for the root.
    prefix: String,
}

/// Grep the git tree's blob contents for `q_lower` (case-insensitive). Returns
/// `(code_hits, code_total)` where `code_total` is the TRUE match count across all
/// blobs (may exceed `CODE_MATCH_CAP`; `code_hits` is the capped subset).
///
/// The walk is depth-first iterative (no recursion stack overflow on deep trees).
/// Files/bytes/matches are bounded by `CODE_FILE_CAP` / `CODE_BLOB_BYTES_CAP` /
/// `CODE_MATCH_CAP`. Results are secret-scrubbed at the read boundary.
fn search_code(
    src: &dyn ObjectSource,
    root_tree: &ObjectId,
    q_lower: &str,
) -> (Vec<SearchCodeVm>, usize) {
    let mut hits: Vec<SearchCodeVm> = Vec::new();
    let mut code_total: usize = 0;
    let mut files_visited: usize = 0;

    // DFS stack: start at the root tree.
    let mut stack: Vec<TreeFrame> = vec![TreeFrame {
        oid: *root_tree,
        prefix: String::new(),
    }];

    while let Some(frame) = stack.pop() {
        if files_visited >= CODE_FILE_CAP {
            break;
        }
        // Fetch the tree object; skip on error (fail-closed, not abort).
        let tree_obj = match src.get(&frame.oid) {
            Ok(Some(o)) if o.kind == ObjectKind::Tree => o,
            _ => continue,
        };
        let entries = match gix_object::TreeRefIter::from_bytes(&tree_obj.data).entries() {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries {
            if files_visited >= CODE_FILE_CAP {
                break;
            }
            // Skip symlinks and gitlinks (mirrors list_tree_at_dir).
            if entry.mode.is_link() || entry.mode.is_commit() {
                continue;
            }
            let name = String::from_utf8_lossy(entry.filename);
            let path = if frame.prefix.is_empty() {
                name.into_owned()
            } else {
                format!("{}/{name}", frame.prefix)
            };
            if entry.mode.is_tree() {
                // Push subtree onto the DFS stack.
                stack.push(TreeFrame {
                    oid: entry.oid.to_owned(),
                    prefix: path,
                });
                continue;
            }
            // It's a blob: fetch + search.
            files_visited += 1;
            let blob_obj = match src.get(&entry.oid.to_owned()) {
                Ok(Some(o)) if o.kind == ObjectKind::Blob => o,
                _ => continue,
            };
            if blob_obj.data.len() > CODE_BLOB_BYTES_CAP {
                continue; // too large — skip this file
            }
            // Decode lossily (binary blobs render as mojibake but don't panic).
            let text = String::from_utf8_lossy(&blob_obj.data);
            let path_scrubbed = scrub(&path);
            let mut file_hits: Vec<(u32, String)> = Vec::new();
            for (i, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(q_lower) {
                    code_total += 1;
                    if hits.len() < CODE_MATCH_CAP && file_hits.len() < CODE_LINES_PER_FILE_CAP {
                        // 1-based line number; scrub the line text at the read boundary.
                        file_hits.push(((i as u32) + 1, scrub(line)));
                    }
                }
            }
            if !file_hits.is_empty() {
                hits.push(SearchCodeVm {
                    path: path_scrubbed,
                    lines: file_hits,
                    lines_tokens: vec![], // no syntax-highlight this wave
                });
            }
        }
    }
    (hits, code_total)
}

/// Build the search view-model.
///
/// When a git source (`git_source` + `root_tree`) is wired, a REAL interim code
/// search (F4c) greps the git tree's blob contents. Without a git source,
/// `code`/`code_total` are honest-empty (never faked).
pub fn build_search(
    log: &EventLog,
    repo: &str,
    q: &str,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
) -> SearchVm {
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

    // F4c interim code search: real git-tree grep when the content seam is wired.
    // No git source → honest-empty (not faked).
    let (code, code_total) = if !q_lower.is_empty()
        && let Some(src) = git_source
        && let Some(root) = root_tree
    {
        search_code(src.as_ref(), root, &q_lower)
    } else {
        (vec![], 0)
    };

    SearchVm {
        repo: repo.to_string(),
        q: q_echo,
        index_note: INDEX_NOTE.to_string(),
        code,
        prs,
        issues,
        intents_note: INTENTS_NOTE.to_string(),
        intents,
        commits: vec![], // honest-default: no git-commit-message records
        people_count: 0, // honest-default: no people/identity index
        code_total,      // REAL when git source wired; 0 otherwise (honest-empty)
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
        let vm = build_search(&EventLog::new(), "r", "anything", None, None);
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
        let vm = build_search(&log, "r", "   ", None, None);
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
        let vm = build_search(&log, "r", "auth-wave", None, None);
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
        let vm = build_search(&log, "r", "needle-intent", None, None);
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
        let vm = build_search(&log, "r", "412", None, None);
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
        let vm = build_search(&log, "r", "refresh", None, None);
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
        let vm = build_search(&log, "r", "skewfix", None, None);
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
        let vm = build_search(&log, "r", "wave-a", None, None);
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
        let vm = build_search(&log, "r", "wave-b", None, None);
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
        let vm = build_search(&log, "r", "camp-", None, None);
        assert_eq!(vm.prs.len(), 1);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT), "PAT must not appear in the VM JSON");
        assert!(j.contains("[REDACTED]"));
    }

    #[test]
    fn redaction_pat_in_charter_not_leaked() {
        let mut log = EventLog::new();
        intent(&mut log, "needle", &format!("secret {PAT} here"), 100);
        let vm = build_search(&log, "r", "needle", None, None);
        assert_eq!(vm.intents.len(), 1);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT));
    }

    #[test]
    fn redaction_pat_in_issue_priority_not_leaked() {
        let mut log = EventLog::new();
        issue(&mut log, 77, "open", Some(PAT), 100);
        let vm = build_search(&log, "r", "77", None, None);
        assert_eq!(vm.issues.len(), 1);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT));
    }

    #[test]
    fn redaction_pat_in_query_echo_not_leaked() {
        let q = format!("find {PAT}");
        let vm = build_search(&EventLog::new(), "r", &q, None, None);
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(PAT));
    }

    #[test]
    fn result_cap_bounds_prs() {
        let mut log = EventLog::new();
        for n in 0..(RESULT_CAP as u32 + 25) {
            open_pr(&mut log, &n.to_string(), "needle", &[], 100 + n as u64);
        }
        let vm = build_search(&log, "r", "needle", None, None);
        assert_eq!(vm.prs.len(), RESULT_CAP);
    }

    #[test]
    fn vm_round_trips() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "wave-test", &["i-1"], 100);
        issue(&mut log, 1, "open", Some("P1"), 100);
        intent(&mut log, "i-1", "charter line one", 100);
        let vm = build_search(&log, "humangr/hugit", "1", None, None);
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<SearchVm>(&j).unwrap());
    }

    // ── F4c: interim code search (git tree grep) ─────────────────────────────

    /// Build a minimal CAS with a single file so code search can find a real hit.
    fn make_git_src_with_file(
        path: &str,
        content: &[u8],
    ) -> (Arc<dyn hugit_proto::ObjectSource + Send + Sync>, ObjectId) {
        use hugit_proto::{CasObjectSource, ObjectKind};

        let mut cas = CasObjectSource::new();
        let blob_oid = cas.insert_raw(ObjectKind::Blob, content.to_vec());

        // Build the tree bytes: `100644 <name>\0<20-byte-oid>`.
        let mut tree_bytes = Vec::new();
        tree_bytes.extend_from_slice(b"100644 ");
        // Only the leaf name (no path separators in a tree entry).
        let leaf = path.rsplit('/').next().unwrap_or(path);
        tree_bytes.extend_from_slice(leaf.as_bytes());
        tree_bytes.push(0);
        tree_bytes.extend_from_slice(blob_oid.as_bytes());
        let root_oid = cas.insert_raw(ObjectKind::Tree, tree_bytes);

        (Arc::new(cas), root_oid)
    }

    #[test]
    fn code_search_finds_real_hit_when_git_source_wired() {
        // F4c: a wired git source + a query that matches a file's content →
        // real code hits in the VM (NOT honest-empty).
        let content = b"fn frobnicate(x: u32) -> u32 { x + 1 }\n";
        let (src, root) = make_git_src_with_file("lib.rs", content);
        let vm = build_search(&EventLog::new(), "r", "frobnicate", Some(&src), Some(&root));
        assert!(
            !vm.code.is_empty(),
            "code must be non-empty when git source is wired and query matches"
        );
        assert_eq!(vm.code[0].path, "lib.rs");
        assert_eq!(vm.code[0].lines.len(), 1);
        assert_eq!(vm.code[0].lines[0].0, 1); // 1-based line number
        assert!(
            vm.code[0].lines[0].1.contains("frobnicate"),
            "line text: {}",
            vm.code[0].lines[0].1
        );
        assert!(vm.code_total >= 1, "code_total must be at least 1");
    }

    #[test]
    fn code_search_is_case_insensitive() {
        let content = b"const MAX_FROB: usize = 42;\n";
        let (src, root) = make_git_src_with_file("cfg.rs", content);
        let vm = build_search(&EventLog::new(), "r", "max_frob", Some(&src), Some(&root));
        assert!(
            !vm.code.is_empty(),
            "case-insensitive match must return a hit"
        );
    }

    #[test]
    fn code_search_empty_when_no_git_source() {
        // Without a git source, code / code_total are honest-empty (not faked).
        let vm = build_search(&EventLog::new(), "r", "frobnicate", None, None);
        assert!(vm.code.is_empty(), "no git source → code must be empty");
        assert_eq!(vm.code_total, 0);
    }

    #[test]
    fn code_search_secret_in_file_is_scrubbed() {
        // A secret embedded in a file must be scrubbed at the read boundary —
        // the raw secret must NOT appear in the code hit lines.
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let content = format!("const KEY: &str = \"{secret}\";\n");
        let (src, root) = make_git_src_with_file("secret.rs", content.as_bytes());
        // q matches the file (via the KEY word before the secret)
        let vm = build_search(&EventLog::new(), "r", "key", Some(&src), Some(&root));
        let j = serde_json::to_string(&vm).unwrap();
        assert!(
            !j.contains(secret),
            "secret must be scrubbed from code hit lines: {j}"
        );
        assert!(
            j.contains("[REDACTED]"),
            "REDACTED sentinel must appear in place of secret: {j}"
        );
    }
}
