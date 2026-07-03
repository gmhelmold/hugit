//! `write_pr_create` — the pure `prs` create verb (open a PR from a branch).
//!
//! The GitHub-faithful "open a PR on a branch's diff" verb: given a `head` and a
//! `base` ref-ish, resolve each to its tip commit oid against the repo's LIVE refs
//! (the same resolution the compare diff uses), assign the next PR number, and emit
//! a single `pr.opened` event that PINS both tip SHAs + the branch names + the
//! title/body. The read side ([`crate::handlers::build_pr_detail`]) reads those
//! pinned SHAs back and renders a REAL head-vs-base numstat — so the PR is never an
//! empty/stub row. Returns `{pr_number, branch: head}`.
//!
//! Grounding (the PR model this transcribes, never re-designs):
//! - `pr.opened` (kind `hugit_cli::pr::PR_OPENED_KIND`) IS the PR-create event;
//!   `find_pr_opened`/`parse_opened` project it. The REQUIRED payload keys are
//!   `pr_id`, `campaign`, `author_kind`, `intent_ids` (so this verb emits all of
//!   them); the head/base SHAs + branch names + title/body are ADDITIVE fields the
//!   pr-detail read consults. A branch PR bundles NO landed intents (`intent_ids:
//!   []`), so its diff comes from the pinned head/base SHAs, not an intent commit.
//! - The PR NUMBER is `count(pr.opened) + 1` (the log-local ordinal — identical to
//!   `write_dispatch`/`write_edit_propose`).
//!
//! Authorization: routed through the D14-guarded `append_authorized` under the
//! chain-derived class ([`crate::writes::asserted_class`]) — a write is NEVER
//! anonymous (the door already cleared the ownership gate; an unclassifiable/empty
//! principal fails closed to 404). Free text (title/body/branch names) is SCRUBBED
//! at this write boundary before it reaches the log.

use std::collections::BTreeMap;

use hugit_cli::pr::PR_OPENED_KIND;
use hugit_contracts::event_record::EventRecord;
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::PrCreateReq;
use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::error::EngineErr;
use crate::fmt::scrub;
use crate::handlers::diff::resolve_refish;

/// The author kind recorded for a web-opened branch PR (mirrors the other web
/// verbs — `dispatch`/`edit-propose` — which record the integration authority as
/// the `orchestrator` author; the real per-user binding is the P2 identity seam).
const PR_CREATE_AUTHOR_KIND: &str = "orchestrator";

/// The next PR ordinal for the log — `count(pr.opened) + 1` (the SAME assignment
/// `write_dispatch`/`write_edit_propose` use, so numbers never collide across the
/// PR-creating verbs).
fn next_pr_id(log: &EventLog) -> u64 {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_OPENED_KIND)
        .count() as u64
        + 1
}

/// `POST /v1/repos/{repo}/prs` — open a PR from `head` against `base`.
///
/// `refs` is the repo's LIVE `ref name → tip-oid` snapshot (the advertise
/// projection); `head`/`base` are resolved against it (then a raw-oid fallback).
///
/// # Errors
/// - `400 INVALID_REQUEST` — `head`, `base`, or `title` blank, or `head == base`.
/// - `404 NOT_FOUND` — `head` or `base` does not resolve to a tip on this repo
///   (an unknown branch is indistinguishable from a private/absent one — no
///   existence oracle), or the principal is unclassifiable (fail-closed).
/// - `503 ENGINE_UNAVAILABLE` — `append_authorized` denied (mapped fail-honest).
pub fn write_pr_create(
    log: &mut EventLog,
    repo: &str,
    req: &PrCreateReq,
    refs: &BTreeMap<String, String>,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    let head = req.head.trim();
    let base = req.base.trim();
    if head.is_empty() || base.is_empty() {
        return Err(EngineErr::invalid_request("head/base vazio"));
    }
    if req.title.trim().is_empty() {
        return Err(EngineErr::invalid_request("title vazio"));
    }
    if head == base {
        return Err(EngineErr::invalid_request(
            "head e base são a mesma ref — nada a comparar",
        ));
    }

    // Resolve BOTH tips against the live refs (same rules as the compare diff). An
    // unresolvable side is a uniform 404 — no existence oracle (an unknown branch
    // looks exactly like a private/absent repo). Assert BEFORE any log append.
    let head_oid = resolve_refish(refs, head).ok_or_else(EngineErr::not_found)?;
    let base_oid = resolve_refish(refs, base).ok_or_else(EngineErr::not_found)?;

    // The D14 author class is derived from the AUTHENTICATED caller (fail-closed);
    // asserted here so an unclassifiable/anonymous chain is refused before any write.
    let class = crate::writes::asserted_class(&principal_chain)?;

    let pr_id = next_pr_id(log);
    let pr_id_str = pr_id.to_string();
    // Scrub every free-text field at the write boundary. Branch names are free text
    // too (a secret-shaped branch must not persist verbatim); the pinned SHAs are
    // structural 40-hex oids the resolver already validated, so they are stored raw.
    let head_branch = scrub(head);
    let base_branch = scrub(base);
    let title = scrub(req.title.trim());
    let body = req.body.as_deref().map(scrub).unwrap_or_default();
    let author = principal_chain.last().cloned().unwrap_or_default();

    let pr_payload_value = json!({
        "author": author,
        "author_kind": PR_CREATE_AUTHOR_KIND,
        "base": base_branch,
        "base_sha": base_oid.to_string(),
        "body": body,
        "campaign": "",
        "head": head_branch,
        "head_sha": head_oid.to_string(),
        "intent_ids": [],
        "pr_id": pr_id_str,
        "principal": null,
        "run_id": null,
        "title": title,
    });
    let pr_payload = hugit_refstore::canonical_json(&pr_payload_value.to_string())
        .unwrap_or_else(|| pr_payload_value.to_string());
    let record: EventRecord = log
        .append_authorized(
            class,
            Endpoint::Land,
            PR_OPENED_KIND,
            principal_chain,
            pr_payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("pr.opened append denied: {}", d.reason.code()))
        })?;

    Ok(Accepted {
        seq: record.seq,
        note: "PR aberto".to_string(),
        extra: None,
        queue_pos: None,
        pr_number: Some(pr_id),
        branch: Some(req.head.trim().to_string()),
        state: None,
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::build_pr_detail;
    use gix_hash::ObjectId;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind, ObjectSource};
    use std::sync::Arc;

    fn chain() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }
    fn req(head: &str, base: &str, title: &str, body: Option<&str>) -> PrCreateReq {
        PrCreateReq {
            head: head.to_string(),
            base: base.to_string(),
            title: title.to_string(),
            body: body.map(str::to_string),
        }
    }

    fn blob(src: &mut CasObjectSource, body: &str) -> ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(src: &mut CasObjectSource, mut entries: Vec<(&str, &str, ObjectId)>) -> ObjectId {
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
    fn commit(src: &mut CasObjectSource, tree_oid: ObjectId, parent: Option<ObjectId>) -> ObjectId {
        let parent_line = parent.map(|p| format!("parent {p}\n")).unwrap_or_default();
        let body = format!(
            "tree {tree_oid}\n{parent_line}author a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nm\n"
        );
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    /// A source with `main` (one file) and `feat/x` (that file + one added line),
    /// plus the `ref → tip` map. The head→base diff is a real one-file, +1 numstat.
    fn repo_with_branches() -> (
        Arc<dyn ObjectSource + Send + Sync>,
        BTreeMap<String, String>,
    ) {
        let mut src = CasObjectSource::new();
        let old = blob(&mut src, "a\n");
        let new = blob(&mut src, "a\nb\n");
        let base_tree = tree(&mut src, vec![("100644", "f.txt", old)]);
        let head_tree = tree(&mut src, vec![("100644", "f.txt", new)]);
        let base_commit = commit(&mut src, base_tree, None);
        let head_commit = commit(&mut src, head_tree, Some(base_commit));
        let refs = BTreeMap::from([
            ("refs/heads/main".to_string(), base_commit.to_string()),
            ("refs/heads/feat/x".to_string(), head_commit.to_string()),
        ]);
        (Arc::new(src), refs)
    }

    /// The full GitHub-faithful round-trip: open a PR from a branch → the SAME log
    /// projected by `build_pr_detail` renders a REAL head-vs-base diff, the fresh
    /// number, and the correct source/target branches. NOT an empty/stub PR.
    #[test]
    fn create_then_pr_detail_renders_real_head_vs_base_diff() {
        let (git, refs) = repo_with_branches();
        let mut log = EventLog::new();
        let a = write_pr_create(
            &mut log,
            "hugit",
            &req("feat/x", "main", "add b", Some("adds a line")),
            &refs,
            chain(),
            1_000,
        )
        .expect("open ok");
        // Fresh number (first PR on this log) + the head branch echoed back.
        assert_eq!(a.pr_number, Some(1));
        assert_eq!(a.branch.as_deref(), Some("feat/x"));
        // Exactly one pr.opened, carrying the pinned SHAs.
        let opened = log
            .records()
            .iter()
            .find(|r| r.kind == PR_OPENED_KIND)
            .expect("pr.opened present");
        let v: serde_json::Value = serde_json::from_str(&opened.payload).unwrap();
        assert_eq!(v["head"], "feat/x");
        assert_eq!(v["base"], "main");
        assert_eq!(v["head_sha"].as_str().unwrap().len(), 40);
        assert_eq!(v["base_sha"].as_str().unwrap().len(), 40);

        // Read it back exactly as GET /v1/repos/{repo}/prs/{n} does.
        let git_src: Option<&Arc<dyn ObjectSource + Send + Sync>> = Some(&git);
        let pr = build_pr_detail(&log, "hugit", 1, git_src).expect("pr detail projects");
        assert_eq!(pr.number, 1);
        assert_eq!(pr.title, "add b");
        assert_eq!(pr.source_branch, "feat/x");
        assert_eq!(pr.target_branch, "main");
        // The REAL diff: one file changed, +1/-0 — never empty/stub.
        assert_eq!(pr.file_count, 1, "the head-vs-base diff is non-empty");
        assert_eq!((pr.added, pr.removed), (1, 0));
        assert_eq!(pr.diff.files.len(), 1);
        assert_eq!(pr.diff.files[0].path, "f.txt");
    }

    /// The number advances past every prior pr.opened (dispatch/edit-propose/create
    /// share the ordinal), so a second create is PR #2.
    #[test]
    fn number_is_next_ordinal_over_all_pr_opened() {
        let (_git, refs) = repo_with_branches();
        let mut log = EventLog::new();
        write_pr_create(
            &mut log,
            "r",
            &req("feat/x", "main", "t", None),
            &refs,
            chain(),
            1,
        )
        .expect("first ok");
        let b = write_pr_create(
            &mut log,
            "r",
            &req("feat/x", "main", "t2", None),
            &refs,
            chain(),
            2,
        )
        .expect("second ok");
        assert_eq!(b.pr_number, Some(2));
    }

    /// A `head` (or `base`) that resolves to no tip on this repo → 404 (no oracle),
    /// and NOTHING is appended.
    #[test]
    fn missing_branch_is_404_no_write() {
        let (_git, refs) = repo_with_branches();
        let mut log = EventLog::new();
        let e = write_pr_create(
            &mut log,
            "r",
            &req("feat/does-not-exist", "main", "t", None),
            &refs,
            chain(),
            1,
        )
        .expect_err("unknown head must 404");
        assert_eq!(e.status, 404);
        // The base branch missing is symmetric.
        let e2 = write_pr_create(
            &mut log,
            "r",
            &req("feat/x", "nope", "t", None),
            &refs,
            chain(),
            1,
        )
        .expect_err("unknown base must 404");
        assert_eq!(e2.status, 404);
        assert!(log.records().is_empty(), "a 404 appends nothing");
    }

    /// A worker/model/anonymous principal is refused (a write is never anonymous):
    /// `asserted_class` fails closed to 404 for an unclassifiable chain, and an empty
    /// chain likewise — before any append.
    #[test]
    fn unauthorized_principal_is_refused_no_write() {
        let (_git, refs) = repo_with_branches();
        let mut log = EventLog::new();
        // Empty chain (anonymous) → fail-closed 404, no write.
        let e = write_pr_create(
            &mut log,
            "r",
            &req("feat/x", "main", "t", None),
            &refs,
            vec![],
            1,
        )
        .expect_err("anonymous write must be refused");
        assert_eq!(e.status, 404);
        // A worker principal is classified then DENIED by the D14 matrix (land is
        // Orchestrator-only) → 503 append-denied, still nothing lands.
        let e2 = write_pr_create(
            &mut log,
            "r",
            &req("feat/x", "main", "t", None),
            &refs,
            vec!["agent:runner".into()],
            1,
        )
        .expect_err("a worker cannot open a PR");
        assert_eq!(e2.status, 503);
        // The D14 matrix appends an `authz.denied` AUDIT record on a worker deny (by
        // design), but NO `pr.opened` write ever leaked — that is the invariant.
        assert!(
            !log.records().iter().any(|r| r.kind == PR_OPENED_KIND),
            "no unauthorized pr.opened leaked"
        );
    }

    /// Blank head/base/title and a self-compare are 400s; a secret-shaped title is
    /// scrubbed at the write boundary (never persisted verbatim).
    #[test]
    fn validation_and_scrub() {
        let (_git, refs) = repo_with_branches();
        let mut log = EventLog::new();
        for bad in [
            req("  ", "main", "t", None),
            req("feat/x", "  ", "t", None),
            req("feat/x", "main", "  ", None),
            req("main", "main", "t", None),
        ] {
            assert_eq!(
                write_pr_create(&mut log, "r", &bad, &refs, chain(), 1)
                    .expect_err("400")
                    .status,
                400
            );
        }
        assert!(log.records().is_empty());

        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        write_pr_create(
            &mut log,
            "r",
            &req("feat/x", "main", &format!("fix {pat}"), Some(pat)),
            &refs,
            chain(),
            9,
        )
        .expect("ok");
        for r in log.records() {
            assert!(!r.payload.contains(pat), "raw PAT in {}", r.kind);
        }
    }
}
