//! `GET /v1/repos/{repo}/new-pr` → [`NewPrVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL backbone:
//! - `campaigns` — REAL: folded from `campaign.opened` records (first-seen,
//!   de-duped by id), scrubbed at the read boundary — mirrors `landing.rs`.
//! - `commits` — REAL: `project_machine` rows (same fold as `commits.rs`),
//!   capped at `COMMITS_CAP` (newest rows kept, applied AFTER reverse), scrubbed.
//! - `checks_ok` — REAL: `check.recorded` exit==0 index (mirrors `commits.rs`).
//! - `base` / `head` — REAL: `replay()` RefState — prefer `main`>`master`>first
//!   plain branch for base; first `intent/*` branch for head (else `""`).
//! - `policy_note` — REAL (latest-wins `policy.set` fold): non-empty when at
//!   least one house rule is (or defaults to) enabled; honest static phrase.
//!
//! HONEST-DEFAULT (static content, no seam — never faked):
//! `mergeable_note`, `branch_title`, `branch_description`, `reviewers_note`,
//! `labels_note`, `checks_note`, `ask_placeholder`, `examples`, `steps`,
//! `executors`. Every free-text field echoed from the log passes `crate::fmt::scrub`.

use std::collections::{BTreeSet, HashSet};

use hugit_cli::checks::CHECK_RECORDED_KIND;
use hugit_cli::pr::CAMPAIGN_OPENED_KIND;
use hugit_http_contracts::commits::CommitRowVm;
use hugit_http_contracts::common::CampaignChipVm;
use hugit_http_contracts::new_pr::{NewPrStepVm, NewPrVm};
use hugit_refstore::EventLog;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};
use hugit_refstore::replay::replay;
use serde_json::Value;

use crate::fmt::{COMMITS_CAP, classify_avatar, humanize_age, scrub, sha_prefix};

/// Sha prefix length for commit rows (structural, not scrubbed).
const SHA_PREFIX_LEN: usize = 6;

/// `policy.set` kind — local constant so this handler stays decoupled.
const POLICY_SET_KIND: &str = "policy.set";

/// House rule ids whose presence signals an active policy. Mirrors
/// `repo_settings.rs::HOUSE_RULES` — duplicated locally for decoupling.
const HOUSE_RULE_IDS: &[&str] = &["dco", "changelog", "secrets"];

/// Build the new-PR view-model from a verified event log.
///
/// The `log` is ALREADY chain-verified by the caller — do NOT re-load or
/// re-verify. REAL fields are projected from `campaign.opened` / `project_machine`
/// / `check.recorded` / `replay` / `policy.set`; honest-default fields are
/// static strings (no seam), never faked.
pub fn build_new_pr(log: &EventLog, repo: &str) -> NewPrVm {
    // ── REAL: base / head branch names via replay ───────────────────────────
    // replay() re-verifies the in-memory chain then folds the ref state.
    // On an empty log this yields an empty RefState — base and head are "".
    let ref_state = replay(log).unwrap_or_default();

    let all_heads: Vec<(String, String)> = ref_state
        .iter()
        .filter(|(name, _)| name.starts_with("refs/heads/"))
        .map(|(name, sha)| {
            (
                name.strip_prefix("refs/heads/").unwrap_or(name).to_string(),
                sha.to_string(),
            )
        })
        .collect();

    let plain_branches: Vec<&str> = all_heads
        .iter()
        .filter(|(name, _)| !name.starts_with("intent/"))
        .map(|(name, _)| name.as_str())
        .collect();

    // base: the default branch (main > master > first plain > ""), scrubbed.
    // Branch names are free-text ref components from the log and must pass
    // the redaction seam before being echoed into the view-model.
    let base = scrub(&pick_default_branch(&plain_branches));

    // head: the first intent/* branch, else "" (no candidate to compare yet).
    // Scrubbed for the same reason as base.
    let head = all_heads
        .iter()
        .find(|(name, _)| name.starts_with("intent/"))
        .map(|(name, _)| scrub(name))
        .unwrap_or_default();

    // ── REAL: checks_ok index (mirrors commits.rs / branches.rs) ───────────
    let checks_ok_shas = build_checks_ok_set(log);

    // ── REAL: commit rows via project_machine (mirrors commits.rs) ──────────
    // project_machine folds the same in-memory records; projection does not
    // re-verify the chain. On an empty log this yields no rows.
    let machine = project_machine(log).unwrap_or_default();
    let mut commit_rows: Vec<CommitRowVm> = Vec::new();
    for row in machine.rows() {
        let seq = row.seq();
        let recorded_at_ms = log
            .records()
            .get(seq as usize)
            .map(|r| r.recorded_at)
            .unwrap_or(0);
        let cr = match row {
            ProjectionRow::Intent(c) => {
                let author = extract_author(log, seq);
                let avatar_class = classify_avatar(&author);
                let sha = sha_prefix(&c.target, SHA_PREFIX_LEN);
                let checks_ok = checks_ok_shas.contains(c.target.as_str())
                    || checks_ok_shas.contains(sha.as_str());
                CommitRowVm {
                    // P0: message + author are free text echoed from the log →
                    // MUST pass through the redaction seam. sha/intent_id/age
                    // are structural (content-addresses), so they are NOT scrubbed.
                    message: scrub(&c.message),
                    author: scrub(&author),
                    avatar_class,
                    age: humanize_age(recorded_at_ms),
                    intent_id: Some(c.intent_id.clone()),
                    sha,
                    checks_ok,
                }
            }
            ProjectionRow::ExternalChange { kind, target, .. } => {
                let sha = target
                    .as_deref()
                    .map(|t| sha_prefix(t, SHA_PREFIX_LEN))
                    .unwrap_or_default();
                let author = extract_author(log, seq);
                let avatar_class = classify_avatar(&author);
                let checks_ok = if sha.is_empty() {
                    false
                } else {
                    checks_ok_shas.contains(sha.as_str())
                };
                CommitRowVm {
                    // P0: `kind` is the displayed free-text message for an
                    // external change; `author` is free text — both pass scrub.
                    message: scrub(kind),
                    author: scrub(&author),
                    avatar_class,
                    age: humanize_age(recorded_at_ms),
                    intent_id: None,
                    sha,
                    checks_ok,
                }
            }
        };
        commit_rows.push(cr);
    }
    // Most-recent-first (machine rows are ascending log order); cap AFTER
    // reversing so the newest COMMITS_CAP entries are kept, not the oldest.
    commit_rows.reverse();
    commit_rows.truncate(COMMITS_CAP);

    // ── REAL: campaign chips from campaign.opened (mirrors landing.rs) ──────
    let campaigns = campaign_chips(log);

    // ── REAL: policy_note from latest-wins policy.set fold ──────────────────
    // If at least one house rule is (or defaults to) enabled, the gate applies.
    // We surface a static canned phrase — the detail lives in settings.
    let policy_note = latest_policy_note(log);

    // ── HONEST-DEFAULT: fields with no local engine seam ────────────────────
    // These are static content strings, never fabricated from non-existent data.
    // Labelled "HONEST-DEFAULT" inline so reviewers can trace each one.

    let commits_note = if commit_rows.is_empty() {
        String::new() // HONEST-DEFAULT — no commits yet on this branch
    } else {
        format!("{} commit(s)", commit_rows.len()) // REAL count, canned phrase
    };

    NewPrVm {
        repo: repo.to_string(),
        base,
        head,
        mergeable_note: String::new(), // HONEST-DEFAULT — no graph-walk/merge-base seam
        branch_title: String::new(),   // HONEST-DEFAULT — no inferred title seam
        branch_description: String::new(), // HONEST-DEFAULT — no inferred description seam
        commits: commit_rows,
        commits_note,
        reviewers_note: String::new(), // HONEST-DEFAULT — no reviewer-assignment seam
        labels_note: String::new(),    // HONEST-DEFAULT — no label seam
        checks_note: String::new(),    // HONEST-DEFAULT — no pre-run checks-summary seam
        campaigns,
        ask_placeholder: String::new(), // HONEST-DEFAULT — no personalised prompt seam
        examples: vec![],               // HONEST-DEFAULT — no examples seam
        steps: dispatch_steps(),        // HONEST-DEFAULT static strip (no process seam)
        executors: executor_options(),  // HONEST-DEFAULT static list (no fleet seam)
        policy_note,
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Pick the default branch from a list of plain branch names.
/// Preference: "main" > "master" > first entry > "".
fn pick_default_branch(plain: &[&str]) -> String {
    if plain.contains(&"main") {
        return "main".to_string();
    }
    if plain.contains(&"master") {
        return "master".to_string();
    }
    plain.first().map(|s| s.to_string()).unwrap_or_default()
}

/// Project campaign chips from `campaign.opened` (REAL, first-seen, de-duped).
/// Mirrors `landing.rs::campaign_chips` exactly — keep the two in sync when the
/// fold logic changes.
fn campaign_chips(log: &EventLog) -> Vec<CampaignChipVm> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut chips: Vec<CampaignChipVm> = Vec::new();
    for r in log
        .records()
        .iter()
        .filter(|r| r.kind == CAMPAIGN_OPENED_KIND)
    {
        if let Ok(v) = serde_json::from_str::<Value>(&r.payload)
            && let Some(id) = v.get("campaign").and_then(Value::as_str)
            && seen.insert(id.to_string())
        {
            // P1 LEAK FIX: campaign id is raw free-text from the log — scrub
            // before id/label/display_label so a secret-shaped id never echoes.
            let safe = scrub(id);
            chips.push(CampaignChipVm {
                id: safe.clone(),
                label: safe.clone(), // REAL id (scrubbed); no separate label seam
                color_class: String::new(), // STUB — no kit color seam on the log
                display_label: safe,
            });
        }
    }
    chips
}

/// Latest-wins fold of `policy.set` records → derive a canned `policy_note`.
/// REAL: if any house rule ends up enabled (default=true when no override),
/// the forge gate applies. Returns "" when all house rules are explicitly
/// disabled (honest — gate is truly off).
fn latest_policy_note(log: &EventLog) -> String {
    // Fold: rule_id → enabled (last-seen wins, mirroring repo_settings.rs).
    let mut map: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for record in log.records().iter().filter(|r| r.kind == POLICY_SET_KIND) {
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        let Some(rule_id) = v.get("rule_id").and_then(Value::as_str) else {
            continue;
        };
        let enabled = v.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        map.insert(rule_id.to_string(), enabled);
    }
    // At least one house rule enabled (default=true when absent) → gate applies.
    let any_enabled = HOUSE_RULE_IDS
        .iter()
        .any(|id| map.get(*id).copied().unwrap_or(true));
    if any_enabled {
        "a política do repo aplica — verificação não é opcional".to_string()
    } else {
        String::new() // HONEST-DEFAULT — all house rules explicitly disabled
    }
}

/// Build the shas (full + 6-char prefix) for commits with ≥1 successful
/// `check.recorded`. Mirrors `commits.rs::build_checks_ok_set`.
fn build_checks_ok_set(log: &EventLog) -> HashSet<String> {
    let mut set = HashSet::new();
    for record in log.records() {
        if record.kind != CHECK_RECORDED_KIND {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        if v.get("exit").and_then(Value::as_i64) != Some(0) {
            continue;
        }
        if let Some(target) = v.get("target").and_then(Value::as_str) {
            let prefix = sha_prefix(target, 6);
            if !prefix.is_empty() {
                set.insert(prefix);
            }
            if target.len() > 6 {
                set.insert(target.to_string());
            }
        }
    }
    set
}

/// Extract the "author" from a record's `principal_chain`. Mirrors `commits.rs`.
fn extract_author(log: &EventLog, seq: u64) -> String {
    let Some(record) = log.records().get(seq as usize) else {
        return String::new();
    };
    let chain = &record.principal_chain;
    for entry in chain {
        if entry.contains("agent:") || entry.contains("orchestrator:") {
            return entry.clone();
        }
    }
    chain.first().cloned().unwrap_or_default()
}

/// The "O que acontece quando você despacha" strip — static, honest (no seam).
/// HONEST-DEFAULT: the process reflects the engine's real dispatch flow; the
/// text is canned (no per-repo or per-PR customisation seam exists today).
fn dispatch_steps() -> Vec<NewPrStepVm> {
    vec![
        NewPrStepVm {
            title: "O orquestrador planeja — e abre o PR".to_string(),
            detail: "decompõe o pedido em intents atômicos".to_string(),
            timing: "agora".to_string(),
        },
        NewPrStepVm {
            title: "A frota executa as intents".to_string(),
            detail: "cada intent é atômica e verificada antes de pousar".to_string(),
            timing: "paralelo".to_string(),
        },
        NewPrStepVm {
            title: "Verificação adversarial".to_string(),
            detail: "painel independente audita o resultado antes do merge".to_string(),
            timing: "antes do merge".to_string(),
        },
        NewPrStepVm {
            title: "Pouso memoizado".to_string(),
            detail: "só a novidade executa — cache de conteúdo, custo flat".to_string(),
            timing: "na fila".to_string(),
        },
    ]
}

/// Executor selector options — static, honest (no fleet-config seam today).
/// HONEST-DEFAULT: the models are the real production fleet options; the notes
/// are canned (no per-org or per-repo override seam exists today).
fn executor_options() -> Vec<(String, String)> {
    vec![
        ("fleet · opus-4.8".to_string(), "padrão".to_string()),
        ("fleet · sonnet-4.6".to_string(), "econômico".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    /// Append a record via the D14-guarded path (campaign.opened, policy.set).
    fn append_authorized(log: &mut EventLog, kind: &str, payload: Value, at: u64) {
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            kind,
            vec!["o".into()],
            payload.to_string(),
            at,
        )
        .expect("append_authorized");
    }

    /// Append a `campaign.opened` record.
    fn push_campaign(log: &mut EventLog, name: &str, at: u64) {
        append_authorized(
            log,
            CAMPAIGN_OPENED_KIND,
            serde_json::json!({"campaign": name, "owner": "o", "charter": "c"}),
            at,
        );
    }

    /// Append a `policy.set` record.
    fn push_policy(log: &mut EventLog, rule: &str, enabled: bool, at: u64) {
        append_authorized(
            log,
            POLICY_SET_KIND,
            serde_json::json!({"rule_id": rule, "enabled": enabled}),
            at,
        );
    }

    /// Append a `ref.update` record via `append_for_test` (the correct raw-kind
    /// path for test-only appends: computes valid prev_hash / this_hash
    /// automatically so `replay()` accepts the chain without unwrap_or_default
    /// swallowing a chain error).
    fn push_ref_update(log: &mut EventLog, branch: &str, sha: &str, at: u64) {
        log.append_for_test(
            "ref.update",
            vec!["o".into()],
            serde_json::json!({"ref": format!("refs/heads/{branch}"), "target": sha}).to_string(),
            at,
        );
    }

    // ── empty-log test ───────────────────────────────────────────────────────

    #[test]
    fn empty_log_yields_honest_defaults() {
        let log = EventLog::new();
        let vm = build_new_pr(&log, "hugit");
        assert_eq!(vm.repo, "hugit");
        // REAL fields — empty log → no data.
        assert_eq!(vm.base, "");
        assert_eq!(vm.head, "");
        assert!(vm.commits.is_empty());
        assert!(vm.campaigns.is_empty());
        // HONEST-DEFAULT static strips always present.
        assert!(!vm.steps.is_empty(), "dispatch steps always present");
        assert!(!vm.executors.is_empty(), "executor options always present");
        // policy_note: all house rules default to enabled → gate note present.
        assert!(
            !vm.policy_note.is_empty(),
            "policy_note non-empty when house rules default to enabled"
        );
        // Every honest-default string field is empty.
        assert_eq!(vm.mergeable_note, "");
        assert_eq!(vm.branch_title, "");
        assert_eq!(vm.branch_description, "");
        assert_eq!(vm.reviewers_note, "");
        assert_eq!(vm.labels_note, "");
        assert_eq!(vm.checks_note, "");
        assert_eq!(vm.ask_placeholder, "");
        assert!(vm.examples.is_empty());
    }

    // ── populated-log tests ──────────────────────────────────────────────────

    #[test]
    fn ref_update_populates_base_branch() {
        // Use append_for_test with kind "ref.update" (not "ref.updated") so
        // replay() processes the record and base reflects the pushed branch.
        let mut log = EventLog::new();
        push_ref_update(&mut log, "main", "aabbcc112233445566778899", 1000);
        let vm = build_new_pr(&log, "hugit");
        assert_eq!(
            vm.base, "main",
            "base is the default branch from ref.update"
        );
    }

    #[test]
    fn campaign_and_policy_project_from_log() {
        let mut log = EventLog::new();
        push_campaign(&mut log, "auth-hardening", 1000);
        push_campaign(&mut log, "auth-hardening", 2000); // de-dup: same id
        push_campaign(&mut log, "refactor-engine", 3000);
        // Disable one house rule: gate still applies (two others default to enabled).
        push_policy(&mut log, "dco", false, 4000);

        let vm = build_new_pr(&log, "hugit");

        // REAL: campaigns de-duped by first-seen.
        assert_eq!(vm.campaigns.len(), 2, "de-dup: only two distinct campaigns");
        assert_eq!(vm.campaigns[0].id, "auth-hardening");
        assert_eq!(vm.campaigns[1].id, "refactor-engine");

        // REAL: policy_note still present (changelog + secrets default to enabled).
        assert!(!vm.policy_note.is_empty());
    }

    #[test]
    fn policy_note_empty_when_all_house_rules_disabled() {
        let mut log = EventLog::new();
        // Explicitly disable all three house rules.
        push_policy(&mut log, "dco", false, 1000);
        push_policy(&mut log, "changelog", false, 2000);
        push_policy(&mut log, "secrets", false, 3000);

        let vm = build_new_pr(&log, "hugit");
        assert_eq!(
            vm.policy_note, "",
            "no enabled house rules → empty policy_note"
        );
    }

    #[test]
    fn vm_round_trips_json() {
        let mut log = EventLog::new();
        push_campaign(&mut log, "wave-1", 1000);
        let vm = build_new_pr(&log, "hugit");
        let json = serde_json::to_string(&vm).unwrap();
        let reparsed: NewPrVm = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, reparsed, "NewPrVm round-trip is lossless");
    }
}
