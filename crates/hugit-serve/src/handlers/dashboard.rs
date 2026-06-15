//! `GET /v1/me/dashboard` → [`DashboardVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. The dashboard is
//! identity-scoped at the route, but its DATA is one repo's verified log (the
//! `/v1/me` principal→repo resolution is the P2 identity seam — NOT this
//! handler's concern); so it mirrors `build_security`/`build_landing`/`build_home`
//! exactly: `(log, repo)` in, the frozen VM out.
//!
//! REAL backbone:
//! - `repos[0].org` / `repos[0].name` — DERIVED (structural) by splitting the
//!   caller-supplied `repo` slug on the first `/` (same posture as
//!   `repo_chrome`/`landing` `repo.to_string()`). Not log free-text → NOT
//!   scrubbed. No `/` → `org=""`, `name=<full slug>`.
//! - `repos[0].open_prs` — REAL count of non-terminal PRs, folded latest-per-id
//!   off `pr.opened` and classified by the `pr_state` precedence
//!   (landed ≻ abandoned ≻ queued ≻ proposed); the same `open_count` `landing`
//!   computes.
//! - `repos[0].last_activity` — PRESENTATION: `humanize_age` of the last record's
//!   `recorded_at` (mirrors `home`'s `updated_ago`). Empty log → "".
//! - `attention_count` — REAL: PRs that need the principal = abandoned (Blocked)
//!   PRs + open PRs carrying an APPROVE `verdict.recorded` that have NOT landed.
//! - `inbox` — REAL (repo-real subset): one row per actionable PR signal — an
//!   APPROVE-not-landed PR → a "pronto pra land" row; an abandoned PR → a
//!   "bloqueado" row. Capped at `PR_CARDS_CAP`.
//! - `inbox_pending_total` — REAL: `inbox.len()`.
//!
//! HONEST-DEFAULT (no local engine seam — never fabricated):
//! - `repos[0].main_green` / `repos[0].status` — `false` / "" (NO local main-CI
//!   status seam — `landing` documents this exact stub).
//! - `repos[0].stack` / `repos[0].visibility` — "" (no crate-count/language seam;
//!   tenant provisioning is the P2 seam, per `landing`/`repo_chrome`).
//! - `github_app.saved_usd` / `saved_ci_minutes` — `0.0` / `0` (no live-AC
//!   savings seam in this read).
//! - cross-repo rows — none emitted (a single-repo log yields exactly the one
//!   real row; sibling rows are never fabricated).
//!
//! Every echoed free-text field passes `crate::fmt::scrub` (the read-boundary
//! redaction spine); structural ids / derived labels are not scrubbed.

use crate::fmt::{PR_CARDS_CAP, humanize_age, scrub};
use hugit_cli::pr::{
    OpenedPr, PR_ABANDONED_KIND, PR_LANDED_KIND, PR_OPENED_KIND, all_pr_queued, find_pr_opened,
};
use hugit_cli::verdict::VERDICT_RECORDED_KIND;
use hugit_contracts::VerdictObject;
use hugit_contracts::verdict_object::Verdict;
use hugit_http_contracts::common::{DashboardRepoVm, GithubAppStripVm};
use hugit_http_contracts::dashboard::{DashboardVm, InboxRowVm};
use hugit_refstore::EventLog;
use serde_json::Value;

/// The lifecycle state of a PR projected off raw record kinds — mirrors the
/// porcelain/`landing` precedence (landed ≻ abandoned ≻ queued ≻ proposed).
#[derive(PartialEq, Eq)]
enum State {
    Landed,
    Abandoned,
    Queued,
    Proposed,
}

/// Classify a PR by its terminal/queue signals (same authority `landing` reads).
fn pr_state(log: &EventLog, pr_id: &str) -> State {
    if record_names_pr(log, PR_LANDED_KIND, pr_id) {
        State::Landed
    } else if record_names_pr(log, PR_ABANDONED_KIND, pr_id) {
        State::Abandoned
    } else if all_pr_queued(log).iter().any(|q| q.pr_id == pr_id) {
        State::Queued
    } else {
        State::Proposed
    }
}

/// Whether a record of `kind` names `pr_id` in its `pr_id` payload field.
fn record_names_pr(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// Whether `pr_id` carries an APPROVE `verdict.recorded` (a `VerdictObject` whose
/// `intent` is the PR id) — the same projection `review`/`landing` already do.
fn has_approve_verdict(log: &EventLog, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .any(|vo| vo.intent == pr_id && vo.verdict == Verdict::Approve)
}

/// The APPROVE verdict's `lens` for `pr_id`, if any (free-text → caller scrubs).
fn approve_lens(log: &EventLog, pr_id: &str) -> Option<String> {
    log.records()
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .find(|vo| vo.intent == pr_id && vo.verdict == Verdict::Approve)
        .map(|vo| vo.lens)
}

/// Project the latest `pr.opened` per id, in first-seen (open seq) order — the
/// same fold `landing::ordered_open_prs` does (latest-per-id, dedup by pr_id).
fn ordered_open_prs(log: &EventLog) -> Vec<OpenedPr> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut ordered: Vec<OpenedPr> = Vec::new();
    for r in log.records().iter().filter(|r| r.kind == PR_OPENED_KIND) {
        if let Ok(v) = serde_json::from_str::<Value>(&r.payload)
            && let Some(id) = v.get("pr_id").and_then(Value::as_str)
            && seen.insert(id.to_string())
            && let Some(latest) = find_pr_opened(log, id)
        {
            ordered.push(latest);
        }
    }
    ordered
}

/// Split a caller-supplied `repo` slug into `(org, name)` on the first `/`.
///
/// Structural (caller-supplied, not log free-text) → NOT scrubbed, same posture
/// as `repo_chrome`/`landing` `repo.to_string()`. No `/` → `("", <full slug>)`.
fn split_repo(repo: &str) -> (String, String) {
    match repo.split_once('/') {
        Some((org, name)) => (org.to_string(), name.to_string()),
        None => (String::new(), repo.to_string()),
    }
}

/// Build the dashboard view-model from a verified event log.
///
/// The `log` is ALREADY chain-verified by the caller — do NOT re-load or
/// re-verify. REAL fields come from the PR / verdict projection; honest-default
/// fields have no local engine source (per the data contract). Nothing is faked.
pub fn build_dashboard(log: &EventLog, repo: &str) -> DashboardVm {
    let (org, name) = split_repo(repo);
    let prs = ordered_open_prs(log);

    let mut open_prs: u32 = 0;
    let mut attention_count: usize = 0;
    let mut inbox: Vec<InboxRowVm> = Vec::new();

    for opened in prs.iter() {
        let state = pr_state(log, &opened.pr_id);

        // REAL open_prs: non-terminal states (proposed/queued) count as open; an
        // abandoned PR is terminal-not-landed and is NOT "open".
        if matches!(state, State::Proposed | State::Queued) {
            open_prs += 1;
        }

        let approve_not_landed = state != State::Landed && has_approve_verdict(log, &opened.pr_id);

        // REAL attention: abandoned (Blocked) PRs + APPROVE-not-landed PRs.
        if state == State::Abandoned || approve_not_landed {
            attention_count += 1;
        }

        // REAL inbox rows (capped). `pr_id` is payload-derived free text (the
        // user-supplied `--pr <id>`, NOT a system number), so the composed
        // `context` is scrubbed at the read boundary; the description echoes the
        // verdict lens (log free-text) → also scrubbed.
        if inbox.len() < PR_CARDS_CAP {
            if approve_not_landed {
                let lens = approve_lens(log, &opened.pr_id).unwrap_or_default();
                let description = if lens.is_empty() {
                    "veredito APPROVE · pronto pra land".to_string()
                } else {
                    scrub(&format!("veredito APPROVE · lens {lens}"))
                };
                inbox.push(InboxRowVm {
                    signal_class: "green".to_string(),
                    context: scrub(&format!("{repo} · #{}", opened.pr_id)),
                    title: "PR pronto pra land".to_string(),
                    description,
                    action_label: "Aprovar e land →".to_string(),
                });
            } else if state == State::Abandoned {
                inbox.push(InboxRowVm {
                    signal_class: "err".to_string(),
                    context: scrub(&format!("{repo} · #{}", opened.pr_id)),
                    title: "PR bloqueado".to_string(),
                    description: "PR abandonado — precisa de atenção".to_string(),
                    action_label: "Revisar →".to_string(),
                });
            }
        }
    }

    // PRESENTATION: last_activity from the last record's recorded_at; "" on empty.
    let last_activity = log
        .records()
        .last()
        .map(|r| humanize_age(r.recorded_at))
        .unwrap_or_default();

    let repo_row = DashboardRepoVm {
        org,                       // DERIVED (structural)
        name,                      // DERIVED (structural)
        main_green: false,         // HONEST-DEFAULT — no local main-CI seam
        status: String::new(),     // HONEST-DEFAULT — no local main-CI seam
        stack: String::new(),      // HONEST-DEFAULT — no crate-count/language seam
        visibility: String::new(), // HONEST-DEFAULT — tenant provisioning is P2
        open_prs,                  // REAL
        last_activity,             // PRESENTATION
    };

    DashboardVm {
        repos: vec![repo_row], // exactly the one real repo (never fabricate siblings)
        github_app: GithubAppStripVm {
            saved_usd: 0.0,      // HONEST-DEFAULT — no live-AC savings seam
            saved_ci_minutes: 0, // HONEST-DEFAULT — no live-AC savings seam
        },
        inbox_pending_total: inbox.len(), // REAL
        inbox,                            // REAL (repo-real subset)
        attention_count,                  // REAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn append(log: &mut EventLog, kind: &str, payload: serde_json::Value, at: u64) {
        let p = hugit_refstore::canonical_json(&payload.to_string())
            .unwrap_or_else(|| payload.to_string());
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            kind,
            vec!["o".into()],
            p,
            at,
        )
        .expect("append");
    }

    fn open_pr(log: &mut EventLog, pr_id: &str, intents: &[&str], at: u64) {
        append(
            log,
            PR_OPENED_KIND,
            serde_json::json!({"author_kind":"orchestrator","campaign":"","intent_ids":intents,"pr_id":pr_id}),
            at,
        );
    }

    fn land(log: &mut EventLog, pr_id: &str, at: u64) {
        append(log, PR_LANDED_KIND, serde_json::json!({"pr_id":pr_id}), at);
    }

    fn abandon(log: &mut EventLog, pr_id: &str, at: u64) {
        append(
            log,
            PR_ABANDONED_KIND,
            serde_json::json!({"pr_id":pr_id}),
            at,
        );
    }

    fn verdict(
        log: &mut EventLog,
        intent: &str,
        lens: &str,
        outcome: &str,
        claims: serde_json::Value,
        at: u64,
    ) {
        append(
            log,
            VERDICT_RECORDED_KIND,
            serde_json::json!({
                "intent":intent,
                "tree_hash":"",
                "lens":lens,
                "model":"m",
                "prompt_digest":"0".repeat(64),
                "verdict":outcome,
                "claims_checked":claims,
                "evidence_refs":[]
            }),
            at,
        );
    }

    #[test]
    fn empty_log_honest_defaults() {
        let vm = build_dashboard(&EventLog::new(), "humangr/hugit");
        assert_eq!(vm.repos.len(), 1);
        let r = &vm.repos[0];
        assert_eq!(r.org, "humangr");
        assert_eq!(r.name, "hugit");
        assert_eq!(r.open_prs, 0);
        assert!(!r.main_green);
        assert_eq!(r.status, "");
        assert_eq!(r.stack, "");
        assert_eq!(r.visibility, "");
        assert_eq!(r.last_activity, "");
        assert_eq!(vm.github_app.saved_usd, 0.0);
        assert_eq!(vm.github_app.saved_ci_minutes, 0);
        assert_eq!(vm.inbox_pending_total, 0);
        assert!(vm.inbox.is_empty());
        assert_eq!(vm.attention_count, 0);
    }

    #[test]
    fn slug_without_slash_is_honest() {
        let vm = build_dashboard(&EventLog::new(), "hugit");
        assert_eq!(vm.repos[0].org, "");
        assert_eq!(vm.repos[0].name, "hugit");
    }

    #[test]
    fn real_projection_open_prs_and_inbox() {
        let mut log = EventLog::new();
        // #1 open, no verdict → open, no inbox row.
        open_pr(&mut log, "1", &["i-1"], 1000);
        // #2 open + APPROVE, not landed → open + attention + inbox "pronto".
        open_pr(&mut log, "2", &["i-2"], 1100);
        verdict(
            &mut log,
            "2",
            "correctness",
            "approve",
            serde_json::json!([]),
            1200,
        );
        // #3 open + APPROVE but LANDED → not open, not attention, no inbox row.
        open_pr(&mut log, "3", &["i-3"], 1300);
        verdict(
            &mut log,
            "3",
            "correctness",
            "approve",
            serde_json::json!([]),
            1400,
        );
        land(&mut log, "3", 1500);
        // #4 abandoned → not open, attention + inbox "bloqueado".
        open_pr(&mut log, "4", &["i-4"], 1600);
        abandon(&mut log, "4", 1700);

        let vm = build_dashboard(&log, "humangr/hugit");
        // open = #1 + #2 (proposed); #3 landed, #4 abandoned excluded.
        assert_eq!(vm.repos[0].open_prs, 2);
        // attention = #2 (approve-not-landed) + #4 (abandoned).
        assert_eq!(vm.attention_count, 2);
        // inbox = #2 pronto + #4 bloqueado.
        assert_eq!(vm.inbox.len(), 2);
        assert_eq!(vm.inbox_pending_total, 2);
        let pronto = vm
            .inbox
            .iter()
            .find(|r| r.title == "PR pronto pra land")
            .expect("pronto");
        assert_eq!(pronto.signal_class, "green");
        assert_eq!(pronto.context, "humangr/hugit · #2");
        let bloq = vm
            .inbox
            .iter()
            .find(|r| r.title == "PR bloqueado")
            .expect("bloqueado");
        assert_eq!(bloq.signal_class, "err");
        assert_eq!(bloq.context, "humangr/hugit · #4");
        // last_activity is non-empty (a record exists).
        assert!(!vm.repos[0].last_activity.is_empty());
    }

    #[test]
    fn secret_in_verdict_lens_is_redacted() {
        let mut log = EventLog::new();
        open_pr(&mut log, "7", &["i"], 1000);
        verdict(
            &mut log,
            "7",
            &format!("lens-{PAT}"),
            "approve",
            serde_json::json!([]),
            1100,
        );
        let j = serde_json::to_string(&build_dashboard(&log, "humangr/hugit")).unwrap();
        assert!(!j.contains(PAT), "PAT must not reach the wire");
        assert!(j.contains("[REDACTED]"), "secret-shaped lens must redact");
    }

    #[test]
    fn secret_pr_id_redacted_in_context() {
        // pr_id is the user-supplied `--pr <id>` (payload free text), so a
        // secret-shaped pr_id in the inbox `context` must be scrubbed.
        let mut log = EventLog::new();
        open_pr(&mut log, PAT, &["i"], 1000);
        verdict(
            &mut log,
            PAT,
            "correctness",
            "approve",
            serde_json::json!([]),
            1100,
        );
        let j = serde_json::to_string(&build_dashboard(&log, "humangr/hugit")).unwrap();
        assert!(
            !j.contains(PAT),
            "secret-shaped pr_id must not reach the wire"
        );
    }

    #[test]
    fn vm_round_trips() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", &["i-1"], 1000);
        verdict(
            &mut log,
            "1",
            "correctness",
            "approve",
            serde_json::json!([]),
            1100,
        );
        let vm = build_dashboard(&log, "humangr/hugit");
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<DashboardVm>(&j).unwrap());
    }
}
