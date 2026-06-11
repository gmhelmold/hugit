//! Adversarial verdict panels — `verdict request --lens` fan-out (WP-D7).
//!
//! A verdict panel fans a change out to INDEPENDENT reviewer lenses. Each lens
//! is a distinct reviewer that judges the change against **served ground truth
//! only** — build-graph impact, contracts, and `CheckResult` evidence. The
//! change *never defends itself*: no author-controlled text is ever placed in a
//! reviewer's input set, so there is no persuasion channel for the author to
//! exploit (the no-self-defense invariant, [`lens_audit`]).
//!
//! The panel is **diversity-enforced** ([`diversity`]): a homogeneous panel
//! (every lens sharing one prompt *and* one model) is rejected by construction;
//! a real panel dispatches distinct prompts AND ≥2 distinct models.
//!
//! Human review is grounded interrogation, not generative prose ([`qa`]): every
//! answer is a citation to a real evidence object, or an explicit refusal when
//! no grounding exists — it never fabricates.
//!
//! ## Acceptance lane note
//!
//! Everything here is **structural / fixture-driven**: lens isolation, the
//! planted-bug catch, diversity enforcement, Q&A grounding, and the
//! persuasion-channel negative are all asserted over local fixtures and pure
//! dispatch logic. There are **no live model API calls** in the verdict
//! acceptance lane — a [`Reviewer`] is an injected strategy, and the fixtures
//! ship deterministic reviewers.

pub mod diversity;
pub mod lens_audit;
pub mod panel_dispatch;
pub mod qa;

pub mod fixtures;

pub use diversity::{DiversityError, DiversityReport, enforce_diversity};
pub use lens_audit::{IsolationError, IsolationReport, audit_isolation};
pub use panel_dispatch::{
    Lens, Panel, PanelError, Reviewer, ReviewerInput, ServedGroundTruth, dispatch,
};
pub use qa::{Answer, EvidenceStore, QaError, answer_question};

pub use hugit_contracts::{IntentSidecar, Verdict, VerdictObject};

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::checks::load_event_log;
use crate::porcelain::PorcelainError;
use crate::pr::filelock::{FileLock, LockError, atomic_write};

/// Event kind a recorded adversarial verdict is appended under.
///
/// **Producer (frozen at W0): `hugit verdict` — the W-VERDICT EXECUTE path**
/// ([`run`]). Additive over the D1 log, sibling to `check.recorded`/`pr.landed`.
/// `hugit campaign show` already READS this kind to surface a PR's `proven`
/// state (the verdict-recorded count), so naming it here freezes the wire string
/// the recorder targets; the campaign read and the recorder agree by this const.
pub const VERDICT_RECORDED_KIND: &str = "verdict.recorded";

/// `hugit verdict` flags — the adversarial-verdict EXECUTE path (W-VERDICT,
/// ported onto W0's frozen flat `verdict` entry point at W-INT).
///
/// Convenes a multi-lens adversarial panel for `--intent` (one `--lens NAME
/// --result approve|fix_first|reject` pair per lens, positionally paired) and —
/// with `--store` — records the aggregate [`VerdictObject`] onto the canonical
/// `--log` as a [`VERDICT_RECORDED_KIND`] event the way `campaign show` reads it.
///
/// W0 froze the seam `--intent --log [--store]`; W-INT ports the W-VERDICT
/// recorder body in behind it and adds the *additive* `--lens` / `--result` /
/// `--tree-hash` / `--recorded-at` flags — all optional-or-repeatable, so the
/// frozen `--intent --log [--store]` contract is unchanged.
#[derive(clap::Args, Debug)]
pub struct VerdictArgs {
    /// The intent / change id to convene the verdict panel over.
    #[arg(long)]
    pub intent: String,
    /// Path to the canonical JSON event log — the shared `--log` seam the
    /// recorded verdict is appended to and `campaign show` projects `proven` from.
    #[arg(long)]
    pub log: PathBuf,
    /// Record the [`VerdictObject`] onto the log as a `verdict.recorded` event
    /// (the recorder seam `campaign show` reads). Omit to convene without
    /// persisting (a dry panel).
    #[arg(long)]
    pub store: bool,
    /// Lens names (`--lens security --lens contracts …`), repeatable. Must pair
    /// 1:1 with `--result`.
    #[arg(long = "lens")]
    pub lens: Vec<String>,
    /// Per-lens results (`--result approve|fix_first|reject`), repeatable.
    /// Positionally paired with `--lens`: the N-th `--result` belongs to the
    /// N-th `--lens`.
    #[arg(long = "result")]
    pub result: Vec<String>,
    /// Workspace Merkle tree hash of the snapshot reviewed. Defaults to the empty
    /// string (honest placeholder — the real hash is a P2 live-infra seam).
    #[arg(long = "tree-hash", default_value = "")]
    pub tree_hash: String,
    /// Unix-ms timestamp to stamp the appended event with (0 = now / left as-is).
    #[arg(long = "recorded-at", default_value_t = 0)]
    pub recorded_at: u64,
}

/// `hugit verdict` — convene an adversarial verdict panel and (with `--store`)
/// record its aggregate verdict to the canonical log (W-VERDICT EXECUTE path).
///
/// Emits stable JSON on stdout under the WB0 one-exit-code law: the recorded
/// verdict (intent, per-lens breakdown, aggregate), or the canonical
/// `{"error":{…}}` envelope (exit 2) on any fault.
pub fn run(args: VerdictArgs) -> ExitCode {
    match record(args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}

/// Convene the panel and (with `--store`) append a `verdict.recorded` event to
/// the canonical log, idempotently.
///
/// # What is validated
///
/// - `--lens` and `--result` must be non-empty and the same length (one result
///   per lens).
/// - Each `--result` must be `approve`, `fix_first`, or `reject`
///   (case-insensitive).
/// - When `--store` is set, the `--log` file must exist (missing →
///   `log_not_found`/exit-2).
///
/// # Idempotency
///
/// If the log already carries a `verdict.recorded` for this exact intent + lens
/// set (same lens names, same results, same order), the existing record is
/// returned with `"already_recorded":true` (exit 0) — no second event is
/// appended.
///
/// # Aggregate result
///
/// The aggregate is APPROVE when every per-lens result is `approve`, and REJECT
/// otherwise (a single `fix_first` or `reject` lens fails the panel).
///
/// # Authorization
///
/// The `--store` append routes through `append_authorized(Orchestrator, Land)` —
/// the same guard the `pr.queued` append uses (WA2b: the orchestrator lands
/// changes through the queue; recording a verdict is its integration primitive).
fn record(args: VerdictArgs) -> Result<Value, PorcelainError> {
    // ── Validate lens/result pairs ────────────────────────────────────────────
    if args.lens.is_empty() {
        return Err(PorcelainError::new(
            "missing_lens",
            "at least one --lens <name> --result <outcome> pair is required",
            "pass e.g. --lens security --result approve (repeatable)",
        ));
    }
    if args.lens.len() != args.result.len() {
        return Err(PorcelainError::new(
            "lens_result_mismatch",
            format!(
                "{} --lens flags but {} --result flags: they must pair 1:1",
                args.lens.len(),
                args.result.len()
            ),
            "pass one --result per --lens, in the same order",
        )
        .with_context("lens_count", json!(args.lens.len()))
        .with_context("result_count", json!(args.result.len())));
    }

    // ── Parse per-lens verdicts ───────────────────────────────────────────────
    let mut lens_verdicts: Vec<(String, Verdict)> = Vec::with_capacity(args.lens.len());
    for (name, raw) in args.lens.iter().zip(args.result.iter()) {
        let v = parse_verdict(raw).ok_or_else(|| {
            PorcelainError::new(
                "invalid_result",
                format!(
                    "--result '{raw}' is not a valid verdict outcome (expected \
                     approve, fix_first, or reject)"
                ),
                "pass --result approve, --result fix_first, or --result reject",
            )
            .with_context("got", json!(raw))
        })?;
        lens_verdicts.push((name.clone(), v));
    }

    let aggregate = aggregate_verdict(&lens_verdicts);
    let lens_verdicts_json: Vec<Value> = lens_verdicts
        .iter()
        .map(|(name, v)| json!({ "lens": name, "result": verdict_to_str(v) }))
        .collect();

    // ── Dry panel (no --store): convene + return, never touch the log. ─────────
    if !args.store {
        return Ok(json!({
            "verdict_recorded": false,
            "stored": false,
            "intent": args.intent,
            "lenses": lens_verdicts_json,
            "aggregate": verdict_to_str(&aggregate),
        }));
    }

    // ── Acquire advisory lock + load log ─────────────────────────────────────
    let path = &args.log;
    let _lock = FileLock::acquire(path).map_err(|e| lock_error(e, path))?;
    let log = load_event_log(path)?;

    // ── Idempotency check ─────────────────────────────────────────────────────
    let intent = &args.intent;
    if let Some(existing) = find_existing_verdict(&log, intent, &lens_verdicts) {
        return Ok(json!({
            "verdict_recorded": true,
            "already_recorded": true,
            "stored": true,
            "intent": intent,
            "lenses": existing.lens_verdicts_json,
            "aggregate": existing.aggregate_json,
        }));
    }

    // ── Build the VerdictObject payload ───────────────────────────────────────
    // One record per recording call covers the multi-lens aggregate. The
    // individual lens breakdowns are carried in claims_checked (lens:result
    // pairs) so the full per-lens picture is on the log and the campaign
    // verdicts_of projection can surface it.
    let claims_checked: Vec<String> = lens_verdicts
        .iter()
        .map(|(name, v)| format!("{}:{}", name, verdict_to_str(v)))
        .collect();

    let verdict_obj = VerdictObject {
        intent: intent.clone(),
        tree_hash: args.tree_hash.clone(),
        // The `lens` field names the aggregate record; individual lens names
        // are in claims_checked.
        lens: "panel".to_string(),
        model: "porcelain".to_string(),
        prompt_digest: "0".repeat(64),
        verdict: aggregate.clone(),
        claims_checked,
        evidence_refs: Vec::new(),
    };

    // Canonical JSON payload — sorted keys, no insignificant whitespace —
    // SCRUBBED-ON-APPEND (WG-SCRUB): `--intent`/`--lens` (and the lens names in
    // `claims_checked`) are user strings; they are redacted BEFORE the bytes
    // reach the hash chain. `tree_hash`/`prompt_digest` survive by the helper's
    // digest-key exemption.
    let payload_value = serde_json::to_value(&verdict_obj)
        .map_err(|e| PorcelainError::internal(format!("serialise VerdictObject: {e}")))?;
    let payload = crate::porcelain::scrub_to_canonical(payload_value);

    // ── Append through the D14 authorization guard ────────────────────────────
    use hugit_refstore::{Endpoint, PrincipalClass};
    let mut log = log;
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        VERDICT_RECORDED_KIND,
        vec!["orchestrator:verdict-recorder".to_string()],
        payload,
        args.recorded_at,
    )
    .map_err(|denied| {
        PorcelainError::new(
            "authz_denied",
            format!(
                "verdict.recorded append denied by D14 guard: {}",
                denied.reason.code()
            ),
            "verdict recording requires the orchestrator principal class",
        )
    })?;

    // ── Persist the updated log ───────────────────────────────────────────────
    let bytes = serde_json::to_vec_pretty(log.records())
        .map_err(|e| PorcelainError::internal(format!("serialise event log: {e}")))?;
    atomic_write(path, &bytes).map_err(|e| lock_error(e, path))?;

    Ok(json!({
        "verdict_recorded": true,
        "already_recorded": false,
        "stored": true,
        "intent": intent,
        "lenses": lens_verdicts_json,
        "aggregate": verdict_to_str(&aggregate),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a `--result` string to a [`Verdict`], case-insensitive.
fn parse_verdict(s: &str) -> Option<Verdict> {
    match s.to_ascii_lowercase().as_str() {
        "approve" => Some(Verdict::Approve),
        "fix_first" => Some(Verdict::FixFirst),
        "reject" => Some(Verdict::Reject),
        _ => None,
    }
}

/// The stable wire string for a verdict outcome.
fn verdict_to_str(v: &Verdict) -> &'static str {
    match v {
        Verdict::Approve => "approve",
        Verdict::FixFirst => "fix_first",
        Verdict::Reject => "reject",
    }
}

/// Aggregate: APPROVE when every lens approves, REJECT otherwise.
fn aggregate_verdict(lens_verdicts: &[(String, Verdict)]) -> Verdict {
    if lens_verdicts
        .iter()
        .all(|(_, v)| matches!(v, Verdict::Approve))
    {
        Verdict::Approve
    } else {
        Verdict::Reject
    }
}

/// A projected existing `verdict.recorded` record that matches this intent +
/// lens/result set exactly.
struct ExistingVerdict {
    lens_verdicts_json: Vec<Value>,
    aggregate_json: Value,
}

/// Project the most-recent `verdict.recorded` for `intent` from the log whose
/// `claims_checked` exactly matches the supplied `lens_verdicts` (same lens
/// names AND same results, same order).
///
/// Returns `Some` only on an exact match — a re-run with different verdicts is
/// a new recording.
fn find_existing_verdict(
    log: &hugit_refstore::EventLog,
    intent: &str,
    lens_verdicts: &[(String, Verdict)],
) -> Option<ExistingVerdict> {
    // The claims_checked format is "lens:result" per entry.
    let wanted: Vec<String> = lens_verdicts
        .iter()
        .map(|(name, v)| format!("{}:{}", name, verdict_to_str(v)))
        .collect();

    let existing: Option<VerdictObject> = log
        .records()
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .rfind(|vo| vo.intent == intent && vo.claims_checked == wanted);

    existing.map(|vo| {
        let lens_verdicts_json: Vec<Value> = vo
            .claims_checked
            .iter()
            .filter_map(|claim| {
                let mut parts = claim.splitn(2, ':');
                let lens = parts.next()?.to_string();
                let result = parts.next()?.to_string();
                Some(json!({ "lens": lens, "result": result }))
            })
            .collect();
        let aggregate_json = json!(verdict_to_str(&vo.verdict));
        ExistingVerdict {
            lens_verdicts_json,
            aggregate_json,
        }
    })
}

/// Map a [`LockError`] into a structured [`PorcelainError`].
fn lock_error(e: LockError, path: &std::path::Path) -> PorcelainError {
    match e {
        LockError::Busy { .. } => PorcelainError::new(
            "log_busy",
            format!(
                "the log file {} is locked by another hugit verb",
                path.display()
            ),
            "another `hugit` process holds the log lock; retry once it releases",
        ),
        LockError::Io { .. } => PorcelainError::new(
            "io",
            format!("I/O error on {}: {e}", path.display()),
            "check the --log path exists and is writable",
        ),
    }
}

#[cfg(test)]
mod recorder_tests {
    use super::*;
    use hugit_refstore::EventLog;

    fn make_log_with_verdict(intent: &str, lens: &str, result: &str) -> EventLog {
        use hugit_refstore::{Endpoint, PrincipalClass};
        let lens_verdict = parse_verdict(result).unwrap();
        let claims_checked = vec![format!("{}:{}", lens, verdict_to_str(&lens_verdict))];
        let agg = aggregate_verdict(&[(lens.to_string(), lens_verdict.clone())]);
        let vo = VerdictObject {
            intent: intent.to_string(),
            tree_hash: String::new(),
            lens: "panel".to_string(),
            model: "porcelain".to_string(),
            prompt_digest: "0".repeat(64),
            verdict: agg,
            claims_checked,
            evidence_refs: Vec::new(),
        };
        let payload = hugit_refstore::canonical_json(&serde_json::to_string(&vo).unwrap()).unwrap();
        let mut log = EventLog::new();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            VERDICT_RECORDED_KIND,
            vec!["orchestrator:test".to_string()],
            payload,
            0,
        )
        .unwrap();
        log
    }

    #[test]
    fn parse_verdict_all_variants() {
        assert_eq!(parse_verdict("approve"), Some(Verdict::Approve));
        assert_eq!(parse_verdict("APPROVE"), Some(Verdict::Approve));
        assert_eq!(parse_verdict("fix_first"), Some(Verdict::FixFirst));
        assert_eq!(parse_verdict("reject"), Some(Verdict::Reject));
        assert_eq!(parse_verdict("unknown"), None);
    }

    #[test]
    fn aggregate_approve_when_all_approve() {
        let lens_verdicts = vec![
            ("security".to_string(), Verdict::Approve),
            ("contracts".to_string(), Verdict::Approve),
        ];
        assert_eq!(aggregate_verdict(&lens_verdicts), Verdict::Approve);
    }

    #[test]
    fn aggregate_reject_on_any_non_approve() {
        let lens_verdicts = vec![
            ("security".to_string(), Verdict::Approve),
            ("contracts".to_string(), Verdict::Reject),
        ];
        assert_eq!(aggregate_verdict(&lens_verdicts), Verdict::Reject);

        let lens_verdicts_fix = vec![
            ("security".to_string(), Verdict::FixFirst),
            ("contracts".to_string(), Verdict::Approve),
        ];
        assert_eq!(aggregate_verdict(&lens_verdicts_fix), Verdict::Reject);
    }

    #[test]
    fn find_existing_verdict_matches_exact_set() {
        let log = make_log_with_verdict("intent-1", "security", "approve");
        let lens_verdicts = vec![("security".to_string(), Verdict::Approve)];
        assert!(find_existing_verdict(&log, "intent-1", &lens_verdicts).is_some());
    }

    #[test]
    fn find_existing_verdict_no_match_different_result() {
        let log = make_log_with_verdict("intent-1", "security", "approve");
        let lens_verdicts = vec![("security".to_string(), Verdict::Reject)];
        assert!(find_existing_verdict(&log, "intent-1", &lens_verdicts).is_none());
    }

    #[test]
    fn verdict_recorded_on_log() {
        let log = make_log_with_verdict("intent-1", "security", "approve");
        let verdicts: Vec<_> = log
            .records()
            .iter()
            .filter(|r| r.kind == VERDICT_RECORDED_KIND)
            .collect();
        assert_eq!(verdicts.len(), 1);
        let vo: VerdictObject =
            serde_json::from_str(&verdicts[0].payload).expect("valid VerdictObject payload");
        assert_eq!(vo.intent, "intent-1");
        assert_eq!(vo.verdict, Verdict::Approve);
    }
}
