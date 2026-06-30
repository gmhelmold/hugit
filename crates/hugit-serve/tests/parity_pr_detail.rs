//! Parity test for `build_pr_detail` (Wave 1 prs/{n} handler).
//!
//! Verifies:
//!   1. An empty log → `build_pr_detail(&log, "hugit", 128, no_git())` returns `None`
//!      (no such PR → HTTP 404, `get_opt` semantics, no existence leak).
//!   2. A minimal log carrying ONE real `pr.opened` for PR 128 (constructed via
//!      the public `hugit_cli::pr::open` verb, the same way the engine creates
//!      the event) → `Some(vm)`, whose serialization round-trips losslessly back
//!      into `hugit_http_contracts::PrDetailVm`.
//!   3. The STUB fields equal their honest defaults — never faked: empty diff /
//!      reviewers / labels / branches, and (no PR-altitude envelope on the log)
//!      a ZERO cost block.

use hugit_cli::pr::{AuthorKind, OpenArgs, PR_ENVELOPE_KIND, open};
use hugit_contracts::context_envelope::{
    Altitude, Authorship, ContextEnvelope, FileRead, IntentMetrics, Snapshot, Spawn, TokenCounts,
    Trajectory,
};
use std::sync::Arc;

use hugit_http_contracts::PrDetailVm;
use hugit_refstore::EventLog;
use hugit_serve::handlers::build_pr_detail;

/// No git source — these parity tests assert the PR projection + cost split, not
/// the numstat (honest-empty without a git seam, exactly as before this wire-up).
fn no_git() -> Option<&'static Arc<dyn hugit_proto::ObjectSource + Send + Sync>> {
    None
}

/// A fresh empty `EventLog` — trivially chain-verified (the handler contract:
/// the log is ALREADY verified by the caller).
fn empty_log() -> EventLog {
    EventLog::new()
}

/// Build a minimal log with ONE real `pr.opened` for PR `n` (human author,
/// no campaign, no bundle — all of which pass the open validators on a log with
/// no campaign/intent vocabulary). No PR-altitude envelope is captured, so the
/// cost block must be honest ZERO.
fn log_with_open_pr(n: u32) -> EventLog {
    let mut log = EventLog::new();
    let args = OpenArgs {
        pr_id: n.to_string(),
        campaign: String::new(),
        author_kind: AuthorKind::Human,
        run_id: None,
        principal: Some("human:owner".to_string()),
        intent_ids: vec![],
        recorded_at: 1_700_000_000_000,
    };
    open(&mut log, &args).expect("pr.opened append succeeds on a fresh log");
    log
}

#[test]
fn empty_log_unknown_pr_is_none() {
    let log = empty_log();
    assert!(
        build_pr_detail(&log, "hugit", 128, no_git()).is_none(),
        "no pr.opened for 128 on an empty log → None (HTTP 404, no existence leak)"
    );
}

#[test]
fn empty_log_none_path_is_total() {
    // The None path is the 404 contract — exercise a few ids to be sure it is a
    // total "not found", not an id-specific quirk.
    let log = empty_log();
    for n in [0u32, 1, 42, 128, u32::MAX] {
        assert!(build_pr_detail(&log, "hugit", n, no_git()).is_none());
    }
}

#[test]
fn open_pr_round_trips() {
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128, no_git()).expect("PR 128 is present → Some(vm)");

    // 1. Serializes without error.
    let json = serde_json::to_string(&vm).expect("PrDetailVm serializes");
    // 2. Re-parses losslessly through the FROZEN contract type.
    let reparsed: PrDetailVm = serde_json::from_str(&json).expect("JSON re-parses into PrDetailVm");
    assert_eq!(vm, reparsed, "round-trip must be lossless");
}

#[test]
fn open_pr_real_fields() {
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128, no_git()).unwrap();
    assert_eq!(vm.repo, "hugit");
    assert_eq!(vm.number, 128);
    assert_eq!(
        vm.change_id, "128",
        "change_id is the PR's stable id (REAL)"
    );
    assert_eq!(
        vm.state_label, "proposed",
        "an opened-but-not-queued PR is `proposed` (REAL projection)"
    );
}

#[test]
fn open_pr_no_envelope_cost_is_zero() {
    // No PR-altitude envelope on the log → honest ZERO cost, never faked.
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128, no_git()).unwrap();
    assert_eq!(vm.cost.work_usd, 0.0, "no envelope → work_usd 0.0");
    assert_eq!(vm.cost.orchestration_usd, 0.0);
    assert_eq!(vm.cost.verification_usd, 0.0);
    assert_eq!(vm.cost.ci_usd, 0.0);
    assert_eq!(vm.cost.total_usd, 0.0);
    assert_eq!(vm.cost.waste_usd, 0.0);
    assert_eq!(vm.cost.overhead_pct, 0);
    assert_eq!(vm.cost.cache_savings_pct, 0);
    assert_eq!(
        vm.cost.work_note, "",
        "ZERO cost notes are empty, not faked"
    );
}

#[test]
fn open_pr_stub_fields_are_honest_defaults() {
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128, no_git()).unwrap();

    // No diffstat seam.
    assert!(vm.diff.files.is_empty(), "diff.files [] — no diffstat seam");
    assert!(vm.diff.hunks.is_empty(), "diff.hunks [] — no diffstat seam");
    assert_eq!(vm.file_count, 0);
    assert_eq!(vm.added, 0);
    assert_eq!(vm.removed, 0);

    // No git-ref tracking.
    assert_eq!(
        vm.source_branch, "",
        "no git-ref tracking → source_branch \"\""
    );
    assert_eq!(
        vm.target_branch, "",
        "no git-ref tracking → target_branch \"\""
    );

    // No reviewer / label / milestone seams.
    assert!(
        vm.reviewers.is_empty(),
        "reviewers [] — no reviewer rail seam"
    );
    assert!(vm.labels.is_empty(), "labels [] — STUB");
    assert!(vm.milestone.is_none(), "milestone None — STUB");
    assert!(vm.conversation.is_empty(), "conversation [] — no Q&A seam");

    // P2 mirror is honest false/empty.
    assert!(!vm.mirror.synced, "mirror.synced false — P2 STUB");
    assert_eq!(vm.mirror.detail, "", "mirror.detail \"\" — P2 STUB");

    // Impact blast-radius is empty (not wired); the checks counts are REAL (0
    // here — no check.recorded on this log).
    assert!(vm.impact.crates_touched.is_empty());
    assert!(vm.impact.direct_dependents.is_empty());
    assert_eq!(vm.impact.checks_selected, 0, "no check.recorded → 0 (REAL)");
    assert_eq!(vm.impact.checks_total, 0);
}

// ── P0 SECRET-LEAK matrix: PR-altitude free text MUST be scrubbed ────────────

/// A secret-shaped GitHub PAT (`ghp_` known prefix) — fires the redaction
/// detector. NOT a real credential.
const SECRET_PAT: &str = "ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
/// A secret-shaped OpenAI-style key (`sk-` + ≥20 chars) — fires the detector.
const SECRET_SK: &str = "sk-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";

/// The `[REDACTED]` sentinel the engine's `redact::apply` substitutes for a
/// secret-bearing field (whole-string replacement).
const REDACTED: &str = "[REDACTED]";

/// A PR-altitude `ContextEnvelope` whose free-text fields carry secret shapes,
/// serialized the way the engine captures it (the acceptance-test technique).
fn poisoned_pr_envelope(pr_id: &str) -> ContextEnvelope {
    ContextEnvelope {
        schema_version: "1.1.0".to_string(),
        altitude: Altitude::Pr,
        intent_id: pr_id.to_string(),
        commit: String::new(),
        tree_hash: String::new(),
        authorship: Authorship {
            model: format!("opus-4.8 {SECRET_SK}"), // → envelope.model
            model_digest: "sha256:dead".to_string(),
            agent_type: "main".to_string(), // top-level session (passes the D14 gate)
            spawn: Spawn {
                run_id: "run-orch".to_string(),
                parent_run_id: None,
                born_at: 1_000,
                died_at: 2_000,
            },
            operator: format!("gustavo {SECRET_PAT}"), // → author
        },
        charter: format!("ship it — token {SECRET_PAT}"), // → why
        campaign: None,
        constraints: vec![],
        acceptance: vec![format!("no leaks — verify with {SECRET_SK}")],
        parent_intents: vec![],
        trajectory: Trajectory {
            raw_transcript_ref: None,
            task_transcript_ref: None,
            // summary → envelope.headline AND envelope.session_summary.
            summary: Some(format!("did the work, key was {SECRET_PAT}")),
            journal_ref: None,
            redaction_policy: "default".to_string(),
        },
        snapshot: Snapshot {
            files_read: vec![FileRead {
                path: format!("src/leak-{SECRET_PAT}.rs"),
                hash: "sha256:beef".to_string(),
            }],
            prompt_ref: None,
            env_manifest: "rustc 1.96.0".to_string(),
        },
        metrics: IntentMetrics {
            tokens: TokenCounts {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                total: 0,
            },
            wall_ms: 0,
            active_ms: 0,
            tool_calls: 0,
            tool_breakdown: vec![],
            model_turns: 0,
            cost_usd_micros: 0,
        },
        verdicts_ref: None,
    }
}

/// A log with a real `pr.opened` for PR 128 + a PR-altitude envelope whose
/// charter/operator/summary/etc. carry secret shapes.
fn log_with_poisoned_envelope(n: u32) -> EventLog {
    let mut log = log_with_open_pr(n);
    let env = poisoned_pr_envelope(&n.to_string());
    let payload = serde_json::to_string(&env).expect("envelope serializes");
    let canonical = hugit_refstore::canonical_json(&payload).expect("canonical json");
    log.append_for_test(PR_ENVELOPE_KIND, vec![], canonical, 3_000);
    log
}

#[test]
fn pr_altitude_secrets_are_scrubbed_not_echoed() {
    let log = log_with_poisoned_envelope(128);
    let vm = build_pr_detail(&log, "hugit", 128, no_git()).expect("PR 128 present → Some(vm)");

    // The whole serialized VM must carry NO raw secret anywhere.
    let json = serde_json::to_string(&vm).expect("PrDetailVm serializes");
    assert!(
        !json.contains(SECRET_PAT) && !json.contains(SECRET_SK),
        "no raw secret may reach the browser anywhere in the VM"
    );

    // Field-by-field: each scrubbed field is the [REDACTED] sentinel, never raw.
    assert_eq!(vm.why, REDACTED, "charter → why scrubbed");
    assert_eq!(vm.author, REDACTED, "operator → author scrubbed");
    assert_eq!(
        vm.envelope.headline, REDACTED,
        "summary → headline scrubbed"
    );
    assert_eq!(
        vm.envelope.session_summary, REDACTED,
        "summary → session_summary scrubbed"
    );
    assert_eq!(vm.envelope.model, REDACTED, "model scrubbed");
    assert_eq!(
        vm.acceptance_note, REDACTED,
        "acceptance line scrubbed before join"
    );
    assert_eq!(
        vm.envelope.snapshot_files,
        vec![REDACTED.to_string()],
        "snapshot path scrubbed"
    );

    // Structural fields are NOT scrubbed (content-addresses, not free text).
    assert_eq!(vm.number, 128);
    assert_eq!(vm.change_id, "128");
    assert_eq!(vm.session, "128", "PR-altitude session id is structural");
}

/// SECRET-MATRIX (read-boundary): a `pr.opened` whose `campaign` is secret-shaped,
/// pushed RAW via `append_for_test` (bypassing `open()`'s write-scrub), is scrubbed
/// at the READ boundary in the PR title, the campaign chip, AND provenance — the
/// path the first fix missed (re-audit P1).
#[test]
fn pr_opened_campaign_is_scrubbed_at_read_boundary() {
    let mut log = EventLog::new();
    let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
    log.append_for_test(
        "pr.opened",
        vec!["test".to_string()],
        serde_json::json!({
            "pr_id": "128",
            "campaign": secret,
            "author_kind": "orchestrator",
            "intent_ids": ["i1"],
            "principal": serde_json::Value::Null,
            "run_id": "r-1",
        })
        .to_string(),
        1_000,
    );

    let vm = build_pr_detail(&log, "hugit", 128, no_git()).expect("PR 128 is present");
    assert!(
        vm.title.contains("[REDACTED]"),
        "PR title scrubs the campaign, got: {}",
        vm.title
    );
    assert!(
        !vm.title.contains("ghp_"),
        "the raw PAT never reaches the PR title"
    );
    assert!(
        vm.provenance_campaign.contains("[REDACTED]"),
        "provenance_campaign scrubbed"
    );
    if let Some(chip) = &vm.campaign {
        assert!(chip.id.contains("[REDACTED]"), "campaign chip id scrubbed");
        assert!(!chip.id.contains("ghp_"), "raw PAT never in the chip id");
    }
    // Structural id is not scrubbed.
    assert_eq!(vm.number, 128);
}
