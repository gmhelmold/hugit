//! `GET /v1/me/attention` → [`AttentionVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. This is the identity-scoped
//! "what needs my attention" feed; `AttentionVm` carries no principal field, so
//! the dev-principal stub is the router's concern, not a VM field.
//!
//! REAL backbone:
//! - Source set = every `pr.opened` (folded in chain order) whose current state
//!   is `proposed`/`queued`, PLUS `abandoned` PRs. PR state is derived exactly
//!   like `review.rs` (`has_pr_event` over `pr.landed`/`pr.abandoned`/`pr.queued`,
//!   else `proposed`).
//! - `verdict.recorded` `VerdictObject`s per `pr_id` → `verdicts` + evidence
//!   (the `review.rs` `verdict_pair` projection).
//! - `pr.comment` count per `pr_id` → the `why` sentence + an evidence line.
//!
//! ORDER: chain (open-seq) order — stable + deterministic. The rich D9 composite
//! rank needs policy/blast/confidence inputs the log records do NOT carry, so we
//! do NOT fabricate a score.
//!
//! DERIVED (not payload free-text, no scrub): `kind`, `type_label`,
//! `signal_class`, `why`, `actions` labels/hrefs, `repo`, numeric ids.
//!
//! ECHOED free-text (scrubbed at the read boundary via [`crate::fmt::scrub`]):
//! the PR `campaign` (in `title`) and verdict `lens`/`claims_checked` (in
//! `verdicts`/`evidence`).
//!
//! HONEST-DEFAULT: `note` is always `None` (no per-decision note record kind);
//! `target_repo`/`target_pr` are `Some(..)` only for write-action kinds
//! (`land`/`verdict`), `None` for `abandoned`; `decisions` can legitimately be
//! `[]` on a log with no attention-worthy PRs.

use crate::fmt::{humanize_age, scrub};
use hugit_cli::pr::{
    PR_ABANDONED_KIND, PR_LANDED_KIND, PR_OPENED_KIND, PR_QUEUED_KIND, find_pr_opened,
};
use hugit_cli::verdict::VERDICT_RECORDED_KIND;
use hugit_contracts::VerdictObject;
use hugit_http_contracts::attention::{
    AttentionActionVm, AttentionDecisionVm, AttentionEvidenceVm, AttentionVm,
};
use hugit_http_contracts::common::{VerdictVm, decision_of};
use hugit_refstore::EventLog;
use serde_json::Value;

const PR_COMMENT_KIND: &str = "pr.comment";

/// Cap the number of decisions projected (bound the read against a huge log).
const ATTENTION_CAP: usize = 200;

/// Build the attention view-model (router calls `build_attention(log, repo)`).
///
/// `repo` is the active/default repo context the router hands in (the `/v1/me/*`
/// route has no `{repo}` path param); it is projected verbatim into each
/// decision's `repo`/`target_repo`, exactly as `security.rs`/`issues.rs` thread
/// their `{repo}` arg.
#[must_use]
pub fn build_attention(log: &EventLog, repo: &str) -> AttentionVm {
    let mut decisions = Vec::new();
    // Fold pr.opened in chain (seq) order — the honest deterministic order.
    for pr_id in opened_pr_ids_in_chain_order(log) {
        if decisions.len() >= ATTENTION_CAP {
            break;
        }
        // Re-resolve via the public anchor (latest pr.opened for the id).
        let Some(opened) = find_pr_opened(log, &pr_id) else {
            continue;
        };
        let state = pr_state(log, &pr_id);
        // Only attention-worthy states surface; landed PRs are settled, not feed.
        if state == PrState::Landed {
            continue;
        }
        decisions.push(build_decision(log, repo, &opened, state));
    }
    // TRIAGE SORT (review-legibility ④): re-order by severity/blast derived from
    // the ALREADY-built VMs — no fabricated score. The sort is STABLE, so within a
    // tier the chain (open-seq) order is preserved.
    decisions.sort_by_key(triage_rank);
    AttentionVm { decisions }
}

/// Build the identity-scoped (`/v1/me/attention`) feed AGGREGATED across the
/// CALLER's own repos (W-METENANT). `repos` is the caller's authorized
/// `(slug, verified-log)` set (from `AppState::me_repo_logs`) — already
/// read-authz-filtered, so it holds ONLY repos the caller may read (a foreign
/// tenant's private repo is simply absent — no oracle).
///
/// The VM WIRE SHAPE is UNCHANGED (`AttentionVm`); only the DATA changes — the
/// decisions of every caller repo, concatenated then TRIAGE-SORTED across the
/// whole aggregate (severity ladder first, stable within a tier). An EMPTY
/// `repos` → `decisions: []` (honest empty, never a default repo's feed). Bounded
/// by `ATTENTION_CAP` over the aggregate. Reuses the per-repo [`build_attention`]
/// verbatim (each decision already carries its own repo's slug).
#[must_use]
pub fn build_me_attention(repos: &[(String, EventLog)]) -> AttentionVm {
    let mut decisions = Vec::new();
    for (slug, log) in repos {
        if decisions.len() >= ATTENTION_CAP {
            break;
        }
        for d in build_attention(log, slug).decisions {
            if decisions.len() >= ATTENTION_CAP {
                break;
            }
            decisions.push(d);
        }
    }
    // Re-sort the AGGREGATE by the same severity ladder (stable → within a tier the
    // per-repo, then chain, order is preserved).
    decisions.sort_by_key(triage_rank);
    AttentionVm { decisions }
}

/// The triage rank of a decision (lower = more urgent), derived solely from the
/// already-computed VM fields (`signal_class` from real verdicts, `kind` from
/// real state). No fabricated blast score — the rank IS the severity ladder:
///
/// 0. REJECT — a recorded reject blocks landing (signal_class `err`).
/// 1. FIX-FIRST — a recorded fix-first needs work before land (signal_class `warn`).
/// 2. APPROVE-ready — a `land` kind with a green verdict (ready to land).
/// 3. pending-verdict — a `verdict` kind (awaiting the panel).
/// 4. land-without-verdict — queued for land, no verdict signal yet.
/// 5. abandoned — terminal, lowest urgency.
fn triage_rank(d: &AttentionDecisionVm) -> u8 {
    match d.signal_class.as_str() {
        "err" => 0,  // any REJECT verdict
        "warn" => 1, // any FIX-FIRST verdict
        "g" => 2,    // all-APPROVE → ready to land
        _ => match d.kind.as_str() {
            "abandoned" => 5,
            "verdict" => 3, // proposed, no verdict yet → awaiting the panel
            _ => 4,         // land kind with no verdict signal (queued)
        },
    }
}

/// The distinct `pr.opened` ids in chain (seq) order, first-seen wins.
fn opened_pr_ids_in_chain_order(log: &EventLog) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut ids = Vec::new();
    for r in log.records().iter().filter(|r| r.kind == PR_OPENED_KIND) {
        let Ok(v) = serde_json::from_str::<Value>(&r.payload) else {
            continue;
        };
        let Some(pr_id) = v.get("pr_id").and_then(Value::as_str) else {
            continue;
        };
        if seen.insert(pr_id.to_string()) {
            ids.push(pr_id.to_string());
        }
    }
    ids
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PrState {
    Landed,
    Abandoned,
    Queued,
    Proposed,
}

/// PR state, derived exactly like `review.rs` (`pr_state` is private upstream, so
/// this replicates the `has_pr_event` anchor).
fn pr_state(log: &EventLog, pr_id: &str) -> PrState {
    if has_pr_event(log, PR_LANDED_KIND, pr_id) {
        PrState::Landed
    } else if has_pr_event(log, PR_ABANDONED_KIND, pr_id) {
        PrState::Abandoned
    } else if has_pr_event(log, PR_QUEUED_KIND, pr_id) {
        PrState::Queued
    } else {
        PrState::Proposed
    }
}

/// Copied from `review.rs` (its anchor explicitly says to replicate it).
fn has_pr_event(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// The `recorded_at` of the record that drives this decision's age: the
/// abandoned/queued record for those states, else the `pr.opened` record.
fn driving_age(log: &EventLog, pr_id: &str, state: PrState, opened_seq: u64) -> u64 {
    let kind = match state {
        PrState::Abandoned => PR_ABANDONED_KIND,
        PrState::Queued => PR_QUEUED_KIND,
        // Proposed (and the unreachable Landed) age off pr.opened.
        _ => PR_OPENED_KIND,
    };
    if kind == PR_OPENED_KIND {
        return record_at_for_seq(log, opened_seq).unwrap_or(0);
    }
    // Latest matching lifecycle record for this PR (chain order → last wins).
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| {
            let v = serde_json::from_str::<Value>(&r.payload).ok()?;
            (v.get("pr_id").and_then(Value::as_str) == Some(pr_id)).then_some(r.recorded_at)
        })
        .next_back()
        .unwrap_or(0)
}

fn record_at_for_seq(log: &EventLog, seq: u64) -> Option<u64> {
    log.records()
        .iter()
        .find(|r| r.seq == seq)
        .map(|r| r.recorded_at)
}

/// `pr.comment` matches the PR by `pr_id` as a string OR a u64 (the write verb
/// serializes it as an integer). Copied from `review.rs`.
fn comment_matches_pr(v: &Value, pr_id: &str) -> bool {
    let by_str = v.get("pr_id").and_then(Value::as_str) == Some(pr_id);
    let by_int = pr_id
        .parse::<u64>()
        .ok()
        .map(|n| v.get("pr_id").and_then(Value::as_u64) == Some(n))
        .unwrap_or(false);
    by_str || by_int
}

fn count_comments(log: &EventLog, pr_id: &str) -> usize {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_COMMENT_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter(|v| comment_matches_pr(v, pr_id))
        .count()
}

/// The `(VerdictVm, evidence_prose)` pairs for a PR id (the `review.rs` pattern).
fn verdict_pairs(log: &EventLog, pr_id: &str) -> Vec<(VerdictVm, String)> {
    log.records()
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .filter(|vo| vo.intent == pr_id)
        .map(|vo| verdict_pair(&vo))
        .collect()
}

/// Copied from `review.rs`: a clean `VerdictObject` → `(VerdictVm, prose)`. Every
/// echoed free-text field (`lens`, `claims_checked`) is scrubbed.
fn verdict_pair(vo: &VerdictObject) -> (VerdictVm, String) {
    use hugit_contracts::verdict_object::Verdict;
    let outcome_str = match vo.verdict {
        Verdict::Approve => "APPROVE",
        Verdict::FixFirst => "FIX-FIRST",
        Verdict::Reject => "REJECT",
    };
    let lens = scrub(&vo.lens);
    // REAL (review-legibility ②): reviewer = the verdict's model (scrubbed).
    let reviewer = scrub(&vo.model);
    let evidence_mono_terms: Vec<String> = vo.claims_checked.iter().map(|c| scrub(c)).collect();
    let evidence_prose = if evidence_mono_terms.is_empty() {
        String::new()
    } else {
        scrub(&evidence_mono_terms.join(" · "))
    };
    let summary = if evidence_mono_terms.is_empty() {
        format!("{lens} · {outcome_str}")
    } else {
        format!(
            "{lens} · {outcome_str} · {} claim(s) checked",
            evidence_mono_terms.len()
        )
    };
    let vm = VerdictVm {
        verdict: outcome_str.to_string(),
        reviewer,
        summary,
        // Invariant (not a fabricated per-verdict signal): every verdict.recorded
        // VerdictObject in hugit is produced by the adversarial review panel.
        adversarial: true,
        lens,
        evidence_mono_terms,
        decision: decision_of(outcome_str), // REAL — structured, same outcome source
    };
    (vm, evidence_prose)
}

/// The verdict-derived signal class: green if all APPROVE, warn if any FIX-FIRST,
/// err if any REJECT, else a neutral default. Derived from real records only.
fn signal_class(verdicts: &[(VerdictVm, String)]) -> &'static str {
    if verdicts.is_empty() {
        return "info";
    }
    if verdicts.iter().any(|(v, _)| v.verdict == "REJECT") {
        "err"
    } else if verdicts.iter().any(|(v, _)| v.verdict == "FIX-FIRST") {
        "warn"
    } else {
        "g"
    }
}

/// The decision kind, derived from real state + whether a verdict exists.
fn decision_kind(state: PrState, has_verdict: bool) -> &'static str {
    match state {
        PrState::Abandoned => "abandoned",
        // Queued = awaiting land; proposed-WITH-a-verdict = ready to land.
        PrState::Queued => "land",
        PrState::Proposed if has_verdict => "land",
        // Proposed and NO verdict yet = awaiting a verdict.
        PrState::Proposed => "verdict",
        // Unreachable (Landed is filtered before build_decision); honest fallback.
        PrState::Landed => "land",
    }
}

fn type_label(kind: &str) -> String {
    match kind {
        "land" => "LAND",
        "verdict" => "VERDICT",
        "abandoned" => "ABANDONED",
        _ => "INFO",
    }
    .to_string()
}

/// The derived "why" sentence — fixed strings + real derived counts, no payload
/// free-text echoed.
fn build_why(state: PrState, comment_count: usize, verdict_count: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    match state {
        PrState::Queued => parts.push("em fila, aguardando land".to_string()),
        PrState::Abandoned => parts.push("abandonado".to_string()),
        PrState::Proposed if verdict_count == 0 => parts.push("sem veredito ainda".to_string()),
        PrState::Proposed => parts.push("veredito registrado, pronto pra land".to_string()),
        PrState::Landed => parts.push("landed".to_string()),
    }
    if comment_count > 0 {
        parts.push(format!("{comment_count} comentário(s) em aberto"));
    }
    parts.join(" · ")
}

/// The evidence sections — real material only; empty Vec is the honest default.
fn build_evidence(
    verdicts: &[(VerdictVm, String)],
    comment_count: usize,
) -> Vec<AttentionEvidenceVm> {
    let mut lines: Vec<String> = verdicts
        .iter()
        .map(|(_, prose)| prose.clone())
        .filter(|p| !p.is_empty())
        .collect();
    if comment_count > 0 {
        lines.push(format!("{comment_count} comentário(s) na conversa"));
    }
    if lines.is_empty() {
        return vec![];
    }
    vec![AttentionEvidenceVm {
        section: "Prova".to_string(),
        lines,
    }]
}

/// The action buttons — fixed UI labels + hrefs derived from numeric ids only.
fn build_actions(repo: &str, pr_id: &str, kind: &str) -> Vec<AttentionActionVm> {
    let mut actions = Vec::new();
    // The write action (href: None) keyed off kind.
    let write_label = match kind {
        "land" => Some("Aprovar e land →"),
        "verdict" => Some("Pedir veredito →"),
        _ => None, // abandoned / info-only: no write action.
    };
    if let Some(label) = write_label {
        actions.push(AttentionActionVm {
            label: label.to_string(),
            href: None,
        });
    }
    actions.push(AttentionActionVm {
        label: "Ver PR".to_string(),
        href: Some(format!("/r/{repo}/pr/{pr_id}")),
    });
    actions
}

fn build_decision(
    log: &EventLog,
    repo: &str,
    opened: &hugit_cli::pr::OpenedPr,
    state: PrState,
) -> AttentionDecisionVm {
    let pr_id = &opened.pr_id;
    let verdicts_pairs = verdict_pairs(log, pr_id);
    let has_verdict = !verdicts_pairs.is_empty();
    let comment_count = count_comments(log, pr_id);
    let kind = decision_kind(state, has_verdict).to_string();

    let title = if opened.campaign.is_empty() {
        format!("PR #{} — {} intents", pr_id, opened.intent_ids.len())
    } else {
        // Campaign is the ONLY payload free-text in the title → MUST scrub.
        format!(
            "PR #{} ({}) — {} intents",
            pr_id,
            scrub(&opened.campaign),
            opened.intent_ids.len()
        )
    };

    let verdicts: Vec<VerdictVm> = verdicts_pairs.iter().map(|(vm, _)| vm.clone()).collect();
    let evidence = build_evidence(&verdicts_pairs, comment_count);
    let actions = build_actions(repo, pr_id, &kind);

    // target_repo/target_pr: Some(..) ONLY for write-action kinds (land/verdict).
    let is_write_kind = kind == "land" || kind == "verdict";
    let target_repo = if is_write_kind {
        Some(repo.to_string())
    } else {
        None
    };
    let target_pr = if is_write_kind {
        pr_id.parse::<u64>().ok()
    } else {
        None
    };

    AttentionDecisionVm {
        type_label: type_label(&kind),
        signal_class: signal_class(&verdicts_pairs).to_string(),
        kind,
        title,
        repo: repo.to_string(),
        age: humanize_age(driving_age(log, pr_id, state, opened.seq)),
        why: build_why(state, comment_count, verdicts_pairs.len()),
        verdicts,
        evidence,
        actions,
        // No record kind carries a per-decision human note → honest default None.
        note: None,
        target_repo,
        target_pr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_cli::pr::PR_OPENED_KIND;
    use hugit_refstore::{Endpoint, PrincipalClass};

    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn append(log: &mut EventLog, kind: &str, payload: Value, at: u64) {
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

    fn open_pr(log: &mut EventLog, pr_id: &str, campaign: &str, intents: &[&str], at: u64) {
        append(
            log,
            PR_OPENED_KIND,
            serde_json::json!({"author_kind":"orchestrator","campaign":campaign,"intent_ids":intents,"pr_id":pr_id}),
            at,
        );
    }

    fn verdict(intent: &str, lens: &str, v: &str, claims: Value) -> Value {
        serde_json::json!({"intent":intent,"tree_hash":"","lens":lens,"model":"m","prompt_digest":"0".repeat(64),"verdict":v,"claims_checked":claims,"evidence_refs":[]})
    }

    #[test]
    fn empty_log_no_decisions() {
        let vm = build_attention(&EventLog::new(), "hugit");
        assert!(vm.decisions.is_empty());
    }

    #[test]
    fn proposed_pr_awaits_verdict() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "wave-x", &["i-1"], 1000);
        let vm = build_attention(&log, "hugit");
        assert_eq!(vm.decisions.len(), 1);
        let d = &vm.decisions[0];
        assert_eq!(d.kind, "verdict");
        assert_eq!(d.type_label, "VERDICT");
        assert_eq!(d.signal_class, "info");
        assert_eq!(d.repo, "hugit");
        assert!(d.why.contains("sem veredito"));
        // Write-action kind → Some target.
        assert_eq!(d.target_repo.as_deref(), Some("hugit"));
        assert_eq!(d.target_pr, Some(1));
        assert!(d.note.is_none());
    }

    #[test]
    fn queued_pr_awaits_land() {
        let mut log = EventLog::new();
        open_pr(&mut log, "2", "", &[], 1000);
        append(
            &mut log,
            PR_QUEUED_KIND,
            serde_json::json!({"item_id":"q-1","order_index":0,"pr_id":"2"}),
            2000,
        );
        let vm = build_attention(&log, "hugit");
        let d = &vm.decisions[0];
        assert_eq!(d.kind, "land");
        assert_eq!(d.type_label, "LAND");
        assert!(d.why.contains("em fila"));
        assert_eq!(d.target_pr, Some(2));
    }

    #[test]
    fn verdict_makes_land_kind_green() {
        let mut log = EventLog::new();
        open_pr(&mut log, "7", "c", &["i-1"], 1000);
        append(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict("7", "correctness", "approve", serde_json::json!(["c:ok"])),
            2000,
        );
        let vm = build_attention(&log, "hugit");
        let d = &vm.decisions[0];
        // proposed + a verdict → ready to land.
        assert_eq!(d.kind, "land");
        assert_eq!(d.signal_class, "g");
        assert_eq!(d.verdicts.len(), 1);
        assert_eq!(d.verdicts[0].verdict, "APPROVE");
        assert!(!d.evidence.is_empty());
    }

    #[test]
    fn abandoned_pr_no_write_target() {
        let mut log = EventLog::new();
        open_pr(&mut log, "9", "", &[], 1000);
        append(
            &mut log,
            PR_ABANDONED_KIND,
            serde_json::json!({"pr_id":"9","reason":"superseded"}),
            2000,
        );
        let vm = build_attention(&log, "hugit");
        let d = &vm.decisions[0];
        assert_eq!(d.kind, "abandoned");
        assert_eq!(d.type_label, "ABANDONED");
        assert!(d.target_repo.is_none());
        assert!(d.target_pr.is_none());
        // The abandoned decision still offers the read-only "Ver PR" link.
        assert!(d.actions.iter().all(|a| a.label != "Aprovar e land →"));
        assert!(d.actions.iter().any(|a| a.label == "Ver PR"));
    }

    #[test]
    fn landed_pr_is_not_attention() {
        let mut log = EventLog::new();
        open_pr(&mut log, "5", "", &[], 1000);
        append(
            &mut log,
            PR_LANDED_KIND,
            serde_json::json!({"pr_id":"5"}),
            2000,
        );
        let vm = build_attention(&log, "hugit");
        assert!(vm.decisions.is_empty());
    }

    #[test]
    fn pat_in_campaign_redacted() {
        let mut log = EventLog::new();
        open_pr(&mut log, "42", PAT, &["i"], 1000);
        let j = serde_json::to_string(&build_attention(&log, "hugit")).unwrap();
        assert!(!j.contains(PAT));
        assert!(j.contains("[REDACTED]"));
    }

    #[test]
    fn pat_in_verdict_claims_redacted() {
        let mut log = EventLog::new();
        open_pr(&mut log, "3", "", &["i"], 1000);
        append(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict(
                "3",
                "correctness",
                "approve",
                serde_json::json!([format!("c:{PAT}")]),
            ),
            2000,
        );
        let j = serde_json::to_string(&build_attention(&log, "hugit")).unwrap();
        assert!(!j.contains(PAT));
        assert!(j.contains("[REDACTED]"));
    }

    #[test]
    fn vm_round_trips() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "wave-test", &["i-1"], 1000);
        append(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict("1", "correctness", "approve", serde_json::json!(["ok"])),
            2000,
        );
        let vm = build_attention(&log, "humangr/hugit");
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<AttentionVm>(&j).unwrap());
    }

    // ── review-legibility ④ : triage-sort by severity/blast ──────────────────
    #[test]
    fn feed_is_ordered_by_severity_not_chain() {
        let mut log = EventLog::new();
        // PR 1 (oldest in chain): proposed, no verdict → pending-verdict tier.
        open_pr(&mut log, "1", "", &["i1"], 1000);
        // PR 2: REJECT verdict → most urgent (tier 0).
        open_pr(&mut log, "2", "", &["i2"], 1001);
        append(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict("2", "correctness", "reject", serde_json::json!(["bad"])),
            1002,
        );
        // PR 3: FIX-FIRST verdict → tier 1.
        open_pr(&mut log, "3", "", &["i3"], 1003);
        append(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict("3", "correctness", "fix_first", serde_json::json!(["meh"])),
            1004,
        );
        // PR 4: APPROVE verdict → ready to land, tier 2.
        open_pr(&mut log, "4", "", &["i4"], 1005);
        append(
            &mut log,
            VERDICT_RECORDED_KIND,
            verdict("4", "correctness", "approve", serde_json::json!(["ok"])),
            1006,
        );
        // PR 5: abandoned → least urgent, tier 5.
        open_pr(&mut log, "5", "", &[], 1007);
        append(
            &mut log,
            PR_ABANDONED_KIND,
            serde_json::json!({"pr_id":"5","reason":"superseded"}),
            1008,
        );

        let vm = build_attention(&log, "hugit");
        let order: Vec<&str> = vm
            .decisions
            .iter()
            .map(|d| d.title.split_whitespace().next().unwrap())
            .collect();
        // Severity order: REJECT(2) → FIX-FIRST(3) → APPROVE(4) → pending(1) → abandoned(5).
        // (titles begin "PR #N …"; compare the signal classes directly.)
        let classes: Vec<&str> = vm
            .decisions
            .iter()
            .map(|d| d.signal_class.as_str())
            .collect();
        assert_eq!(classes[0], "err", "REJECT first");
        assert_eq!(classes[1], "warn", "FIX-FIRST second");
        assert_eq!(classes[2], "g", "APPROVE-ready third");
        // The pending-verdict (info) precedes the abandoned tail.
        let kinds: Vec<&str> = vm.decisions.iter().map(|d| d.kind.as_str()).collect();
        assert_eq!(kinds.last().copied(), Some("abandoned"), "abandoned last");
        assert!(order.iter().all(|t| t.starts_with("PR")));
    }

    // ── W-METENANT: the identity-scoped aggregating feed ──────────────────────

    #[test]
    fn me_attention_empty_repo_set_is_empty() {
        let vm = build_me_attention(&[]);
        assert!(
            vm.decisions.is_empty(),
            "no repos → no feed (not a default repo)"
        );
    }

    #[test]
    fn me_attention_aggregates_and_triage_sorts_across_repos() {
        // repo A carries an APPROVE-ready PR (tier 2); repo B carries a REJECT
        // (tier 0). The aggregate must sort REJECT (repo B) BEFORE the approve
        // (repo A) — the global severity ladder, not per-repo order.
        let mut a = EventLog::new();
        open_pr(&mut a, "10", "", &["i"], 1000);
        append(
            &mut a,
            VERDICT_RECORDED_KIND,
            verdict("10", "correctness", "approve", serde_json::json!(["ok"])),
            1100,
        );
        let mut b = EventLog::new();
        open_pr(&mut b, "20", "", &["i"], 2000);
        append(
            &mut b,
            VERDICT_RECORDED_KIND,
            verdict("20", "correctness", "reject", serde_json::json!(["bad"])),
            2100,
        );
        let vm = build_me_attention(&[("org/alpha".to_string(), a), ("org/beta".to_string(), b)]);
        assert_eq!(vm.decisions.len(), 2);
        // Global triage: the REJECT (repo beta) is first despite being the 2nd repo.
        assert_eq!(vm.decisions[0].signal_class, "err");
        assert_eq!(vm.decisions[0].repo, "org/beta");
        assert_eq!(vm.decisions[1].signal_class, "g");
        assert_eq!(vm.decisions[1].repo, "org/alpha");
    }

    #[test]
    fn me_attention_is_bounded_across_the_aggregate() {
        let mk = |base: u64| {
            let mut log = EventLog::new();
            for i in 0..(ATTENTION_CAP + 10) {
                open_pr(
                    &mut log,
                    &format!("{}", base + i as u64),
                    "",
                    &["i"],
                    base + i as u64,
                );
            }
            log
        };
        let vm = build_me_attention(&[
            ("r/one".to_string(), mk(1_000)),
            ("r/two".to_string(), mk(900_000)),
        ]);
        assert!(
            vm.decisions.len() <= ATTENTION_CAP,
            "aggregate feed must stay bounded, got {}",
            vm.decisions.len()
        );
    }
}
