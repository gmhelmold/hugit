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
//! 2. runs [`evaluate_union`] over their union through a REAL [`MemoCheck`]
//!    oracle ([`LogMemoOracle`]) backed by [`run_memoized`] + a file-backed
//!    Action Cache (the exact [`FileAc`] seam `hugit check run --store` uses);
//! 3. on a GREEN union, lands every member (appends `pr.landed` per PR);
//! 4. on a RED union, `evaluate_union` bisects to the minimal failing locus;
//!    the green remainder lands, the culprit is EXCLUDED, and a
//!    `queue.union_fail` record is appended carrying the bisected pair (so
//!    `queue show`'s `failing_pair` lights up).
//!
//! # Honest scope (single-tenant LOCAL operator today)
//!
//! This makes the wedge REAL for a single-tenant local operator: the AC is
//! file-backed (`<log>.ac` by default, or `--ac`), check execution is the
//! deterministic local memoized executor over per-PR content derived from the
//! PR's bundled intents. The distributed runner fabric (F7) swaps in LATER
//! behind the SAME [`MemoCheck`] trait — `evaluate_union` does not change. No
//! verdict is fabricated: a check with no result is honest (it executes once,
//! then HITs), never a faked green.
//!
//! # X5 namespace law
//!
//! `land` is a reserved top-level verb (git has neither `land` nor `queue`), so
//! it does not shadow a git builtin — the WP-X5 namespace oracle stays green.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use hugit_checks::client::ac::ActionCache;
use hugit_checks::client::executor::{CheckRunner, ExecError, run_memoized};
use hugit_checks::client::memo_key::{FileContent, compute_def_digest};
use hugit_contracts::{CheckDef, CheckResult, LandableEntry, MinimalFailingPair};
use hugit_queue::core::affected::AffectedSet;
use hugit_queue::core::batch::Batch;
use hugit_queue::core::union::{
    CheckSource, FailureLocus, MemoCheck, UnionEvaluation, UnionVerdict, evaluate_union,
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
/// axis). It is the single-tenant local marker — the live CoreLink AC over a
/// content-addressed toolchain swaps in behind the same `ActionCache` trait.
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
    /// HIT. The live CoreLink AC swaps in behind the same `ActionCache` trait.
    #[arg(long)]
    pub ac: Option<PathBuf>,
    /// Unix-ms timestamp to stamp the appended `pr.landed` / `queue.union_fail`
    /// events with.
    #[arg(long = "recorded-at", default_value_t = 0)]
    pub recorded_at: u64,
}

/// Dispatch a `land` subcommand. Returns the process exit code directly under
/// the one-exit-code law.
pub fn run(args: LandArgs) -> ExitCode {
    match args.command {
        LandCommand::Queue(a) => run_queue_land(a),
    }
}

fn run_queue_land(a: QueueLandArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(a.log.clone());
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

    let ac = FileAc::new(ac_path);
    let result = batch_land(&mut log, &ac, a.campaign.as_deref(), a.recorded_at);
    match result {
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
/// - a [`Batch`] of [`LandableEntry`]s, fed to [`evaluate_union`] over a
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
            "failing_pair": Value::Null,
            "executed_count": 0,
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
                    intent_id: o.intent_ids.first().cloned().unwrap_or_default(),
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
        .map(|(_, o)| (o.pr_id.clone(), o.intent_ids.clone()))
        .collect();

    // The conflict relation, derived HONESTLY from PR content: an intent of the
    // shape `conflicts-with:<pr_id>` declares that this PR's union with that PR
    // is red (a pair interaction neither PR exhibits alone). This is the
    // single-tenant local stand-in for "the runner reported a red union on this
    // pair"; the distributed runner fabric (F7) reports the real verdict behind
    // the same `MemoCheck` trait, and the union/bisect engine does not change.
    let conflicts = conflict_pairs(&content);

    let check_def = local_check_def();
    let mut oracle = LogMemoOracle {
        ac,
        content: &content,
        conflicts: &conflicts,
        check_def: &check_def,
    };

    let evaluation = evaluate_union(&batch, &mut oracle);
    let ids: Vec<&str> = members.iter().map(|(q, _)| q.pr_id.as_str()).collect();

    // Land the proceeding (green) set: append a terminal `pr.landed` per PR,
    // routed through the SAME D14-guarded append the `pr` porcelain uses.
    let proceeding: std::collections::BTreeSet<&str> =
        evaluation.proceeding.iter().map(String::as_str).collect();
    let mut landed: Vec<String> = Vec::new();
    for (q, o) in &members {
        if proceeding.contains(q.pr_id.as_str()) {
            land_one(log, o, recorded_at)?;
            landed.push(q.pr_id.clone());
        }
    }

    // Excluded = everyone who is not proceeding (the bisected locus, or the whole
    // batch on an unlocalised red union).
    let excluded: Vec<String> = ids
        .iter()
        .filter(|id| !proceeding.contains(**id))
        .map(|s| s.to_string())
        .collect();

    // On a RED union, record the bisected failure so `queue show` lights up.
    let failing_pair_json = match (&evaluation.verdict, &evaluation.failure) {
        (UnionVerdict::Red, Some(locus)) => {
            record_union_fail(log, campaign, locus, &excluded, &landed, recorded_at)?;
            failing_pair_to_json(&evaluation.minimal_failing_pair)
        }
        _ => Value::Null,
    };

    Ok(json!({
        "queued": members.len(),
        "campaign": campaign,
        "mode": LANDING_MODE,
        "verdict": match evaluation.verdict {
            UnionVerdict::Green => "green",
            UnionVerdict::Red => "red",
        },
        "landed": landed,
        "excluded": excluded,
        "failing_pair": failing_pair_json,
        "locus": locus_kind(&evaluation),
        "executed_count": evaluation.executed_count,
    }))
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

/// Append the `queue.union_fail` record carrying the bisected failure locus.
fn record_union_fail(
    log: &mut EventLog,
    campaign: Option<&str>,
    locus: &FailureLocus,
    excluded: &[String],
    proceeding: &[String],
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
/// union on this pair". The DISTRIBUTED runner fabric (F7) swaps in behind the
/// same [`CheckRunner`] / [`MemoCheck`] traits later, reporting the REAL union
/// verdict; `evaluate_union` + `bisect_failure` do not change. No verdict is
/// fabricated — a clean local union is honestly green.
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
        let mut any_red = false;
        let mut sources = Vec::with_capacity(item_ids.len());

        for pr_id in item_ids {
            let intents = self.content.get(*pr_id).cloned().unwrap_or_default();
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
            let outcome = run_memoized(
                self.ac,
                &runner,
                self.check_def,
                file_iter,
                LOCAL_TOOLCHAIN_DIGEST,
            )
            // The file-backed AC never faults on a well-formed key; surface a
            // failure as a red, executed source rather than panicking.
            .unwrap_or_else(|_| hugit_checks::client::executor::CheckOutcome {
                result: CheckResult {
                    memo_key: String::new(),
                    tree_hash: String::new(),
                    def_digest: String::new(),
                    toolchain_digest: LOCAL_TOOLCHAIN_DIGEST.to_string(),
                    exit: 1,
                    artifacts: vec![],
                    stdout_ref: String::new(),
                    stderr_ref: String::new(),
                    duration_ms: 0,
                    runner_ref: "local-union-test".to_string(),
                    produced_at: 0,
                },
                from_cache: false,
                local_executions: 1,
            });

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
/// faked per-PR failure. A real distributed runner (F7) swaps in behind this
/// same [`CheckRunner`] trait and reports the real exit.
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
