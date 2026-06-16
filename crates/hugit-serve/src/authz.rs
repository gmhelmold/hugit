//! Engine-side per-tenant read authorization (the cross-tenant READ gate).
//!
//! The authz DECISION lives in the engine and is re-decided **fail-closed on
//! every `/v1/repos/{repo}/*` read** — the window's `viewer_can` is a cosmetic
//! hint only (githugr TL request 2026-06-15 / ADR-0007 §3). A denied private repo
//! is a **404** (identical to a non-existent repo — no existence oracle).
//!
//! The decision uses the ENGINE-RESOLVED principal (from `two_tier_auth`), never
//! a window-supplied claim:
//! - `orchestrator:*` → the platform/dev OPERATOR → sees all (keeps single-tenant
//!   dev + the launch repo working; the bootstrap).
//! - `clerk:{org}:{user}` → a tenant principal → sees `public` repos, and
//!   `private` repos ONLY when the repo's `owner_tenant == org`.
//! - anything else → Unknown → denied on private.
//!
//! `owner_tenant` + `visibility` are projected from the log as the latest
//! `repo.meta` record; **fail-safe defaults: PRIVATE, no owner_tenant** (so a
//! repo with no meta is visible only to the operator until its tenant is assigned
//! — never default-public).

use hugit_refstore::EventLog;
use serde_json::Value;

/// The event kind carrying a repo's authz metadata (visibility + owner_tenant).
/// Latest-wins projection (sibling to `policy.set`). Set at repo creation (the
/// owner_tenant assignment path is the later seam — see the handoff reply).
pub const REPO_META_KIND: &str = "repo.meta";

/// A repo's visibility. Defaults to [`Private`](Visibility::Private) (fail-safe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    /// The MACHINE value the wire carries (`"public" | "private"`) — never a
    /// localized display string. Locale lives in the window (githugr TL decision
    /// 2026-06-15: clean contract = machine value + window-side i18n). This is the
    /// single source of truth for the `RepoChromeVm.visibility` field.
    #[must_use]
    pub fn as_machine_str(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
        }
    }
}

/// A repo's projected authz metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMeta {
    pub visibility: Visibility,
    /// The tenant that owns the repo's objects, or `None` when unassigned.
    pub owner_tenant: Option<String>,
}

impl RepoMeta {
    /// Fail-safe default: PRIVATE, unassigned. A repo with no `repo.meta` record
    /// is therefore operator-only until its tenant is assigned — never open.
    fn private_default() -> Self {
        Self {
            visibility: Visibility::Private,
            owner_tenant: None,
        }
    }
}

/// Project the repo's authz metadata from the (already chain-verified) log:
/// the LATEST `repo.meta` record wins; absent ⇒ the fail-safe private default.
#[must_use]
pub fn project_repo_meta(log: &EventLog) -> RepoMeta {
    let mut meta = RepoMeta::private_default();
    for r in log.records().iter().filter(|r| r.kind == REPO_META_KIND) {
        let Ok(v) = serde_json::from_str::<Value>(&r.payload) else {
            continue;
        };
        if let Some(vis) = v.get("visibility").and_then(Value::as_str) {
            meta.visibility = if vis == "public" {
                Visibility::Public
            } else {
                Visibility::Private // any non-"public" value is private (fail-safe)
            };
        }
        if let Some(ot) = v.get("owner_tenant").and_then(Value::as_str) {
            meta.owner_tenant = (!ot.is_empty()).then(|| ot.to_string());
        }
    }
    meta
}

/// The caller's identity, derived from the engine-resolved principal chain.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Caller {
    /// Platform/dev operator (`orchestrator:*`) — bypass.
    Operator,
    /// A tenant principal (`clerk:{org}:{user}`) carrying its org.
    Tenant(String),
    /// Unrecognized principal — fail-closed on private.
    Unknown,
}

fn caller(principal: &[String]) -> Caller {
    match principal.first().map(String::as_str) {
        Some(p) if p.starts_with("orchestrator:") => Caller::Operator,
        Some(p) => match p.strip_prefix("clerk:") {
            // "clerk:{org}:{user}" → the org segment.
            Some(rest) => Caller::Tenant(rest.split(':').next().unwrap_or("").to_string()),
            None => Caller::Unknown,
        },
        None => Caller::Unknown,
    }
}

/// THE read gate — fail-closed. `true` = ALLOW the read; `false` = deny (the
/// caller maps deny to a 404, no existence leak).
///
/// - Operator → always allow (platform/dev bypass; the bootstrap).
/// - `public` → allow anyone authenticated.
/// - `private` → allow ONLY a tenant principal whose org equals a SET
///   `owner_tenant`. A private repo with no `owner_tenant`, or an unknown caller,
///   is denied.
#[must_use]
pub fn authorize_read(principal: &[String], meta: &RepoMeta) -> bool {
    match caller(principal) {
        Caller::Operator => true,
        c => match meta.visibility {
            Visibility::Public => true,
            Visibility::Private => {
                matches!((c, &meta.owner_tenant), (Caller::Tenant(org), Some(owner)) if &org == owner)
            }
        },
    }
}

/// THE write gate — fail-closed, and STRICTER than [`authorize_read`]. `true` =
/// ALLOW the mutation; `false` = deny (the caller maps deny to a 404).
///
/// Write permission is OWNERSHIP, not read-visibility — `public` opens READS to
/// everyone, but NEVER writes (else any signed-up tenant could `land`/`verdict`/
/// `policy`/… into a public repo). So visibility is NOT consulted here:
/// - Operator (`orchestrator:*`) → always allow (platform/dev; the bootstrap).
/// - The owning tenant (`clerk:{org}:…` whose org equals a SET `owner_tenant`) →
///   allow.
/// - Everyone else → deny, INCLUDING on a public repo and on a private repo with
///   no `owner_tenant` (writes stay operator-only until ownership is assigned).
#[must_use]
pub fn authorize_write(principal: &[String], meta: &RepoMeta) -> bool {
    match caller(principal) {
        Caller::Operator => true,
        c => matches!(
            (c, &meta.owner_tenant),
            (Caller::Tenant(org), Some(owner)) if &org == owner
        ),
    }
}

/// `true` iff the principal is the platform/dev OPERATOR (`orchestrator:*`).
///
/// Used to decide who may see a load/verify failure's honest error (a 503 with
/// integrity/transport detail) on a repo they may not own: ONLY the operator does;
/// every other caller gets a uniform 404, so a load failure on a private repo a
/// caller doesn't own is never an existence/integrity oracle (audit 2026-06-16 —
/// upholds the "deny→404, no existence leak" law for the load-failure path too).
#[must_use]
pub fn is_operator(principal: &[String]) -> bool {
    matches!(caller(principal), Caller::Operator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    fn op() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }
    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }
    fn public() -> RepoMeta {
        RepoMeta {
            visibility: Visibility::Public,
            owner_tenant: Some("org-a".into()),
        }
    }
    fn private(owner: Option<&str>) -> RepoMeta {
        RepoMeta {
            visibility: Visibility::Private,
            owner_tenant: owner.map(str::to_string),
        }
    }

    #[test]
    fn operator_sees_everything() {
        assert!(authorize_read(&op(), &private(Some("org-a"))));
        assert!(authorize_read(&op(), &private(None)));
        assert!(authorize_read(&op(), &public()));
    }

    #[test]
    fn public_is_open_to_any_tenant() {
        assert!(authorize_read(&tenant("org-b"), &public()));
    }

    #[test]
    fn private_allows_only_the_owning_tenant() {
        assert!(authorize_read(&tenant("org-a"), &private(Some("org-a"))));
    }

    #[test]
    fn private_denies_a_different_tenant() {
        assert!(!authorize_read(&tenant("org-b"), &private(Some("org-a"))));
    }

    // ── write gate: ownership, NOT visibility ────────────────────────────────

    #[test]
    fn write_on_public_is_denied_to_a_non_owner_tenant() {
        // THE hole this closes: `public` opens reads to all, but a non-owner tenant
        // must NOT be able to write to a public repo (the read gate would allow it).
        assert!(authorize_read(&tenant("org-b"), &public()), "read is open");
        assert!(
            !authorize_write(&tenant("org-b"), &public()),
            "write is NOT — ownership, not visibility"
        );
    }

    #[test]
    fn write_is_allowed_to_the_owner_and_operator() {
        assert!(authorize_write(&tenant("org-a"), &public())); // owner of the public repo
        assert!(authorize_write(&tenant("org-a"), &private(Some("org-a"))));
        assert!(authorize_write(&op(), &public()));
        assert!(authorize_write(&op(), &private(None)));
    }

    #[test]
    fn write_on_a_no_owner_repo_is_operator_only() {
        // Public OR private with no owner_tenant → writes stay operator-only.
        let public_no_owner = RepoMeta {
            visibility: Visibility::Public,
            owner_tenant: None,
        };
        assert!(!authorize_write(&tenant("org-a"), &public_no_owner));
        assert!(!authorize_write(&tenant("org-a"), &private(None)));
        assert!(authorize_write(&op(), &public_no_owner));
    }

    #[test]
    fn is_operator_only_for_orchestrator() {
        assert!(is_operator(&op()));
        assert!(!is_operator(&tenant("org-a")));
        assert!(!is_operator(&["clerk:org-a:u".to_string()]));
        assert!(!is_operator(&["weird:x".to_string()]));
        assert!(!is_operator(&[]));
    }

    #[test]
    fn write_denies_unknown_principal_everywhere() {
        let weird = ["weird:thing".to_string()];
        assert!(!authorize_write(&weird, &public()));
        assert!(!authorize_write(&weird, &private(Some("org-a"))));
        assert!(!authorize_write(&[], &public()));
    }

    #[test]
    fn private_with_no_owner_denies_non_operator() {
        assert!(!authorize_read(&tenant("org-a"), &private(None)));
    }

    #[test]
    fn unknown_principal_denied_on_private() {
        assert!(!authorize_read(
            &["weird:thing".to_string()],
            &private(Some("org-a"))
        ));
        assert!(!authorize_read(&[], &private(Some("org-a"))));
        // ...but an unknown principal can still read a PUBLIC repo (it IS authed).
        assert!(authorize_read(&["weird:thing".to_string()], &public()));
    }

    fn append(log: &mut EventLog, kind: &str, payload: Value) {
        let body = hugit_refstore::canonical_json(&payload.to_string())
            .unwrap_or_else(|| payload.to_string());
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Push,
            kind,
            vec!["o".into()],
            body,
            0,
        )
        .expect("append");
    }

    #[test]
    fn projection_defaults_private_when_no_meta() {
        let m = project_repo_meta(&EventLog::new());
        assert_eq!(m.visibility, Visibility::Private);
        assert_eq!(m.owner_tenant, None);
    }

    #[test]
    fn projection_reads_latest_meta() {
        let mut log = EventLog::new();
        append(
            &mut log,
            REPO_META_KIND,
            serde_json::json!({"visibility":"private","owner_tenant":"org-a"}),
        );
        append(
            &mut log,
            REPO_META_KIND,
            serde_json::json!({"visibility":"public","owner_tenant":"org-a"}),
        ); // latest wins
        let m = project_repo_meta(&log);
        assert_eq!(m.visibility, Visibility::Public);
        assert_eq!(m.owner_tenant.as_deref(), Some("org-a"));
    }

    #[test]
    fn projection_empty_owner_tenant_is_none() {
        let mut log = EventLog::new();
        append(
            &mut log,
            REPO_META_KIND,
            serde_json::json!({"visibility":"private","owner_tenant":""}),
        );
        assert_eq!(project_repo_meta(&log).owner_tenant, None);
    }

    #[test]
    fn end_to_end_cross_tenant_denied_owner_allowed() {
        let mut log = EventLog::new();
        append(
            &mut log,
            REPO_META_KIND,
            serde_json::json!({"visibility":"private","owner_tenant":"org-a"}),
        );
        let meta = project_repo_meta(&log);
        assert!(authorize_read(&tenant("org-a"), &meta), "owner allowed");
        assert!(
            !authorize_read(&tenant("org-b"), &meta),
            "cross-tenant denied"
        );
        assert!(authorize_read(&op(), &meta), "operator allowed");
    }
}
