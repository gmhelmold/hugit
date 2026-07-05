//! `GET /v1/me/account` → [`MeAccountVm`] — the caller's OWN structured account
//! data (usage + PAT metadata), scoped by the session Bearer.
//!
//! Per-principal + isolated: the caller passes their `me` repo set (from
//! `AppState::me_repo_logs`, the SAME read-authz projection as `me/dashboard`), so
//! another principal's data is never in view (a foreign repo is simply absent). The
//! usage figures are REAL today; `pats` is empty until the engine PAT store lands (a
//! fresh caller has none) — the honest shape the window renders now + populates later.

use hugit_http_contracts::account::{AccountUsageVm, MeAccountVm, PatMetaVm};
use hugit_refstore::EventLog;

/// Build the caller's account data from their authorized `(slug, log)` set + their
/// account log (for PAT metadata).
///
/// - `repos_count` = the exact number of owned repos.
/// - `log_footprint_bytes` = the sum of the serialized per-repo event-log sizes — the
///   HONEST, cheap per-account durable footprint. The git CONTENT objects are
///   content-addressed + cross-tenant DEDUPLICATED, so they are NOT a clean per-account
///   figure and are deliberately excluded (a true content meter is a deferred CoreLink
///   seam — see the contract doc).
/// - `pats` = the caller's OWN live tokens' metadata (never the secret hash / another
///   user's tokens), projected from `account_log`. `None` (operator/anon/no account)
///   → an empty list.
#[must_use]
pub fn build_me_account(
    repos: &[(String, EventLog)],
    account_log: Option<&EventLog>,
    user: &str,
) -> MeAccountVm {
    let repos_count = repos.len() as u64;
    let log_footprint_bytes = repos
        .iter()
        .map(|(_, log)| {
            // The durable JSON footprint of this repo's event log (what R2 stores).
            serde_json::to_vec(log.records())
                .map(|b| b.len())
                .unwrap_or(0) as u64
        })
        .sum();
    // Project the caller's OWN live PATs (metadata only — SECRET never leaves the
    // engine, ADR-0002). A caller with no account log (operator/anon) → empty.
    let pats: Vec<PatMetaVm> = account_log
        .map(|log| crate::writes::verbs::write_token::project_pats(log, user))
        .unwrap_or_default();
    MeAccountVm {
        usage: AccountUsageVm {
            repos_count,
            log_footprint_bytes,
        },
        pats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    fn repo_log(owner: &str) -> EventLog {
        let mut log = EventLog::new();
        let meta = serde_json::json!({"visibility":"private","owner_tenant":owner}).to_string();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "repo.meta",
            vec!["o".into()],
            hugit_refstore::canonical_json(&meta).unwrap_or(meta),
            1,
        )
        .expect("append repo.meta");
        log
    }

    #[test]
    fn usage_counts_repos_and_sums_log_bytes() {
        let repos = vec![
            ("alpha".to_string(), repo_log("org-a")),
            ("beta".to_string(), repo_log("org-a")),
        ];
        let vm = build_me_account(&repos, None, "clerk:org-a:u1");
        assert_eq!(vm.usage.repos_count, 2);
        assert!(
            vm.usage.log_footprint_bytes > 0,
            "the footprint is the real serialized log size"
        );
        // Deterministic: the sum equals the two logs' serialized sizes.
        let expected: u64 = repos
            .iter()
            .map(|(_, l)| serde_json::to_vec(l.records()).unwrap().len() as u64)
            .sum();
        assert_eq!(vm.usage.log_footprint_bytes, expected);
    }

    #[test]
    fn no_repos_and_no_account_log_is_an_honest_empty_account() {
        let vm = build_me_account(&[], None, "clerk:org-a:u1");
        assert_eq!(vm.usage.repos_count, 0);
        assert_eq!(vm.usage.log_footprint_bytes, 0);
        assert!(vm.pats.is_empty(), "no account log → no pats");
    }

    #[test]
    fn pats_are_projected_from_the_account_log_for_the_caller() {
        // An account log carrying one pat.created for the caller → it appears; nothing
        // for another user.
        let mut alog = EventLog::new();
        let payload = serde_json::json!({
            "id":"pat_abc","name":"ci","user":"clerk:org-a:u1",
            "scopes":["repo:write"],"secret_hash":"deadbeef","created_at":5,"expires_at":0
        })
        .to_string();
        alog.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            crate::writes::verbs::write_token::PAT_CREATED_KIND,
            vec!["clerk:org-a:u1".into()],
            hugit_refstore::canonical_json(&payload).unwrap_or(payload),
            5,
        )
        .unwrap();
        let mine = build_me_account(&[], Some(&alog), "clerk:org-a:u1");
        assert_eq!(mine.pats.len(), 1);
        assert_eq!(mine.pats[0].id, "pat_abc");
        // A different user sees none (cross-user isolation).
        let other = build_me_account(&[], Some(&alog), "clerk:org-a:u2");
        assert!(other.pats.is_empty());
    }
}
