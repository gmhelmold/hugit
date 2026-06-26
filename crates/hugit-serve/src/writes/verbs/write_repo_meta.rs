//! `write_repo_meta` — the pure `repo meta` write verb. Records `repo.meta`.
//!
//! Sets the repo's visibility (and optionally its `owner_tenant`) by appending the
//! canonical `repo.meta` record that [`crate::authz::project_repo_meta`] already
//! projects. The read gate (`authorize_read`) and the git-wire public-clone gate
//! both re-decide from the log on every request — once this record exists the read
//! gate will reflect the new visibility immediately.
//!
//! Security: this verb sets the authz predicate itself. It MUST be operator/owner-only.
//! The guarantee is structural: `with_write` calls `authorize_write` before any verb
//! runs, and `authorize_write` is fail-closed (ownership, not visibility). No
//! additional check is required here.
//!
//! Fail-safe: the absent-record default (PRIVATE, no owner) is upheld by
//! `project_repo_meta` — this verb only APPENDS a new record; it never removes one.

use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::RepoMetaReq;
use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::authz::REPO_META_KIND;
use crate::error::EngineErr;

/// `POST /v1/repos/{repo}/repo/meta` (operator/owner-only via `with_write`).
///
/// # Errors
/// - `400 INVALID_REQUEST` — `visibility` is not `"public"` or `"private"`.
/// - `503 ENGINE_UNAVAILABLE` — append denied (fail-honest).
pub fn write_repo_meta(
    log: &mut EventLog,
    repo: &str,
    req: &RepoMetaReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    // Validate: visibility must be exactly "public" or "private" — fail-closed on
    // any other value (e.g. "PUBLIC", "", "true", garbage) so the authz predicate
    // is never set to an unexpected/unknown state.
    let vis = req.visibility.trim();
    if vis != "public" && vis != "private" {
        return Err(EngineErr::invalid_request(
            "visibility deve ser \"public\" ou \"private\"",
        ));
    }
    let payload_value = match &req.owner_tenant {
        Some(ot) => json!({"visibility": vis, "owner_tenant": ot}),
        None => json!({"visibility": vis}),
    };
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    // D14: assert the REAL caller's class (chain-derived, fail-closed).
    let class = crate::writes::asserted_class(&principal_chain)?;
    let record = log
        .append_authorized(
            class,
            Endpoint::Land,
            REPO_META_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("repo.meta append denied: {}", d.reason.code()))
        })?;
    Ok(Accepted {
        seq: record.seq,
        note: format!("visibilidade definida como {vis}"),
        extra: None,
        queue_pos: None,
        pr_number: None,
        branch: None,
        state: None,
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::{Visibility, authorize_read, authorize_write, project_repo_meta};

    fn chain_op() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }
    fn chain_tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }
    fn anon() -> Vec<String> {
        vec![]
    }

    #[test]
    fn sets_visibility_public_and_project_returns_public() {
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "public".into(),
            owner_tenant: Some("org-a".into()),
        };
        let a = write_repo_meta(&mut log, "r", &req, chain_op(), 1).expect("ok");
        let meta = project_repo_meta(&log);
        assert_eq!(meta.visibility, Visibility::Public);
        assert_eq!(meta.owner_tenant.as_deref(), Some("org-a"));
        // The seq is present in the log.
        assert!(log.records().iter().any(|r| r.seq == a.seq));
    }

    #[test]
    fn sets_visibility_private() {
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "private".into(),
            owner_tenant: Some("org-b".into()),
        };
        write_repo_meta(&mut log, "r", &req, chain_op(), 1).expect("ok");
        let meta = project_repo_meta(&log);
        assert_eq!(meta.visibility, Visibility::Private);
        assert_eq!(meta.owner_tenant.as_deref(), Some("org-b"));
    }

    #[test]
    fn invalid_visibility_is_400() {
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "PUBLIC".into(),
            owner_tenant: None,
        };
        let err = write_repo_meta(&mut log, "r", &req, chain_op(), 1).expect_err("400");
        assert_eq!(err.status, 400);
        assert_eq!(err.code, "INVALID_REQUEST");
        // No record appended on rejection.
        assert!(log.records().is_empty());
    }

    #[test]
    fn empty_visibility_is_400() {
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "".into(),
            owner_tenant: None,
        };
        assert_eq!(
            write_repo_meta(&mut log, "r", &req, chain_op(), 1)
                .expect_err("400")
                .status,
            400
        );
    }

    #[test]
    fn authorize_read_anon_allowed_after_public_meta() {
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "public".into(),
            owner_tenant: Some("org-a".into()),
        };
        write_repo_meta(&mut log, "r", &req, chain_op(), 1).expect("ok");
        let meta = project_repo_meta(&log);
        // Anonymous public-clone gate: anon MUST now be allowed.
        assert!(
            authorize_read(&anon(), &meta),
            "anon must read a public repo after repo.meta is set"
        );
        // Owner-tenant may still read.
        assert!(authorize_read(&chain_tenant("org-a"), &meta));
    }

    #[test]
    fn authorize_write_unaffected_by_public_visibility() {
        // Public opens READS to everyone, but MUST NOT enable anon/cross-tenant writes.
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "public".into(),
            owner_tenant: Some("org-a".into()),
        };
        write_repo_meta(&mut log, "r", &req, chain_op(), 1).expect("ok");
        let meta = project_repo_meta(&log);
        // A non-owner tenant cannot write.
        assert!(
            !authorize_write(&chain_tenant("org-b"), &meta),
            "a non-owner tenant must NOT write a public repo"
        );
        // Anonymous cannot write.
        assert!(!authorize_write(&anon(), &meta), "anon must not write");
        // The owner may write.
        assert!(authorize_write(&chain_tenant("org-a"), &meta));
        // The operator may write.
        assert!(authorize_write(&chain_op(), &meta));
    }

    #[test]
    fn no_owner_tenant_in_req_leaves_no_owner() {
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "public".into(),
            owner_tenant: None,
        };
        write_repo_meta(&mut log, "r", &req, chain_op(), 1).expect("ok");
        let meta = project_repo_meta(&log);
        assert_eq!(meta.owner_tenant, None);
        // operator can still write; tenant cannot (no owner set).
        assert!(authorize_write(&chain_op(), &meta));
        assert!(!authorize_write(&chain_tenant("org-a"), &meta));
    }

    #[test]
    fn note_contains_visibility_value() {
        let mut log = EventLog::new();
        let req = RepoMetaReq {
            visibility: "public".into(),
            owner_tenant: None,
        };
        let a = write_repo_meta(&mut log, "r", &req, chain_op(), 1).expect("ok");
        assert!(
            a.note.contains("public"),
            "note should mention the visibility"
        );
    }
}
