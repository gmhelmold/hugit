//! `write_edit_propose` — the pure `edit/{path}/propose` write verb.
//!
//! A web edit (full file body + title + optional description) becomes (a) a
//! `ref.update` for a new branch keyed to the scrubbed content's SHA-256, and
//! (b) a `pr.opened` for a new PR (spec §3: "a signed external commit on a branch
//! + a PR, never a synthetic intent"). Returns `{pr_number, branch}`.
//!
//! Assumptions (lead-decided): branch = `edit/<sanitized-path>-<at>`; pr_id =
//! `count(pr.opened)+1` (log-local ordinal; the GitHub PR number is the P2 seam);
//! content stored as its SHA-256 ref target (the CAS blob binding is the P2 seam —
//! the hash is real, never a fabricated CAS ref). `content` is scrubbed (secrets
//! redacted; code survives) — NEVER dropped (spec §3: would lose the user's work).

use hugit_cli::pr::PR_OPENED_KIND;
use hugit_contracts::event_record::EventRecord;
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::EditProposeReq;
use hugit_refstore::intent::ExternalChangeKind;
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::EngineErr;
use crate::fmt::scrub;

const EDIT_PROPOSE_CAMPAIGN: &str = "web-edit";
const EDIT_AUTHOR_KIND: &str = "orchestrator";

fn content_sha256(scrubbed: &str) -> String {
    let digest = Sha256::digest(scrubbed.as_bytes());
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn derive_branch(path: &str, at: u64) -> String {
    let sanitized: String = path
        .bytes()
        .map(|b| if b == b'/' { b'-' } else { b })
        .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        .map(char::from)
        .collect();
    let stem = if sanitized.is_empty() {
        "file".to_string()
    } else {
        sanitized
    };
    format!("edit/{stem}-{at}")
}

fn next_pr_id(log: &EventLog) -> u64 {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_OPENED_KIND)
        .count() as u64
        + 1
}

/// `POST /v1/repos/{repo}/edit/{path}/propose`.
///
/// # Errors
/// - `400 INVALID_REQUEST` — `content` or `title` blank.
/// - `503 ENGINE_UNAVAILABLE` — append denied (mapped fail-honest).
pub fn write_edit_propose(
    log: &mut EventLog,
    repo: &str,
    path: &str,
    req: &EditProposeReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    if req.content.trim().is_empty() {
        return Err(EngineErr::invalid_request(
            "content vazio — descartá-lo perderia o trabalho do usuário (spec §3)",
        ));
    }
    if req.title.trim().is_empty() {
        return Err(EngineErr::invalid_request("title vazio"));
    }
    // The file BODY is content-addressed: its SHA-256 is the ref target and the
    // body itself goes to the CAS at P2 (as the user committed it). It is NEVER
    // stored in the event log, so it is NOT field-scrubbed — `scrub` replaces a
    // whole secret-bearing field with `[REDACTED]`, which would DESTROY the file
    // (spec §3: never lose the user's work). A secret in a committed file is a
    // pre-commit-scan concern, not the log-redaction boundary. The log holds only
    // the hash (no body → no leak) + scrubbed metadata.
    let content_hash = content_sha256(&req.content);
    let title_scrubbed = scrub(&req.title);
    let description_scrubbed: Option<String> = req.description.as_deref().map(scrub);
    let branch = derive_branch(path, at);
    let pr_id = next_pr_id(log);
    let pr_id_str = pr_id.to_string();

    let ref_payload_value = json!({"ref": format!("refs/heads/{branch}"), "target": content_hash});
    let ref_payload = hugit_refstore::canonical_json(&ref_payload_value.to_string())
        .unwrap_or_else(|| ref_payload_value.to_string());
    log.append_external_change(
        ExternalChangeKind::RefUpdate,
        principal_chain.clone(),
        ref_payload,
        at,
    );

    let pr_payload_value = json!({
        "author_kind": EDIT_AUTHOR_KIND,
        "branch": branch,
        "campaign": EDIT_PROPOSE_CAMPAIGN,
        "content_sha": content_hash,
        "description": description_scrubbed,
        "intent_ids": [],
        "path": scrub(path),
        "pr_id": pr_id_str,
        "principal": null,
        "run_id": null,
        "title": title_scrubbed,
    });
    let pr_payload = hugit_refstore::canonical_json(&pr_payload_value.to_string())
        .unwrap_or_else(|| pr_payload_value.to_string());
    let record: EventRecord = log
        .append_authorized(
            PrincipalClass::Orchestrator,
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
        note: "proposta criada".to_string(),
        extra: None,
        queue_pos: None,
        pr_number: Some(pr_id),
        branch: Some(branch),
        state: None,
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Vec<String> {
        vec!["orchestrator:web".to_string()]
    }
    fn req(content: &str, title: &str) -> EditProposeReq {
        EditProposeReq {
            content: content.to_string(),
            title: title.to_string(),
            description: Some("d".into()),
        }
    }

    #[test]
    fn propose_appends_ref_update_and_pr_opened() {
        let mut log = EventLog::new();
        let a = write_edit_propose(
            &mut log,
            "r",
            "src/main.rs",
            &req("fn main(){}", "fix"),
            chain(),
            1_000,
        )
        .expect("ok");
        assert_eq!(a.pr_number, Some(1));
        let b = a.branch.as_deref().unwrap();
        assert!(b.starts_with("edit/") && b.ends_with("-1000"));
        assert!(log.records().iter().any(|r| r.kind == "ref.update"));
        assert!(log.records().iter().any(|r| r.kind == PR_OPENED_KIND));
    }

    #[test]
    fn content_body_not_in_log_only_its_hash() {
        // The file body is content-addressed (CAS at P2), NEVER stored in the log.
        // So even a PAT inside the committed file never reaches the event log —
        // the log holds only the content HASH + scrubbed metadata. (The body is
        // NOT field-scrubbed: that would replace the whole file with [REDACTED]
        // and lose the user's work, spec §3.)
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let content = format!("fn f(){{ let t=\"{pat}\"; }}");
        let a = write_edit_propose(&mut log, "r", "a.rs", &req(&content, "t"), chain(), 2_000)
            .expect("ok");
        // The raw PAT (and the body) must be ABSENT from every log payload.
        let all: String = log.records().iter().map(|r| r.payload.as_str()).collect();
        assert!(!all.contains(pat), "raw PAT must not be in the log");
        assert!(!all.contains("fn f"), "file body must not be in the log (CAS at P2)");
        // The content_sha is the hash of the RAW body (the real content-address
        // the CAS verifies), present on the pr.opened + ref.update.
        let expected = content_sha256(&content);
        let pr = log.records().iter().find(|r| r.kind == PR_OPENED_KIND).unwrap();
        let v: serde_json::Value = serde_json::from_str(&pr.payload).unwrap();
        assert_eq!(v["content_sha"].as_str(), Some(expected.as_str()));
        assert!(a.branch.is_some());
    }

    #[test]
    fn empty_content_is_400() {
        let mut log = EventLog::new();
        assert_eq!(
            write_edit_propose(&mut log, "r", "f.rs", &req("  ", "t"), chain(), 1)
                .expect_err("400")
                .status,
            400
        );
        assert!(log.records().is_empty());
    }

    #[test]
    fn branch_sanitizes() {
        assert_eq!(derive_branch("src/a/t.rs", 9), "edit/src-a-t.rs-9");
        assert_eq!(derive_branch("", 42), "edit/file-42");
    }
}
