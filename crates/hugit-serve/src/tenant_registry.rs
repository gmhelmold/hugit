//! The per-tenant repo REGISTRY (WP-1 of the open-signup front) — the durable,
//! chain-verified authoritative set/count of the repos a tenant OWNS.
//!
//! ## Why it exists (three audit-confirmed defects, fixed at the source)
//! - **(A) reboot cap-bypass / durable undercount.** The prior
//!   [`AppState::count_owned_repos`](crate::state::AppState::count_owned_repos) counted
//!   ONLY the in-memory boot∪runtime union, so after a reboot (runtime overlay lost, the
//!   DURABLE genesis logs survive) the count collapsed and a tenant could re-provision
//!   PAST [`MAX_REPOS_PER_TENANT`](crate::state::MAX_REPOS_PER_TENANT) → an unbounded
//!   `&'static RepoState` leak / OOM. This registry is DURABLE — it survives the reboot,
//!   so the count is correct across engine lifetimes.
//! - **(B) O(all-platform-repos) synchronous R2 scan on every `POST /v1/repos`.** The old
//!   count walked EVERY loaded repo, calling `load_verified` (a chain-verified R2 fetch)
//!   per candidate, on the single-threaded accept loop. The registry count is ONE durable
//!   read of a small per-tenant log → O(1) w.r.t. the platform.
//! - It is the substrate the later erase-execute O(M) surviving-repo scan (#98-G1) reuses.
//!
//! ## Shape — an append-only, chain-verified `EventLog` (mirrors `_accounts/`)
//! The registry for a tenant lives in the reserved `_tenants/{org}.json` keyspace (the
//! SAME [`LogSource`](crate::state::LogSource) fetch/persist + canonical-JSON +
//! create-or-append discipline the `_accounts/{slug}` GDPR1 store uses). It is a chain of
//! two record kinds:
//! - [`TENANT_REPO_REGISTERED_KIND`] `{ "repo": "<slug>" }` — appended (transactionally
//!   with the genesis create) when a repo is provisioned;
//! - [`TENANT_REPO_UNREGISTERED_KIND`] `{ "repo": "<slug>" }` — appended when a repo is
//!   GDPR1-erased (the cap is a HOLD-count: erasable-down, matching the account model).
//!
//! The authoritative set is the FOLD of the chain ([`registered_repos`]): registered −
//! unregistered. Because the fold is a set, a duplicate register is idempotent (never
//! double-counts) and an unregister of an absent repo is a no-op.
//!
//! This module is PURE (it mutates a `&mut EventLog` / reads a `&EventLog`); the durable
//! load/persist CAS discipline + the reconcile live on [`AppState`](crate::state::AppState).

use std::collections::BTreeSet;

use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::error::EngineErr;

/// A repo was registered into its owner tenant's durable set (appended with the genesis
/// create). Payload: `{ "repo": "<slug>" }`.
pub const TENANT_REPO_REGISTERED_KIND: &str = "tenant.repo_registered";

/// A repo was removed from its owner tenant's durable set (appended by the GDPR1 erase
/// tombstone — the cap is erasable-down). Payload: `{ "repo": "<slug>" }`.
pub const TENANT_REPO_UNREGISTERED_KIND: &str = "tenant.repo_unregistered";

/// Extract the `repo` slug field from a registry record's JSON payload (`None` on a
/// malformed/absent field — a corrupt record is SKIPPED by the fold, never trusted).
fn record_repo(payload: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()?
        .get("repo")?
        .as_str()
        .map(str::to_string)
}

/// Fold the registry chain into the CURRENT authoritative set of owned repo slugs:
/// each [`TENANT_REPO_REGISTERED_KIND`] ADDS its slug, each
/// [`TENANT_REPO_UNREGISTERED_KIND`] REMOVES it. Set semantics ⇒ a duplicate register is
/// idempotent (no double-count) and an unregister of an absent slug is a no-op. A record
/// with a malformed/absent `repo` payload is skipped (never trusted).
#[must_use]
pub fn registered_repos(log: &EventLog) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for r in log.records() {
        match r.kind.as_str() {
            TENANT_REPO_REGISTERED_KIND => {
                if let Some(repo) = record_repo(&r.payload) {
                    set.insert(repo);
                }
            }
            TENANT_REPO_UNREGISTERED_KIND => {
                if let Some(repo) = record_repo(&r.payload) {
                    set.remove(&repo);
                }
            }
            _ => {}
        }
    }
    set
}

/// The O(1)-w.r.t.-platform authoritative count: the size of the folded owned set. Reading
/// this from the durable registry (ONE per-tenant log fetch) is what replaces the old
/// O(all-platform-repos) per-candidate `load_verified` scan (defect B) and makes the cap
/// durable across reboots (defect A).
#[must_use]
pub fn count(log: &EventLog) -> usize {
    registered_repos(log).len()
}

/// Whether `repo` is currently in the tenant's owned set (registered and not since
/// unregistered) — the idempotency check the register/unregister CAS loops consult.
#[must_use]
pub fn is_registered(log: &EventLog, repo: &str) -> bool {
    registered_repos(log).contains(repo)
}

/// Append a [`TENANT_REPO_REGISTERED_KIND`] for `repo` IFF it is not already in the set.
/// Returns `Ok(true)` when a record was appended, `Ok(false)` when it was already present
/// (idempotent no-op — the caller can skip the durable persist). Appended AS the caller
/// (chain-derived class), on the append-only log (never a rewrite).
///
/// # Errors
/// `503 ENGINE_UNAVAILABLE` — the guarded append is denied (fail-honest).
pub fn append_register(
    log: &mut EventLog,
    repo: &str,
    principal_chain: &[String],
    at: u64,
) -> Result<bool, EngineErr> {
    if is_registered(log, repo) {
        return Ok(false); // idempotent — already counted
    }
    append_kind(log, TENANT_REPO_REGISTERED_KIND, repo, principal_chain, at)?;
    Ok(true)
}

/// Append a [`TENANT_REPO_UNREGISTERED_KIND`] for `repo` IFF it is currently in the set.
/// Returns `Ok(true)` when a record was appended, `Ok(false)` when it was already absent
/// (idempotent no-op). The decrement leg of the GDPR1 erase tombstone.
///
/// # Errors
/// `503 ENGINE_UNAVAILABLE` — the guarded append is denied (fail-honest).
pub fn append_unregister(
    log: &mut EventLog,
    repo: &str,
    principal_chain: &[String],
    at: u64,
) -> Result<bool, EngineErr> {
    if !is_registered(log, repo) {
        return Ok(false); // idempotent — already decremented / never counted
    }
    append_kind(
        log,
        TENANT_REPO_UNREGISTERED_KIND,
        repo,
        principal_chain,
        at,
    )?;
    Ok(true)
}

/// The shared guarded append: a canonical-JSON `{ "repo": <slug> }` record of `kind`,
/// routed through the D14 [`append_authorized`](EventLog::append_authorized) under the
/// caller's chain-derived class (identical discipline to the account-erase / repo.erased
/// appends). The registry is a LAND-class artifact (like `_accounts/`).
fn append_kind(
    log: &mut EventLog,
    kind: &str,
    repo: &str,
    principal_chain: &[String],
    at: u64,
) -> Result<(), EngineErr> {
    let payload_value = json!({ "repo": repo });
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    let class = crate::writes::asserted_class(principal_chain)?;
    log.append_authorized(
        class,
        Endpoint::Land,
        kind,
        principal_chain.to_vec(),
        payload,
        at,
    )
    .map_err(|d| EngineErr::unavailable(format!("{kind} append denied: {}", d.reason.code())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }

    #[test]
    fn fold_registers_and_unregisters_as_a_set() {
        let mut log = EventLog::new();
        assert_eq!(count(&log), 0, "empty registry counts 0");

        assert!(append_register(&mut log, "a", &tenant("org-a"), 1).unwrap());
        assert!(append_register(&mut log, "b", &tenant("org-a"), 2).unwrap());
        assert_eq!(count(&log), 2);
        assert!(is_registered(&log, "a"));
        assert!(is_registered(&log, "b"));
        assert!(!is_registered(&log, "c"));

        // Unregister decrements the hold-count.
        assert!(append_unregister(&mut log, "a", &tenant("org-a"), 3).unwrap());
        assert_eq!(count(&log), 1);
        assert!(!is_registered(&log, "a"));
        assert!(is_registered(&log, "b"));

        assert_eq!(
            registered_repos(&log),
            ["b".to_string()].into_iter().collect()
        );
    }

    #[test]
    fn register_is_idempotent_no_double_count() {
        let mut log = EventLog::new();
        assert!(append_register(&mut log, "a", &tenant("org-a"), 1).unwrap());
        // A second register of the SAME repo is a no-op (returns false, appends nothing).
        assert!(!append_register(&mut log, "a", &tenant("org-a"), 2).unwrap());
        assert_eq!(count(&log), 1, "no double-count");
        assert_eq!(
            log.records().len(),
            1,
            "the idempotent register appends nothing"
        );
    }

    #[test]
    fn unregister_absent_is_a_noop() {
        let mut log = EventLog::new();
        assert!(!append_unregister(&mut log, "ghost", &tenant("org-a"), 1).unwrap());
        assert_eq!(count(&log), 0);
        assert!(log.records().is_empty());
    }

    #[test]
    fn re_register_after_unregister_counts_again() {
        // The hold-count is erasable-DOWN and re-usable-UP (an erased slug can be
        // re-provisioned later, re-consuming a cap slot).
        let mut log = EventLog::new();
        append_register(&mut log, "a", &tenant("org-a"), 1).unwrap();
        append_unregister(&mut log, "a", &tenant("org-a"), 2).unwrap();
        assert_eq!(count(&log), 0);
        assert!(append_register(&mut log, "a", &tenant("org-a"), 3).unwrap());
        assert_eq!(count(&log), 1);
    }

    #[test]
    fn a_corrupt_record_is_skipped_by_the_fold() {
        // A record with no `repo` field is not trusted — the fold skips it, never
        // panics or mis-counts.
        let mut log = EventLog::new();
        append_register(&mut log, "a", &tenant("org-a"), 1).unwrap();
        // Manually append a malformed registered record via the guarded door.
        let bogus = hugit_refstore::canonical_json(&json!({ "nope": 1 }).to_string())
            .unwrap_or_else(|| "{}".to_string());
        log.append_authorized(
            crate::writes::asserted_class(&tenant("org-a")).unwrap(),
            Endpoint::Land,
            TENANT_REPO_REGISTERED_KIND,
            tenant("org-a"),
            bogus,
            2,
        )
        .unwrap();
        assert_eq!(
            count(&log),
            1,
            "the malformed record is skipped, not counted"
        );
    }
}
