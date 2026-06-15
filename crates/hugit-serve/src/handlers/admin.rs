//! Admin control-plane reads (operator area): the audit timeline, the erasure
//! governance history, and the one-call overview.
//!
//! All three are PURE projections over the already-chain-verified `log` (the
//! caller owns load → `verify_chain` → 404/503). Real data or a documented empty
//! shape — never faked. The raw event payload is NEVER echoed; every surfaced
//! free-text field is scrubbed at this read boundary.

use std::collections::{BTreeMap, HashSet};

use hugit_http_contracts::admin::{
    AdminOverviewVm, AuditEntryVm, AuditVm, ErasureHistoryVm, ErasureRowVm,
};
use hugit_refstore::EventLog;
use serde_json::Value;

use crate::fmt::{humanize_age, scrub, sha_prefix, str_field};

/// Default page size for the audit read; capped at [`AUDIT_LIMIT_MAX`].
const AUDIT_LIMIT_DEFAULT: usize = 100;
const AUDIT_LIMIT_MAX: usize = 500;

const PR_OPENED: &str = "pr.opened";
const PR_QUEUED: &str = "pr.queued";
const PR_LANDED: &str = "pr.landed";
const POLICY_SET: &str = "policy.set";
const ERASURE_DECIDED: &str = "erasure.decided";

/// The safe identifying field of a record, per kind — an identifier (safe-shape
/// by the door) projected as the audit summary, ALWAYS scrubbed (belt-and-
/// suspenders). Never the raw free-text payload. Unknown kinds → empty.
fn audit_summary(kind: &str, payload: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload) else {
        return String::new();
    };
    let id = match kind {
        "pr.opened" | "pr.queued" | "pr.landed" | "pr.comment" | "pr.abandoned" => {
            str_field(&v, "pr_id").map(|x| format!("PR #{x}"))
        }
        "intent.landed" => str_field(&v, "intent_id").map(|x| format!("intent {x}")),
        "verdict.recorded" => str_field(&v, "verdict").map(|x| format!("veredito {x}")),
        "check.recorded" => str_field(&v, "name").map(|x| format!("check '{x}'")),
        "policy.set" => str_field(&v, "rule_id").map(|x| format!("regra '{x}'")),
        "erasure.decided" => str_field(&v, "erasure_id").map(|x| format!("erasure {x}")),
        "issue.transition" => str_field(&v, "issue_id").map(|x| format!("issue #{x}")),
        _ => None,
    };
    scrub(&id.unwrap_or_default())
}

/// The acting principal — the tail of the record's principal chain, scrubbed.
fn principal_of(chain: &[String]) -> String {
    match chain.last() {
        Some(p) if !p.is_empty() => scrub(p),
        _ => "—".to_string(),
    }
}

/// `GET /v1/repos/{repo}/audit?since=&limit=&kind=&principal=` → [`AuditVm`].
///
/// Forward pagination: rows with `seq >= since` (ascending), optionally filtered
/// by exact `kind` and/or a `principal` substring, capped at `limit`. `next_since`
/// is the cursor for the next page (`None` at the head).
pub fn build_audit(
    log: &EventLog,
    _repo: &str,
    since: u64,
    limit: usize,
    kind_filter: Option<&str>,
    principal_filter: Option<&str>,
) -> AuditVm {
    // limit == 0 means "unset" (the query param was absent) → the default page;
    // any explicit value is capped at the max.
    let limit = if limit == 0 {
        AUDIT_LIMIT_DEFAULT
    } else {
        limit.min(AUDIT_LIMIT_MAX)
    };
    let records = log.records();
    let head_seq = records.last().map(|r| r.seq).unwrap_or(0);

    let mut entries: Vec<AuditEntryVm> = Vec::new();
    let mut next_since: Option<u64> = None;

    for record in records.iter().filter(|r| r.seq >= since) {
        if let Some(k) = kind_filter
            && record.kind != k
        {
            continue;
        }
        let principal = principal_of(&record.principal_chain);
        if let Some(pf) = principal_filter
            && !principal.contains(pf)
        {
            continue;
        }
        // Page is full — there is at least one more matching row, so hand back a
        // cursor and stop (we do NOT include this row).
        if entries.len() >= limit {
            next_since = Some(record.seq);
            break;
        }
        entries.push(AuditEntryVm {
            seq: record.seq,
            kind: record.kind.clone(),
            principal,
            summary: audit_summary(&record.kind, &record.payload),
            age: humanize_age(record.recorded_at),
            recorded_at: record.recorded_at,
            hash_short: sha_prefix(&record.this_hash, 12),
        });
    }

    AuditVm {
        returned: entries.len(),
        entries,
        next_since,
        head_seq,
    }
}

/// `GET /v1/repos/{repo}/erasure` → [`ErasureHistoryVm`].
///
/// Latest decision per `erasure_id` (by seq), newest first. Surfaces BOTH
/// `approved` and `denied` (unlike `security`, which shows only latest-approved).
/// Execution is always `pending` — recording a decision never runs the CAS scrub
/// (X12 execution is the P2 seam).
pub fn build_erasure(log: &EventLog, _repo: &str) -> ErasureHistoryVm {
    // latest-per-id by seq
    let mut latest: BTreeMap<String, (String, String, u64, u64)> = BTreeMap::new();
    for record in log.records().iter().filter(|r| r.kind == ERASURE_DECIDED) {
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        let Some(id) = str_field(&v, "erasure_id") else {
            continue;
        };
        let state = str_field(&v, "state")
            .or_else(|| str_field(&v, "decision"))
            .unwrap_or_else(|| "denied".to_string());
        let by = principal_of(&record.principal_chain);
        latest.insert(id, (state, by, record.recorded_at, record.seq));
    }

    let mut entries: Vec<ErasureRowVm> = latest
        .into_iter()
        .map(|(id, (state, by, at, seq))| ErasureRowVm {
            erasure_id: scrub(&id),
            state,
            execution: "pending".to_string(),
            decided_by: by,
            age: humanize_age(at),
            seq,
        })
        .collect();
    // Newest first.
    entries.sort_by_key(|e| std::cmp::Reverse(e.seq));

    let approved_count = entries.iter().filter(|e| e.state == "approved").count();
    let denied_count = entries.iter().filter(|e| e.state != "approved").count();

    ErasureHistoryVm {
        entries,
        approved_count,
        denied_count,
        note: "execução de uma erasure aprovada é o seam P2 (CAS-scrub); aqui só a \
               decisão é registrada"
            .to_string(),
    }
}

/// `GET /v1/repos/{repo}/admin/overview` → [`AdminOverviewVm`].
///
/// The one-call operational snapshot, folded from the log. `attention_count`
/// reuses [`super::build_dashboard`] so the overview agrees with the dashboard by
/// construction. `policy_rules_active` counts operator-set enabled rules (the
/// house-rule defaults live in `/security`).
pub fn build_admin_overview(log: &EventLog, repo: &str) -> AdminOverviewVm {
    let records = log.records();

    let mut opened_prs: HashSet<String> = HashSet::new();
    let mut queued_prs: HashSet<String> = HashSet::new();
    let mut landed_prs: HashSet<String> = HashSet::new();
    let mut campaigns_open: HashSet<String> = HashSet::new();
    // pr_id → campaign (from pr.opened), to scope active campaigns to open PRs.
    let mut pr_campaign: BTreeMap<String, String> = BTreeMap::new();
    let mut policy_enabled: BTreeMap<String, bool> = BTreeMap::new();
    let mut erasure_ids: HashSet<String> = HashSet::new();

    for r in records {
        let v = serde_json::from_str::<Value>(&r.payload).ok();
        match r.kind.as_str() {
            PR_OPENED => {
                if let Some(v) = &v
                    && let Some(id) = str_field(v, "pr_id")
                {
                    if let Some(c) = str_field(v, "campaign") {
                        pr_campaign.insert(id.clone(), c);
                    }
                    opened_prs.insert(id);
                }
            }
            PR_QUEUED => {
                if let Some(v) = &v
                    && let Some(id) = str_field(v, "pr_id")
                {
                    queued_prs.insert(id);
                }
            }
            PR_LANDED => {
                if let Some(v) = &v
                    && let Some(id) = str_field(v, "pr_id")
                {
                    landed_prs.insert(id);
                }
            }
            POLICY_SET => {
                if let Some(v) = &v
                    && let Some(rule) = str_field(v, "rule_id")
                {
                    let enabled = v.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    policy_enabled.insert(rule, enabled); // latest-wins (seq order)
                }
            }
            ERASURE_DECIDED => {
                if let Some(v) = &v
                    && let Some(id) = str_field(v, "erasure_id")
                {
                    erasure_ids.insert(id);
                }
            }
            _ => {}
        }
    }

    // Active queue = queued and not yet landed; active campaigns = the campaigns
    // those still-active PRs belong to.
    let queue_depth = queued_prs.difference(&landed_prs).count();
    for pr in queued_prs.difference(&landed_prs) {
        if let Some(c) = pr_campaign.get(pr) {
            campaigns_open.insert(c.clone());
        }
    }

    let last_activity_age = records
        .last()
        .map(|r| humanize_age(r.recorded_at))
        .unwrap_or_else(|| "—".to_string());

    AdminOverviewVm {
        queue_depth,
        active_campaigns: campaigns_open.len(),
        attention_count: super::build_dashboard(log, repo).attention_count,
        total_prs: opened_prs.len(),
        policy_rules_active: policy_enabled.values().filter(|&&e| e).count(),
        erasure_decisions: erasure_ids.len(),
        log_depth: records.len() as u64,
        last_activity_age,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    fn append(log: &mut EventLog, kind: &str, principal: &str, payload: Value, at: u64) {
        let body = hugit_refstore::canonical_json(&payload.to_string())
            .unwrap_or_else(|| payload.to_string());
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Push,
            kind,
            vec![principal.to_string()],
            body,
            at,
        )
        .expect("append");
    }

    #[test]
    fn audit_paginates_forward_with_cursor() {
        let mut log = EventLog::new();
        for i in 0..5u64 {
            append(
                &mut log,
                "pr.opened",
                "o",
                serde_json::json!({"pr_id": i}),
                1000 + i,
            );
        }
        // First page of 2 → cursor at seq 2, two rows (seq 0,1).
        let p1 = build_audit(&log, "r", 0, 2, None, None);
        assert_eq!(p1.returned, 2);
        assert_eq!(p1.entries[0].seq, 0);
        assert_eq!(p1.next_since, Some(2));
        assert_eq!(p1.head_seq, 4);
        // Continue from the cursor.
        let p2 = build_audit(&log, "r", p1.next_since.unwrap(), 2, None, None);
        assert_eq!(p2.entries[0].seq, 2);
        // Last page exhausts → no cursor.
        let p3 = build_audit(&log, "r", 4, 2, None, None);
        assert_eq!(p3.returned, 1);
        assert_eq!(p3.next_since, None);
    }

    #[test]
    fn audit_filters_by_kind_and_never_echoes_payload() {
        let mut log = EventLog::new();
        append(
            &mut log,
            "pr.opened",
            "o",
            serde_json::json!({"pr_id": "7"}),
            1000,
        );
        append(
            &mut log,
            "policy.set",
            "o",
            serde_json::json!({"rule_id": "dco", "enabled": true, "secret_note": "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}),
            2000,
        );
        let only_policy = build_audit(&log, "r", 0, 100, Some("policy.set"), None);
        assert_eq!(only_policy.returned, 1);
        assert_eq!(only_policy.entries[0].kind, "policy.set");
        assert_eq!(only_policy.entries[0].summary, "regra 'dco'");
        // The raw payload (which carried a secret) is NEVER in the projection.
        let blob = serde_json::to_string(&only_policy).unwrap();
        assert!(
            !blob.contains("ghp_"),
            "audit must not echo the raw payload"
        );
        assert!(!blob.contains("secret_note"));
        assert!(!only_policy.entries[0].hash_short.is_empty());
    }

    #[test]
    fn erasure_history_surfaces_approved_and_denied_latest_per_id() {
        let mut log = EventLog::new();
        append(
            &mut log,
            "erasure.decided",
            "o",
            serde_json::json!({"erasure_id": "er-1", "state": "denied"}),
            1000,
        );
        append(
            &mut log,
            "erasure.decided",
            "o",
            serde_json::json!({"erasure_id": "er-1", "state": "approved"}),
            2000,
        ); // latest wins
        append(
            &mut log,
            "erasure.decided",
            "o",
            serde_json::json!({"erasure_id": "er-2", "state": "denied"}),
            3000,
        );
        let vm = build_erasure(&log, "r");
        assert_eq!(vm.entries.len(), 2, "latest-per-id");
        assert_eq!(vm.approved_count, 1);
        assert_eq!(vm.denied_count, 1);
        assert!(
            vm.entries.iter().all(|e| e.execution == "pending"),
            "never executed locally"
        );
        // Newest first.
        assert_eq!(vm.entries[0].erasure_id, "er-2");
    }

    #[test]
    fn overview_folds_queue_campaigns_and_counts() {
        let mut log = EventLog::new();
        // PR 1 (auth) opened + queued; PR 2 (billing) opened + queued + landed.
        append(
            &mut log,
            "pr.opened",
            "o",
            serde_json::json!({"pr_id": "1", "campaign": "auth"}),
            1000,
        );
        append(
            &mut log,
            "pr.opened",
            "o",
            serde_json::json!({"pr_id": "2", "campaign": "billing"}),
            1100,
        );
        append(
            &mut log,
            "pr.queued",
            "o",
            serde_json::json!({"pr_id": "1", "item_id": "i1", "order_index": 0}),
            1200,
        );
        append(
            &mut log,
            "pr.queued",
            "o",
            serde_json::json!({"pr_id": "2", "item_id": "i2", "order_index": 1}),
            1300,
        );
        append(
            &mut log,
            "pr.landed",
            "o",
            serde_json::json!({"pr_id": "2"}),
            1400,
        );
        append(
            &mut log,
            "policy.set",
            "o",
            serde_json::json!({"rule_id": "dco", "enabled": true}),
            1500,
        );
        append(
            &mut log,
            "policy.set",
            "o",
            serde_json::json!({"rule_id": "changelog", "enabled": false}),
            1600,
        );
        append(
            &mut log,
            "erasure.decided",
            "o",
            serde_json::json!({"erasure_id": "er-1", "state": "approved"}),
            1700,
        );

        let vm = build_admin_overview(&log, "r");
        assert_eq!(vm.total_prs, 2);
        assert_eq!(vm.queue_depth, 1, "PR 1 queued+unlanded; PR 2 landed → out");
        assert_eq!(vm.active_campaigns, 1, "only auth (PR1) still active");
        assert_eq!(vm.policy_rules_active, 1, "dco enabled, changelog disabled");
        assert_eq!(vm.erasure_decisions, 1);
        assert_eq!(vm.log_depth, 8);
        assert_ne!(vm.last_activity_age, "—");
    }

    #[test]
    fn overview_empty_log_is_honest_zeroes() {
        let vm = build_admin_overview(&EventLog::new(), "r");
        assert_eq!(vm.queue_depth, 0);
        assert_eq!(vm.total_prs, 0);
        assert_eq!(vm.log_depth, 0);
        assert_eq!(vm.last_activity_age, "—");
    }
}
