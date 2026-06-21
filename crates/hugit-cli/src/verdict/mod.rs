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

use clap::Subcommand;
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

/// `hugit verdict <subcommand>` — the adversarial-verdict verb.
///
/// Git-proximity cleanup: the single-lens stakeholder decisions that used to be
/// the top-level `hugit approve` / `hugit reject` verbs now live UNDER `verdict`
/// (`verdict approve` / `verdict reject`), alongside the full multi-lens panel
/// (`verdict record`). One concept, one verb, three subcommands — no two extra
/// top-level tokens for what is a verdict.
#[derive(clap::Args, Debug)]
pub struct VerdictArgs {
    #[command(subcommand)]
    pub command: VerdictCommand,
}

/// The verdict subcommand surface — `record` (multi-lens panel) / `approve` /
/// `reject` (single-lens stakeholder decisions).
#[derive(Subcommand, Debug)]
pub enum VerdictCommand {
    /// Convene a multi-lens adversarial panel and record its aggregate verdict.
    Record(VerdictRecordArgs),
    /// Record a single-lens APPROVE verdict for an intent (stakeholder decision).
    Approve(DecisionArgs),
    /// Record a single-lens REJECT verdict for an intent (stakeholder decision).
    Reject(DecisionArgs),
}

/// `hugit verdict record` flags — convene a multi-lens adversarial panel for
/// `--intent` (one `--lens NAME --result approve|fix_first|reject` pair per lens,
/// positionally paired) and — with `--store` — record the aggregate
/// [`VerdictObject`] onto the canonical `--log` as a [`VERDICT_RECORDED_KIND`]
/// event the way `campaign show` reads it.
#[derive(clap::Args, Debug)]
pub struct VerdictRecordArgs {
    /// The intent / change id to convene the verdict panel over.
    #[arg(long)]
    pub intent: String,
    /// Path to the canonical JSON event log — the shared `--log` seam the
    /// recorded verdict is appended to and `campaign show` projects `proven` from.
    /// Defaults to $HUGIT_LOG, else .hugit/log.json.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
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
    match args.command {
        VerdictCommand::Record(a) => emit(record(a)),
        VerdictCommand::Approve(a) => run_approve(a),
        VerdictCommand::Reject(a) => run_reject(a),
    }
}

/// Emit a `Result<Value, PorcelainError>` as stable JSON on stdout under the one
/// error/exit law.
fn emit(result: Result<Value, PorcelainError>) -> ExitCode {
    match result {
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
fn record(args: VerdictRecordArgs) -> Result<Value, PorcelainError> {
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

    // ── Door the --tree-hash identifier (K-SCRUB) ─────────────────────────────
    // `--tree-hash` is a content-address identifier that is stamped into the
    // recorded verdict and reaches the forever-log. Route it through the SAME
    // structural-secret door as --id/--campaign/--run-id so a credential smuggled
    // as a tree-hash (`cas:ghp_…`) is rejected at input with a clear exit-2
    // (`secret_in_identifier`) BEFORE it is appended. The default empty
    // placeholder and a legitimate hex / `cas:<hex>` content address pass through;
    // the central scrub boundary remains the security backstop regardless.
    if !args.tree_hash.trim().is_empty() {
        crate::ident::validate_identifier(&args.tree_hash, "--tree-hash")
            .map_err(|e| PorcelainError::new(e.kind, e.message, e.fix))?;
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

    // ── Reject duplicate-lens-with-conflicting-result at the door (C5-F1) ──────
    // Defense-in-depth for the within-record lens-substitution launder: a single
    // call must not carry the SAME lens twice with DIFFERENT results
    // (`--lens security --result reject --lens security --result approve`). The
    // ledger fold is now reject-sticky within a record (the source of truth), but
    // we also refuse the conflicting input here so the record never stores a
    // self-contradictory `claims_checked` vector in the first place. A repeated
    // lens with the SAME result is harmless (idempotent) and allowed.
    {
        use std::collections::BTreeMap;
        let mut seen: BTreeMap<&str, &Verdict> = BTreeMap::new();
        for (name, v) in &lens_verdicts {
            if let Some(prev) = seen.get(name.as_str())
                && *prev != v
            {
                let safe_lens = crate::redaction::scrub(name);
                return Err(PorcelainError::new(
                    "duplicate_lens",
                    format!(
                        "lens '{safe_lens}' appears more than once in this call with \
                         conflicting --result values: a single verdict cannot both \
                         reject and approve the same lens"
                    ),
                    "pass each --lens at most once per call (or repeat it with the \
                     same --result); to revise a prior verdict, record it in a later call",
                )
                .with_context("lens", json!(safe_lens)));
            }
            seen.insert(name.as_str(), v);
        }
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
    let log_path = crate::log_resolve::resolve_log(args.log.clone());
    let path = &log_path;
    let _lock = FileLock::acquire(path).map_err(|e| lock_error(e, path))?;
    let log = load_event_log(path)?;

    // ── Intent existence guard (B2) ───────────────────────────────────────────
    // Validate the referenced intent exists on the log as an `intent.landed`
    // record, but ONLY when the log has intent vocabulary at all (at least one
    // `intent.landed` record). A log with no intent records stays permissive —
    // it may be a pre-intent log or a tests-only log that never lands intents.
    // A log that DOES carry intent.landed records but has none for this id is a
    // ghost: reject with `intent_not_found` / exit-2.
    let intent = &args.intent;
    let has_any_intent_landed = log
        .records()
        .iter()
        .any(|r| r.kind == hugit_refstore::intent::INTENT_LANDED_KIND);
    if has_any_intent_landed && !intent_is_on_log(&log, intent) {
        // The intent did not resolve, so it is echoed back in the error — route
        // it through the redaction engine first so a prefixed/JWT/PEM/conn-string
        // secret smuggled as `--intent` never lands raw in the error message or
        // context (WJ-VERDICT, Round-6 Cluster A leak). This is an UNRESOLVED id
        // (not a lookup key), so the full free-text engine is safe here — a real
        // address still survives for a genuine-typo diagnostic.
        let safe_intent = crate::redaction::scrub(intent);
        return Err(PorcelainError::new(
            "intent_not_found",
            format!(
                "no intent.landed record for intent '{safe_intent}' on the log: cannot record a \
                 verdict for a nonexistent intent"
            ),
            "run `hugit intent new --log <path> --id <id> …` first, or check the \
             --intent id is spelled correctly",
        )
        .with_context("intent", json!(safe_intent))
        .with_context("log", json!(path.display().to_string())));
    }

    // ── Post-seal append guard (K-VERDICT) ────────────────────────────────────
    // If the intent belongs to a campaign that has been sealed (`campaign.closed`
    // on the log), refuse to append a new verdict.  A sealed campaign is an
    // immutable audit trail fact; a post-close verdict revision would silently
    // mutate the projection AFTER the seal (the `sealed_with_rejected` payload
    // was already written at close time and would diverge from the live ledger).
    //
    // Resolution rule: look up the intent's campaign from its `intent.landed`
    // record.  If a `campaign.closed` event for that campaign is already on the
    // log, error `campaign_sealed` / exit-2.  A log with no `intent.landed`
    // records (pre-intent or tests-only logs) is permissive — the vocabulary is
    // absent, not wrong.
    if let Some(campaign_key) = campaign_for_intent(&log, intent) {
        // Route through the SHARED terminal-seal chokepoint (C5-F2): the same
        // `guard_not_sealed` every campaign-scoped mutation verb now calls. The
        // seal-detection logic lives once in `campaign::seal_guard`, so verdict,
        // intent, and pr cannot drift on what "sealed" means (the K-VERDICT guard
        // was point-local to this verb; it is now the shared precondition).
        if let Err(v) = crate::campaign::seal_guard::guard_not_sealed(&log, &campaign_key) {
            let safe_intent = crate::redaction::scrub(intent);
            let safe_campaign = crate::redaction::scrub(&v.campaign);
            return Err(PorcelainError::new(v.kind(), v.message(), v.fix())
                .with_context("intent", json!(safe_intent))
                .with_context("campaign", json!(safe_campaign))
                .with_context("log", json!(path.display().to_string())));
        }
    }

    // ── Idempotency check ─────────────────────────────────────────────────────
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
// `hugit approve` / `hugit reject` — single-decision convenience verbs (W3)
// ─────────────────────────────────────────────────────────────────────────────
//
// These are the stakeholder approve/reject verbs. They are thin, parity-faithful
// wrappers over the SAME [`record`] path `hugit verdict` uses: each records a
// single-lens `verdict.recorded` event with lens `"human-approval"` and result
// `approve` / `reject`. This mirrors the live serve verb `POST /prs/{n}/verdict`
// (which maps `approve` / `request-changes` into the same `verdict.recorded`
// kind through `(Orchestrator, Land)`), so the CLI and the web surface produce
// the identical wire fact — no web-only verb, no new D14 endpoint, no
// matrix change. They always store (an unrecorded approval is meaningless), so
// there is no `--store` flag.

/// The fixed lens label a stakeholder approve/reject is recorded under.
const STAKEHOLDER_LENS: &str = "human-approval";

/// Arguments for `hugit approve` / `hugit reject` (identical shape).
#[derive(clap::Args, Debug)]
pub struct DecisionArgs {
    /// The intent / change id to approve or reject.
    #[arg(long)]
    pub intent: String,
    /// Path to the canonical JSON event log the decision is appended to. Defaults
    /// to $HUGIT_LOG, else .hugit/log.json.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// Workspace Merkle tree hash of the reviewed snapshot (honest-default empty;
    /// the real hash is a P2 live-infra seam — same as `hugit verdict`).
    #[arg(long = "tree-hash", default_value = "")]
    pub tree_hash: String,
    /// Unix-ms timestamp to stamp the appended event with (0 = now / left as-is).
    #[arg(long = "recorded-at", default_value_t = 0)]
    pub recorded_at: u64,
}

/// `hugit verdict approve` — record a single-lens APPROVE verdict for `--intent`.
fn run_approve(args: DecisionArgs) -> ExitCode {
    run_decision(args, "approve")
}

/// `hugit verdict reject` — record a single-lens REJECT verdict for `--intent`.
fn run_reject(args: DecisionArgs) -> ExitCode {
    run_decision(args, "reject")
}

/// Build a single-lens [`VerdictArgs`] from a [`DecisionArgs`] + a fixed result
/// and route it through the shared [`record`] path — one producer, one wire kind.
fn run_decision(args: DecisionArgs, result: &str) -> ExitCode {
    let verdict_args = VerdictRecordArgs {
        intent: args.intent,
        log: args.log,
        store: true,
        lens: vec![STAKEHOLDER_LENS.to_string()],
        result: vec![result.to_string()],
        tree_hash: args.tree_hash,
        recorded_at: args.recorded_at,
    };
    match record(verdict_args) {
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

/// Dedup against the **LATEST** `verdict.recorded` for `intent` ONLY (WJ-VERDICT).
///
/// The campaign projection is **latest-wins**: the most-recent verdict for an
/// intent governs its `proven`/`rejected` state. The recorder's idempotency
/// MUST agree with that contract, so dedup is decided against the LATEST verdict
/// for the intent, never against all history:
///
/// - If the most-recent `verdict.recorded` for this intent has the SAME
///   `claims_checked` (same lens names AND results, same order) as the wanted
///   set, the call is a true idempotent no-op → `Some` (`already_recorded`).
/// - If the latest differs — even when an EARLIER verdict matched the wanted set
///   — this is a legitimate revision (including one that returns to a prior
///   state, e.g. `approve → reject → approve`) → `None` (APPEND, so latest-wins
///   is honored). The old `.rfind` over all history swallowed the final verdict:
///   `approve → reject → approve` matched the 1st approve and refused the 3rd,
///   leaving the ledger latest = reject → `proven:0` though the operator's final
///   action was approve (adversarial Round 6, Cluster B — reproduced).
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

    // The LATEST verdict.recorded for THIS intent (last in chain order), then
    // gate idempotency on it. dedup only when the most-recent verdict already
    // equals the wanted set; an earlier match with a differing latest is a
    // legitimate revision and MUST append.
    let latest: VerdictObject = log
        .records()
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .rfind(|vo| vo.intent == intent)?;

    if latest.claims_checked != wanted {
        return None;
    }

    let lens_verdicts_json: Vec<Value> = latest
        .claims_checked
        .iter()
        .filter_map(|claim| {
            let mut parts = claim.splitn(2, ':');
            let lens = parts.next()?.to_string();
            let result = parts.next()?.to_string();
            Some(json!({ "lens": lens, "result": result }))
        })
        .collect();
    let aggregate_json = json!(verdict_to_str(&latest.verdict));
    Some(ExistingVerdict {
        lens_verdicts_json,
        aggregate_json,
    })
}

/// Look up the campaign key for `intent_id` from the first `intent.landed`
/// record on the log that names it.  Returns `None` if no `intent.landed` record
/// exists for this id or if the payload has no `campaign` field (permissive —
/// the vocabulary is absent, not wrong).
///
/// Used by the post-seal guard (K-VERDICT): a verdict for an intent whose
/// campaign is sealed must be refused.
fn campaign_for_intent(log: &hugit_refstore::EventLog, intent_id: &str) -> Option<String> {
    log.records()
        .iter()
        .filter(|r| r.kind == hugit_refstore::intent::INTENT_LANDED_KIND)
        .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.payload).ok())
        .filter(|v| {
            v.get("intent_id")
                .and_then(serde_json::Value::as_str)
                .map(|id| id == intent_id)
                .unwrap_or(false)
        })
        .find_map(|v| {
            v.get("campaign")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
}

/// Check whether an `intent.landed` record for `intent_id` exists on the log.
///
/// Used by the existence guard (B2): a log that carries at least one
/// `intent.landed` record MUST also carry one for the requested intent, or the
/// verdict is refused with `intent_not_found`. A log with no `intent.landed`
/// records at all is permissive — the vocabulary is absent, not wrong.
fn intent_is_on_log(log: &hugit_refstore::EventLog, intent_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == hugit_refstore::intent::INTENT_LANDED_KIND)
        .any(|r| {
            serde_json::from_str::<serde_json::Value>(&r.payload)
                .ok()
                .and_then(|v| {
                    v.get("intent_id")
                        .and_then(serde_json::Value::as_str)
                        .map(|id| id == intent_id)
                })
                .unwrap_or(false)
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

    /// Append a second `verdict.recorded` onto an existing log (same machinery as
    /// the recorder) so a multi-verdict history can be built in-test.
    fn append_verdict(log: &mut EventLog, intent: &str, lens: &str, result: &str) {
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
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            VERDICT_RECORDED_KIND,
            vec!["orchestrator:test".to_string()],
            payload,
            0,
        )
        .unwrap();
    }

    /// WJ-VERDICT: dedup is decided against the LATEST verdict only. After
    /// approve→reject, a third approve must NOT dedup against the earlier approve
    /// (the latest is reject) — it is a legitimate revision and must append.
    #[test]
    fn find_existing_verdict_dedups_latest_only_not_history() {
        let mut log = make_log_with_verdict("intent-1", "security", "approve");
        append_verdict(&mut log, "intent-1", "security", "reject");
        // The wanted set is approve — it MATCHES the 1st record but the LATEST is
        // reject, so no idempotent match: the revision must append.
        let wanted = vec![("security".to_string(), Verdict::Approve)];
        assert!(
            find_existing_verdict(&log, "intent-1", &wanted).is_none(),
            "approve must NOT false-dedup against an earlier approve when the latest is reject"
        );
        // Conversely, the wanted set EQUAL to the latest (reject) IS idempotent.
        let wanted_reject = vec![("security".to_string(), Verdict::Reject)];
        assert!(
            find_existing_verdict(&log, "intent-1", &wanted_reject).is_some(),
            "a re-run equal to the LATEST verdict is a true idempotent no-op"
        );
    }

    /// Dedup is per-intent: the latest verdict for a DIFFERENT intent never
    /// governs this intent's idempotency.
    #[test]
    fn find_existing_verdict_is_per_intent() {
        let mut log = make_log_with_verdict("intent-1", "security", "approve");
        append_verdict(&mut log, "intent-2", "security", "reject");
        // intent-1's latest is still its approve, so an approve re-run dedups.
        let wanted = vec![("security".to_string(), Verdict::Approve)];
        assert!(find_existing_verdict(&log, "intent-1", &wanted).is_some());
    }

    /// A scratch log bootstrapped with one record (no `intent.landed`, so the
    /// existence guard stays permissive) for driving `record` end-to-end.
    fn scratch_log(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-decision-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("L.json");
        let mut el = EventLog::new();
        el.append_for_test("repo.init", vec!["orchestrator:hugit".to_string()], "{}", 0);
        std::fs::write(&path, serde_json::to_string_pretty(el.records()).unwrap()).unwrap();
        path
    }

    /// `hugit approve` records a single-lens `human-approval:approve` verdict that
    /// aggregates to approve — exercising the exact VerdictArgs run_approve builds.
    #[test]
    fn single_lens_approve_records_approve_verdict() {
        let path = scratch_log("approve");
        let v = record(VerdictRecordArgs {
            intent: "i1".to_string(),
            log: Some(path.clone()),
            store: true,
            lens: vec![STAKEHOLDER_LENS.to_string()],
            result: vec!["approve".to_string()],
            tree_hash: String::new(),
            recorded_at: 0,
        })
        .expect("approve records");
        assert_eq!(v["aggregate"], "approve");
        assert_eq!(v["stored"], true);
        let recs: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let vr = recs
            .iter()
            .find(|r| r["kind"] == VERDICT_RECORDED_KIND)
            .expect("verdict.recorded appended");
        let vo: VerdictObject = serde_json::from_str(vr["payload"].as_str().unwrap()).unwrap();
        assert_eq!(vo.verdict, Verdict::Approve);
        assert_eq!(
            vo.claims_checked,
            vec!["human-approval:approve".to_string()]
        );
    }

    /// `hugit reject` records a single-lens `human-approval:reject` verdict that
    /// aggregates to reject.
    #[test]
    fn single_lens_reject_records_reject_verdict() {
        let path = scratch_log("reject");
        let v = record(VerdictRecordArgs {
            intent: "i1".to_string(),
            log: Some(path.clone()),
            store: true,
            lens: vec![STAKEHOLDER_LENS.to_string()],
            result: vec!["reject".to_string()],
            tree_hash: String::new(),
            recorded_at: 0,
        })
        .expect("reject records");
        assert_eq!(v["aggregate"], "reject");
        let recs: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let vr = recs
            .iter()
            .find(|r| r["kind"] == VERDICT_RECORDED_KIND)
            .expect("verdict.recorded appended");
        let vo: VerdictObject = serde_json::from_str(vr["payload"].as_str().unwrap()).unwrap();
        assert_eq!(vo.verdict, Verdict::Reject);
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
