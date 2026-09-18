//! `hugit land --queue <log>` — the batch land: run the REAL union-test +
//! bisect + memoize engine over the queued PRs of a campaign/batch.
//!
//! # The missing front door (the wedge made invocable)
//!
//! The union-testing landing engine ([`hugit_queue::core::evaluate_union`] +
//! [`hugit_queue::core::union`]'s `bisect_failure`) and the memoized executor
//! ([`hugit_checks::client::executor::run_memoized`]) both already exist and are
//! hermetically tested — but the only thing that ever drove the engine end to
//! end was the in-process dogfood `WaveOracle`. There was no operator-invocable
//! verb. `hugit pr land` settles a SINGLE PR terminal and explicitly does NOT
//! run the union verdict (the disclosed P2 seam).
//!
//! This verb is that front door. Given the set of queued PRs for a campaign (or
//! the whole queue), it:
//!
//! 1. folds them into a [`hugit_queue::core::Batch`];
//! 2. runs [`evaluate_union_with_limits`] over their union through a REAL [`MemoCheck`]
//!    oracle ([`LogMemoOracle`]) backed by [`run_memoized`] + a file-backed
//!    Action Cache (the exact [`FileAc`] seam `hugit check run --store` uses);
//! 3. on a GREEN union, lands every member (appends `pr.landed` per PR);
//! 4. on a RED union, `evaluate_union` bisects to the minimal failing locus;
//!    the green remainder lands, the culprit is EXCLUDED, and a
//!    `queue.union_fail` record is appended carrying the bisected pair (so
//!    `queue show`'s `failing_pair` lights up).
//!
//! # Scope: simulation only
//!
//! The local oracle memoizes synthetic per-PR content and a declared conflict
//! relation. It does NOT build, execute checks on, or promote a real Git union
//! tree. The JSON result explicitly declares this boundary; real managed-ref
//! promotion remains HUG-043. A red remainder stays held, not landed.
//!
//! # X5 namespace law
//!
//! `land` is a reserved top-level verb (git has neither `land` nor `queue`), so
//! it does not shadow a git builtin — the WP-X5 namespace oracle stays green.

#[cfg(test)]
use hugit_queue::core::union::evaluate_union;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Subcommand;
use hugit_checks::client::ac::ActionCache;
use hugit_checks::client::executor::{CheckRunner, ExecError, run_memoized};
use hugit_checks::client::memo_key::{FileContent, compute_def_digest};
use hugit_contracts::{CheckDef, CheckResult, LandableEntry, MinimalFailingPair};
use hugit_queue::core::affected::AffectedSet;
use hugit_queue::core::batch::Batch;
use hugit_queue::core::union::{
    CheckSource, EvaluationLimits, EvaluationStop, FailureLocus, MemoCheck, UnionEvaluation,
    UnionVerdict, evaluate_union_with_limits,
};
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::{Value, json};

use crate::porcelain::PorcelainError;
use crate::pr::{LANDING_MODE, OpenedPr, QueuedPr, all_pr_queued, find_pr_opened};

/// Event kind a batch land appends when a union test localises a failure: it
/// carries the bisected minimal failing pair (or the single culprit) so the
/// `queue show` projection's `failing_pair` field lights up. Additive over the
/// canonical log, sibling to `pr.queued` / `pr.landed`.
///
/// Payload (canonical JSON):
/// `{"campaign","mode":"union","locus":"pair|single|unlocalised",
///   "item_a","item_b","excluded":[…],"proceeding":[…]}`.
/// `item_a`/`item_b` are the bisected pair members (a single-item failure puts
/// the culprit in `item_a`, `item_b` null); `excluded` is the held set, and
/// `proceeding` the green set that landed.
pub const QUEUE_UNION_FAIL_KIND: &str = "queue.union_fail";

/// The toolchain digest the local batch-land memoized executor keys checks on.
///
/// A safe-identifier slug (the AC write-boundary guard refuses a secret-shaped
/// axis). It identifies this local simulation only, not a qualified native toolchain.
const LOCAL_TOOLCHAIN_DIGEST: &str = "local-union-test-v1";

// ─────────────────────────────────────────────────────────────────────────────
// CLI surface
// ─────────────────────────────────────────────────────────────────────────────

/// `hugit land <subcommand>` — the batch landing surface.
#[derive(clap::Args, Debug)]
pub struct LandArgs {
    #[command(subcommand)]
    pub command: LandCommand,
}

/// The land subcommand surface — `queue` (batch land via the union engine).
#[derive(Subcommand, Debug)]
pub enum LandCommand {
    /// Batch-land the queued PRs: run the union-test + bisect + memoize engine.
    Queue(QueueLandArgs),
}

/// `hugit land queue` flags — run the real union engine over the queue.
#[derive(clap::Args, Debug)]
pub struct QueueLandArgs {
    /// Path to the canonical JSON event log. Defaults to $HUGIT_LOG, else
    /// .hugit/log.json. The batch is read from its `pr.queued` records and the
    /// `pr.landed` / `queue.union_fail` outcomes are appended back.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// Optional campaign key: scope the batch to one campaign's queued PRs.
    /// Omit to batch-land the WHOLE active queue as one union.
    #[arg(long)]
    pub campaign: Option<String>,
    /// The file-backed Action Cache path. Defaults to `<log>.ac` (the SAME seam
    /// `hugit check run --store` uses) so a warm re-run is a real cross-process
    /// HIT. No remote cache is contacted by this command.
    #[arg(long)]
    pub ac: Option<PathBuf>,
    /// Unix-ms timestamp to stamp the appended `pr.landed` / `queue.union_fail`
    /// events with.
    #[arg(long = "recorded-at", default_value_t = 0)]
    pub recorded_at: u64,
    /// Maximum diagnostic probes, INCLUDING cache hits and remainder validation.
    #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u32).range(0..=64))]
    pub max_probes: u32,
    /// Cooperative in-process evaluation deadline (ms); not an OS kill timeout.
    #[arg(long, default_value_t = 30000, value_parser = clap::value_parser!(u64).range(0..=300000))]
    pub evaluation_timeout_ms: u64,
}

/// Dispatch a `land` subcommand. Returns the process exit code directly under
/// the one-exit-code law.
pub fn run(args: LandArgs) -> ExitCode {
    match args.command {
        LandCommand::Queue(a) => run_queue_land(a),
    }
}

fn run_queue_land(a: QueueLandArgs) -> ExitCode {
    let log_path = match crate::log_resolve::resolve_log(a.log.clone()) {
        Ok(path) => path,
        Err(error) => return emit(Err(error)),
    };
    let ac_path =
        a.ac.clone()
            .unwrap_or_else(|| crate::checks::run::default_ac_path(&log_path));

    // Hold the advisory exclusive lock across load→mutate→persist (WP-WC1): a
    // concurrent mutating verb on the same `--log` serialises or fails
    // `log_busy`, never clobbers.
    let _lock = match crate::pr::filelock::FileLock::acquire(&log_path) {
        Ok(lock) => lock,
        Err(e) => return emit(Err(lock_error(&log_path, &e))),
    };
    let mut log = match crate::checks::load_event_log(&log_path) {
        Ok(log) => log,
        Err(e) => return emit(Err(e)),
    };

    // The file-backed local AC is the only backend used by this command.
    // `--ac` overrides the default path; no remote service is contacted.
    let ac = FileAc::new(ac_path);
    let result = batch_land_with_limits(
        &mut log,
        &ac,
        a.campaign.as_deref(),
        a.recorded_at,
        EvaluationLimits {
            max_probes: a.max_probes as usize,
            timeout: Duration::from_millis(a.evaluation_timeout_ms),
        },
    );
    match result {
        Ok(value) if value.get("stop_reason").is_some_and(|v| !v.is_null()) => {
            // A held evaluation is visible and nonzero, with NO log rewrite.
            emit(Err(PorcelainError::new("evaluation_held", "queue evaluation is inconclusive",
                "inspect evaluation.stop_reason; retry as a new evaluation after resolving the cause")
                .with_context("evaluation", value)))
        }
        Ok(value) => match crate::pr::filelock::atomic_write(
            &log_path,
            serde_json::to_string_pretty(log.records())
                .unwrap_or_default()
                .as_bytes(),
        ) {
            Ok(()) => emit(Ok(value)),
            Err(e) => emit(Err(PorcelainError::new(
                "io_error",
                format!("write log {log_path:?}: {e}"),
                "check the --log path is on a writable filesystem with sufficient space",
            ))),
        },
        Err(e) => emit(Err(e)),
    }
}

fn lock_error(path: &std::path::Path, e: &crate::pr::filelock::LockError) -> PorcelainError {
    match e {
        crate::pr::filelock::LockError::Busy { .. } => PorcelainError::new(
            "log_busy",
            format!("the --log file {path:?} is locked by another hugit verb"),
            "another `hugit` process holds the log lock; retry once it releases",
        ),
        crate::pr::filelock::LockError::Io { .. } => PorcelainError::new(
            "io_error",
            format!("lock log {path:?}: {e}"),
            "check the --log path is on a writable filesystem",
        ),
    }
}

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

// ─────────────────────────────────────────────────────────────────────────────
// The batch-land engine driver (library — testable without the CLI shell).
// ─────────────────────────────────────────────────────────────────────────────

/// Run the real union-test + bisect + memoize engine over the queued PRs of a
/// `campaign` (or the whole active queue when `campaign` is `None`), landing the
/// green set and excluding + recording the bisected failure.
///
/// This is the LIBRARY core (the CLI shell is the thin wrapper above): it takes
/// the live `log`, an [`ActionCache`] backend, and drives:
///
/// - [`all_pr_queued`] → the ordered active queue (already filters settled PRs);
/// - a [`Batch`] of [`LandableEntry`]s, fed to [`evaluate_union_with_limits`] over a
///   [`LogMemoOracle`] (REAL memoized execution, AC-backed);
/// - on the resulting [`UnionEvaluation`]: append `pr.landed` for each
///   proceeding PR, and on a localised red union append one `queue.union_fail`
///   carrying the bisected pair.
///
/// Returns the stable JSON summary: `{queued, landed:[…], excluded:[…],
/// verdict, failing_pair, executed_count}`.
pub fn batch_land<A: ActionCache>(
    log: &mut EventLog,
    ac: &A,
    campaign: Option<&str>,
    recorded_at: u64,
) -> Result<Value, PorcelainError> {
    batch_land_with_limits(log, ac, campaign, recorded_at, EvaluationLimits::default())
}

/// Bounded simulation. Inconclusive attempts preserve the input log; staged
/// transitions commit only after the last cooperative deadline check.
pub fn batch_land_with_limits<A: ActionCache>(
    log: &mut EventLog,
    ac: &A,
    campaign: Option<&str>,
    recorded_at: u64,
    limits: EvaluationLimits,
) -> Result<Value, PorcelainError> {
    // The active queue, in queue (order_index) order, scoped to the campaign.
    let mut queued = all_pr_queued(log);
    queued.sort_by_key(|q| q.order_index);

    // Resolve each queued PR's owning `pr.opened` (campaign + intents). A queued
    // PR with no `pr.opened` is a corrupt log — skip it (never guessed).
    let members: Vec<(QueuedPr, OpenedPr)> = queued
        .into_iter()
        .filter_map(|q| find_pr_opened(log, &q.pr_id).map(|o| (q, o)))
        .filter(|(_, o)| campaign.is_none_or(|c| o.campaign == c))
        .collect();

    if members.is_empty() {
        return Ok(json!({
            "queued": 0,
            "campaign": campaign,
            "mode": LANDING_MODE,
            "verdict": "green",
            "landed": [],
            "excluded": [],
            "held": [],
            "validation_scope": "simulation_only",
            "git_tree_verified": false,
            "remainder_verdict": Value::Null,
            "failing_pair": Value::Null,
            "executed_count": 0,
            "execution_count_complete": true,
            "probe_count": 0,
            "stop_reason": Value::Null,
            "evaluation_budget": {"max_probes": limits.max_probes,
                "timeout_ms": limits.timeout.as_millis(), "deadline_enforcement": "cooperative_in_process"},
            "note": "no queued PRs to batch-land (empty queue or campaign scope matched nothing)",
        }));
    }

    // The union batch (one campaign = one union-testing unit). The affected-set
    // at this altitude is the honest placeholder (disjointness over real tree
    // hashes is the engine queue's job under P2); the union VERDICT — what this
    // verb runs — is driven by the memoized oracle over per-PR content.
    let entries: Vec<(LandableEntry, AffectedSet)> = members
        .iter()
        .map(|(q, o)| {
            (
                LandableEntry {
                    item_id: q.pr_id.clone(),
                    // A PR's member id: the first intent, else the first
                    // captured commit (W2: a commits-only PR must still land —
                    // its raw-commit members ARE the content).
                    intent_id: o
                        .intent_ids
                        .first()
                        .or_else(|| o.commit_ids.first())
                        .cloned()
                        .unwrap_or_default(),
                    tree_hash: String::new(),
                    order_index: q.order_index,
                },
                AffectedSet::new(Vec::<String>::new()),
            )
        })
        .collect();
    let batch = Batch::from_entries("land-queue", entries);

    // Per-PR content for the memoized executor: deterministic, derived from the
    // PR's bundled intent ids. Same PR → same content → same memo key → an AC
    // HIT on re-run (the wedge). The oracle owns the run_memoized wire.
    let content: BTreeMap<String, Vec<String>> = members
        .iter()
        .map(|(_, o)| {
            // The PR's full content for the memo/union oracle: intents + the
            // captured commits (W2). An intent-less PR is NOT empty — its
            // commits are the content.
            let mut ids = o.intent_ids.clone();
            ids.extend(o.commit_ids.iter().cloned());
            (o.pr_id.clone(), ids)
        })
        .collect();

    // The conflict relation, derived HONESTLY from PR content: an intent of the
    // shape `conflicts-with:<pr_id>` declares that this PR's union with that PR
    // is red (a pair interaction neither PR exhibits alone). This is the
    // local simulation of a pair conflict, not a verdict on a Git tree.
    let conflicts = conflict_pairs(&content);

    let check_def = local_check_def();
    let mut oracle = LogMemoOracle {
        ac,
        content: &content,
        conflicts: &conflicts,
        check_def: &check_def,
    };

    let evaluation = evaluate_union_with_limits(&batch, &mut oracle, limits);
    let ids: Vec<&str> = members.iter().map(|(q, _)| q.pr_id.as_str()).collect();

    if let Some(reason) = evaluation.stop_reason {
        return Ok(held_summary(&evaluation, campaign, &ids, reason));
    }
    if evaluation.deadline_exceeded() {
        return Ok(held_summary(
            &evaluation,
            campaign,
            &ids,
            EvaluationStop::DeadlineExceeded,
        ));
    }
    let mut staged = log.clone();
    // Land the proceeding (green) set: append a terminal `pr.landed` per PR,
    // routed through the SAME D14-guarded append the `pr` porcelain uses.
    let proceeding: std::collections::BTreeSet<&str> = evaluation
        .validated_proceeding()
        .iter()
        .map(String::as_str)
        .collect();
    let mut landed: Vec<String> = Vec::new();
    for (q, o) in &members {
        if proceeding.contains(q.pr_id.as_str()) {
            land_one(&mut staged, o, recorded_at)?;
            landed.push(q.pr_id.clone());
        }
    }

    // Only the diagnosed locus is excluded. The unvalidated remainder stays
    // held; absence from proceeding does not accuse it of the diagnosed failure.
    let excluded = evaluation.validated_excluded().to_vec();

    // On a RED union, record the bisected failure so `queue show` lights up.
    let failing_pair_json = match (&evaluation.verdict, &evaluation.failure) {
        (UnionVerdict::Red, Some(locus)) => {
            record_union_fail(
                &mut staged,
                campaign,
                locus,
                &excluded,
                &landed,
                &evaluation.held,
                recorded_at,
            )?;
            failing_pair_to_json(&evaluation.minimal_failing_pair)
        }
        _ => Value::Null,
    };

    if evaluation.deadline_exceeded() {
        return Ok(held_summary(
            &evaluation,
            campaign,
            &ids,
            EvaluationStop::DeadlineExceeded,
        ));
    }
    *log = staged;
    Ok(json!({
        "queued": members.len(),
        "campaign": campaign,
        "mode": LANDING_MODE,
        "verdict": evaluation.verdict.as_str(),
        "landed": landed,
        "excluded": excluded,
        "held": evaluation.held,
        "validation_scope": "simulation_only",
        "git_tree_verified": false,
        "remainder_verdict": evaluation.remainder_verdict.map(UnionVerdict::as_str),
        "failing_pair": failing_pair_json,
        "locus": locus_kind(&evaluation),
        "executed_count": evaluation.executed_count,
        "execution_count_complete": evaluation.execution_count_complete,
        "probe_count": evaluation.probe_count,
        "stop_reason": Value::Null,
        "evaluation_budget": evaluation_budget(&evaluation),
    }))
}

fn evaluation_budget(ev: &UnionEvaluation) -> Value {
    json!({"max_probes": ev.max_probes, "timeout_ms": ev.timeout.as_millis(),
           "elapsed_ms": ev.elapsed.as_millis(), "deadline_enforcement": "cooperative_in_process"})
}

fn held_summary(
    ev: &UnionEvaluation,
    campaign: Option<&str>,
    ids: &[&str],
    reason: EvaluationStop,
) -> Value {
    json!({"queued":ids.len(), "campaign":campaign, "mode":LANDING_MODE,
           "verdict":ev.verdict.as_str(), "landed":[], "excluded":[], "held":ids,
           "remainder_verdict":ev.remainder_verdict.map(UnionVerdict::as_str),
           "failing_pair":failing_pair_to_json(&ev.minimal_failing_pair),
           "locus":locus_kind(ev), "stop_reason":reason.as_str(),
           "executed_count":ev.executed_count, "execution_count_complete":ev.execution_count_complete,
           "probe_count":ev.probe_count, "evaluation_budget":evaluation_budget(ev),
           "validation_scope":"simulation_only", "git_tree_verified":false})
}

/// Append one terminal `pr.landed` for a proceeding PR, idempotently (a PR the
/// log already settles as landed is skipped). Routed through the D14-guarded
/// append under the opened PR's author class (the SAME seam `pr land --settle`
/// uses), so a non-author class is denied + audited, never appended.
fn land_one(log: &mut EventLog, opened: &OpenedPr, recorded_at: u64) -> Result<(), PorcelainError> {
    // Idempotent: skip a PR already recorded as landed.
    if log
        .records()
        .iter()
        .filter(|r| r.kind == "pr.landed")
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(opened.pr_id.as_str()))
    {
        return Ok(());
    }
    let payload = crate::porcelain::scrub_to_canonical(json!({
        "campaign": opened.campaign,
        "pr_id": opened.pr_id,
    }));
    let (class, endpoint) = author_authz(opened.author_kind);
    log.append_authorized(class, endpoint, "pr.landed", vec![], payload, recorded_at)
        .map_err(|denied| {
            PorcelainError::new(
                "authz_denied",
                format!(
                    "landing pr '{}' denied: {}",
                    opened.pr_id,
                    denied.reason.code()
                ),
                "a pr.landed is authored by the opened PR's author class (orchestrator/human)",
            )
        })?;
    Ok(())
}

/// Append the diagnosed locus, keeping the held remainder distinct.
fn record_union_fail(
    log: &mut EventLog,
    campaign: Option<&str>,
    locus: &FailureLocus,
    excluded: &[String],
    proceeding: &[String],
    held: &[String],
    recorded_at: u64,
) -> Result<(), PorcelainError> {
    let (locus_kind, item_a, item_b) = match locus {
        FailureLocus::Pair(p) => ("pair", Some(p.item_a.clone()), Some(p.item_b.clone())),
        FailureLocus::SingleItem(id) => ("single", Some(id.clone()), None),
        FailureLocus::Unlocalised => ("unlocalised", None, None),
    };
    let payload = crate::porcelain::scrub_to_canonical(json!({
        "campaign": campaign,
        "mode": LANDING_MODE,
        "locus": locus_kind,
        "item_a": item_a,
        "item_b": item_b,
        "excluded": excluded,
        "proceeding": proceeding,
        "held": held,
        "validation_scope": "simulation_only",
    }));
    // A union-fail recording is orchestrator/CI provenance — route under
    // Orchestrator/Land (the same matrix cell `pr.landed` uses).
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        QUEUE_UNION_FAIL_KIND,
        vec!["orchestrator:hugit".to_string()],
        payload,
        recorded_at,
    )
    .map_err(|denied| {
        PorcelainError::new(
            "authz_denied",
            format!("recording union-fail denied: {}", denied.reason.code()),
            "a queue.union_fail is orchestrator/CI provenance",
        )
    })?;
    Ok(())
}

/// Map the bisected pair into the `failing_pair` JSON `queue show` reads.
fn failing_pair_to_json(pair: &Option<MinimalFailingPair>) -> Value {
    match pair {
        Some(p) => json!({ "item_a": p.item_a, "item_b": p.item_b }),
        None => Value::Null,
    }
}

/// The locus kind string for the summary JSON.
fn locus_kind(ev: &UnionEvaluation) -> Value {
    match &ev.failure {
        None => Value::Null,
        Some(FailureLocus::Pair(_)) => json!("pair"),
        Some(FailureLocus::SingleItem(_)) => json!("single"),
        Some(FailureLocus::Unlocalised) => json!("unlocalised"),
    }
}

/// Map an [`crate::pr::AuthorKind`] to the D14 `(class, endpoint)` the
/// `pr.landed` append is gated on — mirrors `pr::author_authz`.
fn author_authz(kind: crate::pr::AuthorKind) -> (PrincipalClass, Endpoint) {
    match kind {
        crate::pr::AuthorKind::Orchestrator => (PrincipalClass::Orchestrator, Endpoint::Land),
        crate::pr::AuthorKind::Human => (PrincipalClass::Human, Endpoint::Undo),
    }
}

/// Build the local batch-land [`CheckDef`] (the gate the union is tested under).
fn local_check_def() -> CheckDef {
    let mut def = CheckDef {
        def_digest: String::new(),
        command: "hugit union-test".to_string(),
        inputs: vec![],
        toolchain_ref: "local".to_string(),
        env_manifest: String::new(),
        glob_set: vec!["**".to_string()],
    };
    def.def_digest = compute_def_digest(&def);
    def
}

// ─────────────────────────────────────────────────────────────────────────────
// The REAL MemoCheck → run_memoized wire (single-tenant local altitude).
// ─────────────────────────────────────────────────────────────────────────────

/// A [`MemoCheck`] oracle that drives the REAL memoized executor
/// ([`run_memoized`]) against a file-backed [`ActionCache`] — the wire from the
/// union engine to the executor + AC that was the missing front door.
///
/// For each PR in a probed union it derives the PR's content deterministically
/// from its bundled intent ids (so the same PR keys the same memo key → an AC
/// HIT on re-run), runs the memoized check via [`run_memoized`], and accumulates
/// the per-check [`CheckSource`] (hit/executed). The memoized per-PR check
/// proves the wedge: re-evaluating the same union is zero-execution.
///
/// The union VERDICT layers a real PAIR-interaction relation on top of the
/// per-PR checks: a union is RED if any individual member check fails OR if the
/// probed set contains BOTH members of a declared conflict pair (see
/// [`conflict_pairs`]). A pair interaction is precisely the case where each
/// member is individually GREEN but their union is RED — the failing pair the
/// engine's `bisect_failure` isolates.
///
/// # Honest scope (single-tenant local operator)
///
/// The per-PR check execution is the deterministic local executor (it passes,
/// exit 0 — the local stand-in for "this PR's own checks are green"); the
/// pair-conflict relation is the local stand-in for "the runner reported a red
/// union on this pair". This is only a simulator; real Git integration is HUG-043.
/// The cooperative deadline bounds result acceptance, not arbitrary blocking I/O.

struct LogMemoOracle<'a, A: ActionCache> {
    ac: &'a A,
    /// pr_id → its bundled intent ids (the deterministic content source).
    content: &'a BTreeMap<String, Vec<String>>,
    /// The set of declared conflict pairs (unordered) — a probed union
    /// containing both members of any pair is red.
    conflicts: &'a std::collections::BTreeSet<(String, String)>,
    check_def: &'a CheckDef,
}

impl<'a, A: ActionCache> MemoCheck for LogMemoOracle<'a, A> {
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        self.evaluate_until(item_ids, None)
    }
    fn evaluate_before(
        &mut self,
        item_ids: &[&str],
        deadline: Instant,
    ) -> (UnionVerdict, Vec<CheckSource>) {
        self.evaluate_until(item_ids, Some(deadline))
    }
}

impl<A: ActionCache> LogMemoOracle<'_, A> {
    fn evaluate_until(
        &mut self,
        item_ids: &[&str],
        deadline: Option<Instant>,
    ) -> (UnionVerdict, Vec<CheckSource>) {
        let mut any_red = false;
        let mut sources = Vec::with_capacity(item_ids.len());

        for pr_id in item_ids {
            if deadline.is_some_and(|until| Instant::now() >= until) {
                return (UnionVerdict::Unknown, sources);
            }
            let Some(intents) = self.content.get(*pr_id) else {
                return (UnionVerdict::Unknown, sources);
            };
            // The PR's content tree: one file per bundled intent (content = the
            // intent id), else a single file named for the PR. Deterministic ⇒
            // stable memo key ⇒ an AC HIT on re-run (the wedge).
            let files: Vec<(String, FileContent)> = if intents.is_empty() {
                vec![(format!("src/{pr_id}.txt"), pr_id.as_bytes().to_vec())]
            } else {
                intents
                    .iter()
                    .map(|i| (format!("src/{pr_id}/{i}.txt"), i.as_bytes().to_vec()))
                    .collect()
            };
            let file_iter: Vec<(&str, &FileContent)> =
                files.iter().map(|(k, v)| (k.as_str(), v)).collect();

            let runner = LocalRunner {
                pr_id: pr_id.to_string(),
            };
            let outcome = match run_memoized(
                self.ac,
                &runner,
                self.check_def,
                file_iter,
                LOCAL_TOOLCHAIN_DIGEST,
            ) {
                Ok(outcome) => outcome,
                // A cache/runner failure is NOT a red check and must not blame
                // an author or fabricate a completed execution count.
                Err(_) => return (UnionVerdict::InfrastructureFailure, sources),
            };

            if outcome.result.exit != 0 {
                any_red = true;
            }
            sources.push(if outcome.from_cache {
                CheckSource::Hit
            } else {
                CheckSource::Executed
            });
        }

        // Pair-interaction layer: the probed union is red if it contains BOTH
        // members of any declared conflict pair (each green alone — a genuine
        // pair the engine bisects to).
        if !any_red {
            let present: std::collections::BTreeSet<&str> = item_ids.iter().copied().collect();
            for (a, b) in self.conflicts {
                if present.contains(a.as_str()) && present.contains(b.as_str()) {
                    any_red = true;
                    break;
                }
            }
        }

        let verdict = if any_red {
            UnionVerdict::Red
        } else {
            UnionVerdict::Green
        };
        (verdict, sources)
    }
}

/// The local deterministic check runner for the batch-land oracle — a PURE
/// function of its inputs that always passes (exit 0). The per-PR check is
/// genuinely memoized through [`run_memoized`] (the wedge); the union's RED
/// verdict, when any, comes from the oracle's pair-conflict relation, not from a
/// faked per-PR failure. This fixture does not execute a user's build or tests.
struct LocalRunner {
    pr_id: String,
}

impl CheckRunner for LocalRunner {
    fn run(
        &self,
        _def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, ExecError> {
        Ok(CheckResult {
            memo_key: memo_key.to_string(),
            tree_hash: tree_root.to_string(),
            def_digest: def_digest.to_string(),
            toolchain_digest: toolchain_digest.to_string(),
            exit: 0,
            artifacts: vec![],
            stdout_ref: format!("local-stdout-{}", self.pr_id),
            stderr_ref: String::new(),
            duration_ms: 1,
            runner_ref: "local-union-test".to_string(),
            produced_at: 0,
        })
    }
}

/// Derive the declared conflict pairs from PR content. A PR whose bundled
/// intents include one of the shape `conflicts-with:<other_pr_id>` declares an
/// unordered conflict pair `(this_pr, other_pr)` — the local stand-in for "the
/// union of these two PRs is red". Only pairs where BOTH PRs are in the batch
/// are kept (a conflict with an absent PR is inert). Pairs are stored
/// canonically (sorted) so the relation is symmetric.
fn conflict_pairs(
    content: &BTreeMap<String, Vec<String>>,
) -> std::collections::BTreeSet<(String, String)> {
    let mut pairs = std::collections::BTreeSet::new();
    for (pr_id, intents) in content {
        for intent in intents {
            if let Some(other) = intent.strip_prefix("conflicts-with:")
                && content.contains_key(other)
                && other != pr_id
            {
                let (a, b) = if pr_id.as_str() <= other {
                    (pr_id.clone(), other.to_string())
                } else {
                    (other.to_string(), pr_id.clone())
                };
                pairs.insert((a, b));
            }
        }
    }
    pairs
}

// ─────────────────────────────────────────────────────────────────────────────
// File-backed AC (re-uses the same on-disk shape `hugit check run` writes).
// ─────────────────────────────────────────────────────────────────────────────

use crate::checks::run::FileAc;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod remainder_safety_tests {
    use super::*;
    use hugit_checks::client::ac::InMemoryAc;

    fn seed(log: &mut EventLog, id: &str, intents: &[&str], order: u64) {
        log.append_for_test(
            "pr.opened",
            vec!["orchestrator:hugit".into()],
            json!({"pr_id":id,"campaign":"remainder","author_kind":"orchestrator",
                   "run_id":"fixture","principal":null,"intent_ids":intents})
            .to_string(),
            0,
        );
        log.append_for_test("pr.queued", vec!["orchestrator:hugit".into()],
            json!({"pr_id":id,"item_id":format!("{id}#{order}"),"order_index":order,"mode":"union"}).to_string(), 0);
    }

    #[test]
    fn independent_conflicts_keep_remainder_queued_without_false_landing() {
        let mut log = EventLog::new();
        seed(&mut log, "A", &["conflicts-with:B"], 0);
        seed(&mut log, "B", &["content-b"], 1);
        seed(&mut log, "C", &["conflicts-with:D"], 2);
        seed(&mut log, "D", &["content-d"], 3);
        let before = serde_json::to_value(log.records()).unwrap();
        let result = batch_land(&mut log, &InMemoryAc::new(), Some("remainder"), 100).unwrap();
        assert_eq!(result["landed"], json!([]));
        assert_eq!(result["excluded"], json!(["A", "B"]));
        assert_eq!(result["held"], json!(["C", "D"]));
        assert_eq!(result["remainder_verdict"], "red");
        assert_eq!(result["validation_scope"], "simulation_only");
        assert_eq!(result["git_tree_verified"], false);
        assert!(!log.records().iter().any(|r| r.kind == "pr.landed"));
        assert_eq!(all_pr_queued(&log).len(), 4);
        assert_eq!(serde_json::to_value(&log.records()[..8]).unwrap(), before);
        let event: Value = serde_json::from_str(&log.records().last().unwrap().payload).unwrap();
        assert_eq!(event["excluded"], json!(["A", "B"]));
        assert_eq!(event["held"], json!(["C", "D"]));
        assert_eq!(event["proceeding"], json!([]));
    }

    #[test]
    fn validated_remainder_is_reported_separately_from_the_original_red_union() {
        let mut log = EventLog::new();
        seed(&mut log, "A", &["conflicts-with:B"], 0);
        seed(&mut log, "B", &["content-b"], 1);
        seed(&mut log, "C", &["content-c"], 2);
        let result = batch_land(&mut log, &InMemoryAc::new(), Some("remainder"), 100).unwrap();
        assert_eq!(result["verdict"], "red");
        assert_eq!(result["remainder_verdict"], "green");
        assert_eq!(result["landed"], json!(["C"]));
        assert_eq!(result["held"], json!([]));
        assert_eq!(result["validation_scope"], "simulation_only");
        assert_eq!(result["git_tree_verified"], false);
    }

    struct BrokenCache;
    impl hugit_checks::client::ac::ActionCache for BrokenCache {
        fn lookup(
            &self,
            _: &str,
        ) -> Result<Option<CheckResult>, hugit_checks::client::ac::AcError> {
            Err(hugit_checks::client::ac::AcError::Transport(
                "controlled local failure".into(),
            ))
        }
        fn store(&self, _: &CheckResult) -> Result<(), hugit_checks::client::ac::AcError> {
            panic!("lookup failed: no store or runner is authorized")
        }
    }

    #[test]
    fn infrastructure_failure_holds_every_member_without_false_red_or_log_mutation() {
        let mut log = EventLog::new();
        seed(&mut log, "A", &["content-a"], 0);
        seed(&mut log, "B", &["content-b"], 1);
        let before = log.clone();
        let value = batch_land(&mut log, &BrokenCache, Some("remainder"), 100).unwrap();
        assert_eq!(value["verdict"], "infrastructure_failure");
        assert_eq!(value["stop_reason"], "infrastructure_failure");
        assert_eq!(value["held"], json!(["A", "B"]));
        assert_eq!(value["landed"], json!([]));
        assert_eq!(value["excluded"], json!([]));
        assert_eq!(value["probe_count"], 1);
        assert_eq!(value["executed_count"], 0);
        assert_eq!(value["execution_count_complete"], false);
        assert_eq!(log, before);
    }

    #[test]
    fn exhausted_remainder_budget_preserves_the_entire_log_and_diagnosis() {
        let mut log = EventLog::new();
        seed(&mut log, "A", &["conflicts-with:B"], 0);
        seed(&mut log, "B", &["content-b"], 1);
        seed(&mut log, "C", &["content-c"], 2);
        let before = log.clone();
        let value = batch_land_with_limits(
            &mut log,
            &InMemoryAc::new(),
            Some("remainder"),
            100,
            EvaluationLimits {
                max_probes: 5,
                timeout: Duration::from_secs(30),
            },
        )
        .unwrap();
        assert_eq!(value["verdict"], "red");
        assert_eq!(value["stop_reason"], "probe_budget_exhausted");
        assert_eq!(value["held"], json!(["A", "B", "C"]));
        assert_eq!(value["excluded"], json!([]));
        assert_eq!(value["landed"], json!([]));
        assert_eq!(value["locus"], "pair");
        assert_eq!(value["probe_count"], 5);
        assert_eq!(log, before);
    }

    #[test]
    fn zero_deadline_never_calls_cache_or_changes_the_log() {
        let mut log = EventLog::new();
        seed(&mut log, "A", &["content-a"], 0);
        let before = log.clone();
        let value = batch_land_with_limits(
            &mut log,
            &BrokenCache,
            Some("remainder"),
            100,
            EvaluationLimits {
                max_probes: 64,
                timeout: Duration::ZERO,
            },
        )
        .unwrap();
        assert_eq!(value["stop_reason"], "deadline_exceeded");
        assert_eq!(value["probe_count"], 0);
        assert_eq!(log, before);
    }

    #[test]
    fn missing_oracle_content_is_unknown_not_an_empty_green_check() {
        let content = BTreeMap::new();
        let conflicts = std::collections::BTreeSet::new();
        let def = local_check_def();
        let mut oracle = LogMemoOracle {
            ac: &BrokenCache,
            content: &content,
            conflicts: &conflicts,
            check_def: &def,
        };
        assert_eq!(
            oracle.evaluate(&["MISSING"]),
            (UnionVerdict::Unknown, vec![])
        );
    }
}
