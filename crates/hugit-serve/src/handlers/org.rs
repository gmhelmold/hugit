//! `GET /v1/orgs/{name}` → [`OrgVm`].
//!
//! A thin REAL read: `name` is the path param (REAL — the org the caller asked
//! for); `repos` is the real repo set this engine knows (the loaded + verified
//! repo log, one row). Multi-tenant repo enumeration is the P2 seam; today the
//! engine knows exactly the one repo whose log was loaded by the caller. All other
//! `OrgVm` fields that require multi-tenant identity (`members`/`billing`/
//! `github_app_line`) are honest-null/empty — gated on the P2 identity seam and
//! documented as such, never fabricated.
//!
//! ## REAL vs honest-default
//! - **REAL**: `name` (path param), `handle` (`@<name>`), `avatar_letter` (first
//!   char of name, uppercased), `repos[0].name` + `repos[0].org` (from the log's
//!   repo slug via the caller), `repos[0].open_prs` (fold of pr.opened ≠ terminal).
//! - **HONEST-DEFAULT**: `bio`/`location` (no org-profile record), `github_app_line`
//!   (no live GitHub-App seam), `people` (no multi-tenant identity), `agent_note`
//!   (fixed doctrine string), `repos[0].main_green`/`stack`/`visibility`/`status`
//!   (no CI-seam / no tenant provisioning this wave — same posture as `dashboard`).

use hugit_cli::pr::{PR_ABANDONED_KIND, PR_LANDED_KIND, PR_OPENED_KIND, all_pr_queued};
use hugit_http_contracts::common::DashboardRepoVm;
use hugit_http_contracts::org::OrgVm;
use hugit_refstore::EventLog;
use serde_json::Value;

/// Fixed doctrine caption (mirrors `dashboard`'s comment on agents-as-members).
const AGENT_NOTE: &str = "Agentes não são membros — PR e campanha têm sempre dono humano.";

/// Count the open (non-terminal) PRs in the log. A PR is open when it has NOT
/// landed (`pr.landed`) and has NOT been abandoned (`pr.abandoned`). Mirrors the
/// same predicate `dashboard::build_dashboard` uses for `open_prs`.
fn count_open_prs(log: &EventLog) -> u32 {
    // Collect every distinct pr_id that has appeared in a `pr.opened` record.
    let opened_ids: std::collections::BTreeSet<String> = log
        .records()
        .iter()
        .filter(|r| r.kind == PR_OPENED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter_map(|v| v.get("pr_id").and_then(Value::as_str).map(str::to_string))
        .collect();

    let queued_ids: std::collections::BTreeSet<String> =
        all_pr_queued(log).into_iter().map(|q| q.pr_id).collect();

    let mut open: u32 = 0;
    for pr_id in &opened_ids {
        let landed = has_event(log, PR_LANDED_KIND, pr_id);
        let abandoned = has_event(log, PR_ABANDONED_KIND, pr_id);
        if !landed && !abandoned {
            // proposed OR queued — both are "open"
            let _ = queued_ids.contains(pr_id); // just for documentation
            open += 1;
        }
    }
    open
}

/// Whether the log has a record of `kind` whose `pr_id` payload field equals `id`.
fn has_event(log: &EventLog, kind: &str, id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(id))
}

/// Split a caller-supplied `repo` slug into `(org, name)` on the first `/`.
/// Structural (caller-supplied, not log free-text) → NOT scrubbed.
/// No `/` → `("", <full slug>)`.
fn split_repo(repo: &str) -> (String, String) {
    match repo.split_once('/') {
        Some((org, name)) => (org.to_string(), name.to_string()),
        None => (String::new(), repo.to_string()),
    }
}

/// Build the org view-model.
///
/// `org_name` is the path param the caller supplied (`/v1/orgs/{name}`).
/// `log` + `repo` are the engine's one loaded repo (the launch repo) — used to
/// populate the real `repos` row. Both are ALREADY chain-verified by the caller.
#[must_use]
pub fn build_org(log: &EventLog, org_name: &str, repo: &str) -> OrgVm {
    let (repo_org, repo_name) = split_repo(repo);

    let open_prs = count_open_prs(log);

    let repo_row = DashboardRepoVm {
        org: repo_org,                // DERIVED (structural)
        name: repo_name,              // DERIVED (structural)
        main_green: false,            // HONEST-DEFAULT — no CI seam
        status: String::new(),        // HONEST-DEFAULT — no CI seam
        stack: String::new(),         // HONEST-DEFAULT — no language seam
        visibility: String::new(),    // HONEST-DEFAULT — tenant provisioning is P2
        open_prs,                     // REAL
        last_activity: String::new(), // HONEST-DEFAULT (not needed on org screen)
    };

    // `avatar_letter`: the first char of the org name, uppercased.
    let avatar_letter = org_name
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_default();

    OrgVm {
        name: org_name.to_string(),
        handle: format!("@{org_name}"),
        avatar_letter,
        bio: String::new(),             // HONEST-DEFAULT — no org-profile record
        location: String::new(),        // HONEST-DEFAULT — no org-profile record
        github_app_line: String::new(), // HONEST-DEFAULT — no GitHub-App seam
        repos: vec![repo_row],          // REAL (one row — the engine's launch repo)
        people: vec![],                 // HONEST-DEFAULT — multi-tenant identity is P2
        agent_note: AGENT_NOTE.to_string(),
    }
}

/// Build the org view AGGREGATED over the CALLER's own repos (W-METENANT). The
/// header (`name`/`handle`/`avatar_letter`/…) derives purely from the `org_name`
/// path param; the `repos` list is one row per repo in the caller's authorized
/// `(slug, verified-log)` set (from `AppState::me_repo_logs`, already
/// read-authz-filtered) — never a hardcoded default repo. An EMPTY set → an org
/// view with NO repo rows (honest empty). VM WIRE SHAPE unchanged (`OrgVm`).
/// Reuses the per-repo [`build_org`] verbatim for both the header shell and each
/// repo row (no second projection to drift).
#[must_use]
pub fn build_me_org(org_name: &str, repos: &[(String, EventLog)]) -> OrgVm {
    // The header shell: build_org over an empty log yields the org-name-derived
    // header; its one placeholder repo row is discarded below.
    let mut vm = build_org(&EventLog::new(), org_name, "");
    vm.repos = repos
        .iter()
        .filter_map(|(slug, log)| build_org(log, org_name, slug).repos.into_iter().next())
        .collect();
    vm
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, EventLog, PrincipalClass};

    fn append(log: &mut EventLog, kind: &str, payload: serde_json::Value, at: u64) {
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

    fn open_pr(log: &mut EventLog, pr_id: &str, at: u64) {
        append(
            log,
            PR_OPENED_KIND,
            serde_json::json!({"author_kind":"orchestrator","campaign":"","intent_ids":[],"pr_id":pr_id}),
            at,
        );
    }

    fn land_pr(log: &mut EventLog, pr_id: &str, at: u64) {
        append(log, PR_LANDED_KIND, serde_json::json!({"pr_id": pr_id}), at);
    }

    #[test]
    fn empty_log_honest_defaults() {
        let vm = build_org(&EventLog::new(), "hugit", "hugit");
        assert_eq!(vm.name, "hugit");
        assert_eq!(vm.handle, "@hugit");
        assert_eq!(vm.avatar_letter, "H");
        assert_eq!(vm.bio, "");
        assert_eq!(vm.location, "");
        assert_eq!(vm.github_app_line, "");
        assert_eq!(vm.repos.len(), 1);
        assert_eq!(vm.repos[0].open_prs, 0);
        assert!(!vm.repos[0].main_green);
        assert!(vm.people.is_empty());
        assert_eq!(vm.agent_note, AGENT_NOTE);
    }

    #[test]
    fn real_name_and_repos_populated() {
        // name + handle derive from the path param; repos come from the log slug.
        let mut log = EventLog::new();
        open_pr(&mut log, "1", 1000);
        open_pr(&mut log, "2", 2000);
        land_pr(&mut log, "2", 3000); // #2 landed → not open
        let vm = build_org(&log, "humangr", "humangr/hugit");
        assert_eq!(vm.name, "humangr");
        assert_eq!(vm.handle, "@humangr");
        assert_eq!(vm.avatar_letter, "H");
        // repos[0]: the one loaded repo
        assert_eq!(vm.repos[0].org, "humangr");
        assert_eq!(vm.repos[0].name, "hugit");
        assert_eq!(vm.repos[0].open_prs, 1, "only non-terminal PRs are open");
    }

    #[test]
    fn vm_round_trips() {
        let vm = build_org(&EventLog::new(), "hugit", "hugit");
        let j = serde_json::to_string(&vm).expect("serializes");
        let reparsed: OrgVm = serde_json::from_str(&j).expect("deserializes");
        assert_eq!(vm, reparsed);
    }

    // ── W-METENANT: the identity-scoped aggregating org view ──────────────────

    #[test]
    fn me_org_empty_repo_set_has_no_repo_rows() {
        let vm = build_me_org("humangr", &[]);
        assert_eq!(vm.name, "humangr");
        assert_eq!(vm.handle, "@humangr");
        assert!(vm.repos.is_empty(), "no caller repos → no repo rows");
        // Wire shape intact (empty repos vec round-trips).
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<OrgVm>(&j).unwrap());
    }

    #[test]
    fn me_org_aggregates_the_callers_repos() {
        let mut a = EventLog::new();
        open_pr(&mut a, "1", 1000);
        let mut b = EventLog::new();
        open_pr(&mut b, "2", 2000);
        land_pr(&mut b, "2", 2100); // beta has 0 open
        let vm = build_me_org(
            "humangr",
            &[
                ("humangr/alpha".to_string(), a),
                ("humangr/beta".to_string(), b),
            ],
        );
        assert_eq!(vm.name, "humangr");
        assert_eq!(vm.repos.len(), 2, "one row per caller repo");
        assert_eq!(vm.repos[0].name, "alpha");
        assert_eq!(vm.repos[0].open_prs, 1);
        assert_eq!(vm.repos[1].name, "beta");
        assert_eq!(vm.repos[1].open_prs, 0);
    }
}
