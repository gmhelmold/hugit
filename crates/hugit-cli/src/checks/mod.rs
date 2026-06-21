//! `hugit checks` — the memoized-CI wedge made visible (WP-WB2).
//!
//! The audit's Tier-2 P1 finding: the wedge — memoized-CI hit-rate, memo keys,
//! union-batch state — is unreachable by the agent. WB0 landed the VERB into the
//! registry as an honest stub; WB2 fills the projection over real engine seams.
//!
//! # The two reads, and the honest-null law
//!
//! - **`checks show --log <path>`** projects per-check rows + the wedge KPIs
//!   (hit-rate %, hits/executed counts, saved_ms) from `check.*` records on the
//!   canonical event log. **There is no porcelain seam that records a
//!   `CheckResult` onto the log yet** (the local executor `run_memoized` returns
//!   a `CheckOutcome` in-process; no verb appends it). So `checks show` projects
//!   from whatever `check.recorded` events DO exist, and where a field was never
//!   captured it is `null`/absent — **never invented**. On a log with zero check
//!   records the KPIs are honest nulls and `checks: []`, with a `note` disclosing
//!   the gap. This is the wedge made visible the moment any seam records checks,
//!   with zero fakery before then.
//!
//! - **`checks key`** computes the REAL three-axis memo key
//!   (`H(tree ‖ def ‖ toolchain)`) through the engine's own primitive
//!   ([`hugit_refstore::compute_memo_key`]) — so an agent can predict cache
//!   behavior before running anything. No log needed; pure computation.
//!
//! Every output is stable JSON on stdout under the WB0 one-error/one-exit law
//! ([`crate::porcelain`]): `log_not_found` / `parse_log` are the canonical
//! `{"error":{…}}` envelopes, exit `2`.

mod run;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use hugit_refstore::EventLog;
use serde_json::{Value, json};

use crate::porcelain::PorcelainError;

/// Event kind a captured `CheckResult` is recorded under, when a seam records
/// one (the additive, forward-compatible read target). Payload (canonical JSON):
/// the [`hugit_contracts::CheckResult`] fields, optionally extended with the
/// provenance the executor carries: `name`, `cache_hit` (bool), `pr_id`.
///
/// **Producer (frozen at W0): `hugit check --store` — the W-CHECK EXECUTE
/// path** (`check run`). Until W-CHECK lands the recorder body, no porcelain
/// verb appends this kind — the local executor (`hugit_checks::run_memoized`)
/// returns its `CheckOutcome` in-process. This read is therefore honest-empty on
/// today's logs and live the instant `check --store` records onto the log.
/// Naming it here (additive over the D1 log, sibling to `pr.opened`/`pr.queued`)
/// is the forward contract the recorder targets.
pub const CHECK_RECORDED_KIND: &str = "check.recorded";

/// `hugit check <subcommand>` — the memoized-CI verb: `run` (execute), `show`
/// (project hit-rate), `key` (predict the memo key).
///
/// Git-proximate cleanup: the old split between a top-level `check` (execute) and
/// a separate plural `checks show|key` (read) was two verbs for one concept. They
/// are now ONE verb with three subcommands — `run` is the EXECUTE path, `show`
/// and `key` are the reads. (`hugit check` with no subcommand is a usage error,
/// like `git remote` with no subcommand.)
#[derive(clap::Args, Debug)]
pub struct CheckArgs {
    #[command(subcommand)]
    pub command: CheckCommand,
}

/// The check subcommand surface — `run` (execute) / `show` / `key`.
#[derive(Subcommand, Debug)]
pub enum CheckCommand {
    /// Run a memoized CI check for real; with --store, record it onto the log.
    Run(CheckRunArgs),
    /// Show the memoized-CI hit-rate + per-check rows for a log/target.
    Show(ShowArgs),
    /// Compute the content memo key a check resolves to (predict cache behavior).
    Key(KeyArgs),
}

/// `hugit checks show` flags.
#[derive(clap::Args, Debug)]
pub struct ShowArgs {
    /// Path to the canonical JSON event log. Defaults to $HUGIT_LOG, else
    /// .hugit/log.json.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// Optional PR id: scope the projection to checks recorded for one PR.
    #[arg(long)]
    pub pr: Option<String>,
}

/// `hugit checks key` flags — the three memo axes, computed through the engine.
#[derive(clap::Args, Debug)]
pub struct KeyArgs {
    /// Workspace tree-root hash (axis 1 — the scoped Merkle root over the
    /// check's input subtree; lowercase hex).
    #[arg(long)]
    pub tree: String,
    /// Check-definition digest (axis 2 — `H(command ‖ inputs ‖ toolchain_ref ‖
    /// env ‖ glob_set)`; lowercase hex).
    #[arg(long)]
    pub def: String,
    /// Toolchain digest (axis 3 — content-addressed toolchain; lowercase hex).
    #[arg(long)]
    pub toolchain: String,
}

/// Dispatch a `check` subcommand, emitting stable JSON on stdout and returning
/// the process exit code under the WB0 one-exit-code law.
pub fn run(args: CheckArgs) -> ExitCode {
    match args.command {
        CheckCommand::Run(a) => emit(run::run(&a)),
        CheckCommand::Show(a) => emit(show(&a)),
        CheckCommand::Key(a) => emit(Ok(key(&a))),
    }
}

/// `hugit check run` flags — the wedge EXECUTE path (W-CHECK).
///
/// This is the `check run` subcommand (distinct from the `check show`/`check key`
/// READ subcommands above). It runs a single memoized check for real: resolve the
/// three-axis memo key, look up the AC, execute on a miss, and — with `--store`
/// — record a [`CHECK_RECORDED_KIND`] event onto the canonical `--log` so the
/// `check show` read can project it.
///
/// W0 froze the seam `--def --log [--store]`; W-INT ports the W-CHECK executor
/// body in behind it and adds the *additive* run-shaping flags (`--cmd`,
/// `--root`, `--toolchain`, `--pr`, `--principal`, `--ac`) — all optional, so
/// the frozen `--def --log [--store]` contract is unchanged.
#[derive(clap::Args, Debug)]
pub struct CheckRunArgs {
    /// Check-definition name to run. A built-in (`fmt` / `clippy` / `test`)
    /// resolves to its frozen gate command; any other name is an ad-hoc check
    /// that REQUIRES `--cmd`.
    #[arg(long)]
    pub def: String,
    /// Path to the canonical JSON event log. Defaults to $HUGIT_LOG, else
    /// .hugit/log.json. The run's memo lookup/result is read from / (with
    /// `--store`) recorded to it.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// Record the [`hugit_contracts::CheckResult`] onto the log as a
    /// `check.recorded` event (the recorder seam `checks show` projects from).
    /// Omit to run without persisting (a dry memoized check).
    #[arg(long)]
    pub store: bool,
    /// The shell command for an ad-hoc `--def <name>` that is not a built-in.
    /// Ignored (the built-in command wins) for a built-in name.
    #[arg(long)]
    pub cmd: Option<String>,
    /// Workspace root the check's input subtree is scoped from (default: cwd).
    /// The memo `tree_hash` axis is the Merkle root over the files under this
    /// root matching the def's `glob_set` — so an edit inside the glob is a MISS.
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// The toolchain digest (third memo axis). Defaults to a fixed local marker
    /// so the wedge is deterministic without a content-addressed toolchain.
    #[arg(long)]
    pub toolchain: Option<String>,
    /// Optional PR id stamped onto the `check.recorded` row (so `checks show
    /// --pr` can scope to it).
    #[arg(long)]
    pub pr: Option<String>,
    /// The principal recording the check (default `orchestrator:hugit` — fleet/CI
    /// provenance). Routed through the append authorization guard.
    #[arg(long)]
    pub principal: Option<String>,
    /// The local file-backed Action Cache path. Defaults to `<log>.ac` so a warm
    /// re-run (a separate process) is a HIT over the same wedge state. The live
    /// CoreLink AC swaps in behind the same `ActionCache` seam at P2.
    #[arg(long)]
    pub ac: Option<PathBuf>,
    /// Bound the check execution to this many seconds (default 300). A command
    /// that runs past the deadline is killed and the run is a structured
    /// `check_timeout` error (exit 2) — so a hang (`sleep infinity`) can never
    /// block forever holding the `--log` lock.
    #[arg(long)]
    pub timeout_secs: Option<u64>,
    /// Declare a custom environment variable this check legitimately depends on
    /// (repeatable). The hermetic spawn CLEARS every ambient var not on the
    /// built-in result-affecting allowlist (RUSTFLAGS, CARGO_*, …), so an ad-hoc
    /// `--cmd` check that reads a CUSTOM var (e.g. `MY_GATE_MODE`) would otherwise
    /// see it UNSET. Naming it with `--env-axis MY_GATE_MODE` (a) folds it into the
    /// memo key (a change to its value is a MISS) AND (b) passes it through to the
    /// spawn — so the dependency is sound (declared == keyed == present), never a
    /// silent stale green. Unset declared vars contribute nothing (toggling the var
    /// on later is itself a MISS).
    #[arg(long = "env-axis")]
    pub env_axis: Vec<String>,
}

impl CheckRunArgs {
    /// The resolved `--log` path: the explicit flag, else `$HUGIT_LOG`, else the
    /// conventional `.hugit/log.json` (the ONE shared resolver).
    pub fn log_path(&self) -> PathBuf {
        crate::log_resolve::resolve_log(self.log.clone())
    }
}

/// Emit a `Result<Value, PorcelainError>` as stable JSON on stdout under the one
/// error/exit law: the success value, or the canonical `{"error":{…}}` envelope.
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

/// `hugit checks key` — compute the real three-axis memo key through the engine.
///
/// Pure: `memo_key = H(tree ‖ def ‖ toolchain)` via the ONE canonical primitive
/// [`hugit_refstore::compute_memo_key`]. We never re-transcribe the formula; the
/// output is byte-identical to what `run_memoized` keys an AC lookup on for the
/// same three axes. An agent calls this to predict a hit/miss before running.
fn key(args: &KeyArgs) -> Value {
    let memo_key = hugit_refstore::compute_memo_key(&args.tree, &args.def, &args.toolchain);
    json!({
        "memo_key": memo_key,
        "axes": {
            "tree_hash": args.tree,
            "def_digest": args.def,
            "toolchain_digest": args.toolchain,
        },
        "formula": "H(tree_hash || def_digest || toolchain_digest)",
    })
}

/// One projected check row from a `check.recorded` event on the log.
///
/// Every field is read from the recorded payload (the `CheckResult` fields plus
/// the executor's provenance). Fields the recorder did not capture are honestly
/// absent in the output — never defaulted to a fake value.
struct CheckRow {
    /// Human/agent check name (provenance — absent on the bare `CheckResult`).
    name: Option<String>,
    /// `true` if this row's check passed (`exit == 0`), `None` if exit absent.
    ok: Option<bool>,
    /// Wall-clock duration in ms (`CheckResult::duration_ms`).
    duration_ms: Option<u64>,
    /// `true` if the result was served from cache (zero local execution).
    cache_hit: Option<bool>,
    /// The full memo key (64-char lowercase hex).
    memo_key: Option<String>,
    /// PR id this check was recorded for, when the recorder scoped it.
    pr_id: Option<String>,
    /// Memo axis 1 — the workspace Merkle tree-root hash the check was keyed on
    /// (`CheckResult::tree_hash`). REAL on a `check --store` row.
    tree_hash: Option<String>,
    /// Memo axis 2 — the `CheckDef` body digest (`CheckResult::def_digest`).
    def_digest: Option<String>,
    /// Memo axis 3 — the content-addressed toolchain digest
    /// (`CheckResult::toolchain_digest`). REAL on a `check --store` row.
    toolchain_digest: Option<String>,
    /// Content-addressed ref to the cached `runner` that produced the result
    /// (`CheckResult::runner_ref`). On a HIT this is the cached proof's runner;
    /// **not yet on the `check.recorded` payload** → honest `None`/`n/a`.
    runner_ref: Option<String>,
    /// Content-addressed ref to the cached stdout blob (`CheckResult::stdout_ref`).
    /// **Not yet recorded** by the porcelain seam → honest `None`/`n/a`.
    stdout_ref: Option<String>,
    /// Content-addressed ref to the cached stderr blob (`CheckResult::stderr_ref`).
    /// **Not yet recorded** by the porcelain seam → honest `None`/`n/a`.
    stderr_ref: Option<String>,
}

impl CheckRow {
    /// Project a row from a `check.recorded` payload — honest reads, no defaults.
    fn from_payload(v: &Value) -> Self {
        let exit = v.get("exit").and_then(Value::as_i64);
        CheckRow {
            name: str_field(v, "name"),
            ok: exit.map(|e| e == 0),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64),
            // `cache_hit` is the executor's provenance bool; absent ⇒ unknown,
            // never assumed false (a false "miss" would mis-state the wedge).
            cache_hit: v.get("cache_hit").and_then(Value::as_bool),
            memo_key: str_field(v, "memo_key"),
            pr_id: str_field(v, "pr_id"),
            // The three memo axes ARE on the `check --store` payload (run.rs builds
            // them from the CheckResult), so an agent can see WHY a key resolved
            // the way it did. A pre-axis row (e.g. a hand-seeded fixture) that omits
            // them surfaces honest `null`, never a fabricated digest.
            tree_hash: str_field(v, "tree_hash"),
            def_digest: str_field(v, "def_digest"),
            toolchain_digest: str_field(v, "toolchain_digest"),
            // The cached PROOF refs (runner/stdout/stderr) are CheckResult fields
            // that the current `check --store` payload does NOT yet carry (run.rs
            // records the identity axes + cache_hit + duration, not the blob refs).
            // We read them honestly so the field lights up the instant the recorder
            // adds it — until then it is `null`/`n/a`, never invented.
            runner_ref: str_field(v, "runner_ref").filter(|s| !s.is_empty()),
            stdout_ref: str_field(v, "stdout_ref").filter(|s| !s.is_empty()),
            stderr_ref: str_field(v, "stderr_ref").filter(|s| !s.is_empty()),
        }
    }

    /// The scannable HIT/MISS verdict for this row, derived from `cache_hit`:
    /// `"HIT"` (served from cache, zero local execution — the wedge firing),
    /// `"MISS"` (executed for real), or `null` when `cache_hit` was never recorded
    /// (honest unknown, never assumed a miss).
    fn cache_verdict(&self) -> Value {
        match self.cache_hit {
            Some(true) => json!("HIT"),
            Some(false) => json!("MISS"),
            None => Value::Null,
        }
    }

    /// The three-axis memo decomposition (`tree ‖ def ‖ toolchain`) the key was
    /// computed from — so an agent sees WHY this check hit or missed. Each axis is
    /// honest `null` if the row never recorded it (a pre-axis fixture).
    fn axes_json(&self) -> Value {
        json!({
            "tree_hash": self.tree_hash,
            "def_digest": self.def_digest,
            "toolchain_digest": self.toolchain_digest,
        })
    }

    /// The cached PROOF on a HIT — the content-addressed refs to the memoized
    /// runner / stdout / stderr. On a MISS this is `null` (no proof was served —
    /// the check just ran). On a HIT, each ref is the cached value when recorded,
    /// else honest `null`/`n/a` (the `check --store` payload does not yet carry the
    /// blob refs — a tracked recorder follow-up). `recorded:false` discloses that
    /// the proof-ref fields are not on today's payload, so an agent never mistakes
    /// an absent ref for "no proof exists".
    fn proof_json(&self) -> Value {
        // A proof is only meaningful on a served HIT; a MISS executed for real.
        if self.cache_hit != Some(true) {
            return Value::Null;
        }
        let recorded =
            self.runner_ref.is_some() || self.stdout_ref.is_some() || self.stderr_ref.is_some();
        json!({
            "runner_ref": self.runner_ref,
            "stdout_ref": self.stdout_ref,
            "stderr_ref": self.stderr_ref,
            // false ⇒ the cached-proof refs are not yet on the `check.recorded`
            // payload (a tracked recorder follow-up); the HIT itself is real.
            "recorded": recorded,
        })
    }

    /// Render the row as stable JSON. `memo_key_short` is a 12-char prefix for
    /// scannability; `memo_key` is the full key. `cache_verdict` is the
    /// HIT/MISS label; `axes` is the three-axis memo decomposition; `proof` is the
    /// cached proof ref on a HIT. Honestly-absent fields render as JSON `null` so
    /// the shape is stable for an agent parsing rows.
    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "ok": self.ok,
            "duration_ms": self.duration_ms,
            "cache_hit": self.cache_hit,
            "cache_verdict": self.cache_verdict(),
            "memo_key": self.memo_key,
            "memo_key_short": self.memo_key.as_deref().map(truncate_key),
            "axes": self.axes_json(),
            "proof": self.proof_json(),
            "pr_id": self.pr_id,
        })
    }
}

/// `hugit checks show` — project per-check rows + the wedge KPIs from the log.
///
/// Reads the canonical event log through the engine's [`EventLog`], folds every
/// `check.recorded` event (optionally scoped to `--pr`), and aggregates the
/// REAL hit-rate KPIs over the rows whose `cache_hit` was actually captured.
/// When no checks were ever recorded, the rows are `[]` and the KPIs are honest
/// nulls with a disclosing `note` — never a fabricated hit-rate.
fn show(args: &ShowArgs) -> Result<Value, PorcelainError> {
    let log_path = crate::log_resolve::resolve_log(args.log.clone());
    let log = load_event_log(&log_path)?;

    let rows: Vec<CheckRow> = log
        .records()
        .iter()
        .filter(|r| r.kind == CHECK_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter(|v| match &args.pr {
            // Scope to a PR only when asked; a row missing pr_id is excluded
            // under a --pr filter (it was not attributed to that PR).
            Some(pr) => v.get("pr_id").and_then(Value::as_str) == Some(pr.as_str()),
            None => true,
        })
        .map(|v| CheckRow::from_payload(&v))
        .collect();

    let kpis = aggregate_kpis(&rows);
    let row_json: Vec<Value> = rows.iter().map(CheckRow::to_json).collect();

    // Whether ANY hit row carried a cached proof ref — disclosed at the top level
    // so an agent knows, without scanning every row, that the proof-ref fields are
    // honest-`null` on today's payload (a tracked recorder follow-up) rather than
    // "no proof exists". `true` the instant the recorder starts capturing them.
    let proof_refs_recorded = rows.iter().any(|r| {
        r.cache_hit == Some(true)
            && (r.runner_ref.is_some() || r.stdout_ref.is_some() || r.stderr_ref.is_some())
    });

    let mut out = json!({
        "log": log_path.display().to_string(),
        "pr": args.pr,
        "check_count": rows.len(),
        "checks": row_json,
        "kpis": kpis,
        // The memoization story, made legible: each row carries the three memo
        // axes (tree ‖ def ‖ toolchain), a HIT/MISS `cache_verdict`, and — on a HIT
        // — the cached `proof` ref block. `proof_refs_recorded` discloses whether
        // those refs are on today's payload (currently the recorder captures the
        // identity axes + cache verdict but not the blob refs — honest n/a).
        "proof_refs_recorded": proof_refs_recorded,
    });
    if rows.is_empty()
        && let Some(obj) = out.as_object_mut()
    {
        // Honest disclosure: no seam records CheckResults onto the porcelain log
        // yet, so a clean log legitimately has zero check rows. Say so — never
        // imply checks ran with a 0% hit-rate. (`out` is a freshly-built `json!`
        // object so `as_object_mut` is always `Some`; the `if let` keeps that a
        // SAFE no-op rather than a latent `unwrap` panic — advisory #5.)
        obj.insert(
            "note".to_string(),
            json!(
                "no check.recorded events on this log; the local executor \
                 (hugit_checks::run_memoized) returns its CheckOutcome in-process \
                 and no porcelain verb appends it yet — KPIs are null, not zero"
            ),
        );
    }
    Ok(out)
}

/// The memoized-CI wedge KPIs, aggregated over the rows whose `cache_hit` was
/// actually captured. Each KPI is `null` (not zero) when the underlying data
/// was never recorded — the honest-null law: the agent learns "unknown", never
/// a fabricated figure.
///
/// - `hits` / `executed` — counts over rows with a known `cache_hit`.
/// - `hit_rate_pct` — `hits / (hits+executed) * 100`, `null` if no row had a
///   known `cache_hit`.
/// - `saved_ms` — summed `duration_ms` of the rows that were cache HITS (the
///   wall-clock the wedge avoided re-spending); `null` when no hit row carried a
///   duration.
fn aggregate_kpis(rows: &[CheckRow]) -> Value {
    let mut hits: u64 = 0;
    let mut executed: u64 = 0;
    let mut saved_ms: u64 = 0;
    let mut any_cache_hit_known = false;
    let mut any_saved_known = false;

    for row in rows {
        match row.cache_hit {
            Some(true) => {
                any_cache_hit_known = true;
                hits += 1;
                if let Some(ms) = row.duration_ms {
                    any_saved_known = true;
                    saved_ms += ms;
                }
            }
            Some(false) => {
                any_cache_hit_known = true;
                executed += 1;
            }
            // Unknown cache_hit contributes to neither count — honest.
            None => {}
        }
    }

    let denom = hits + executed;
    let hit_rate_pct = if any_cache_hit_known && denom > 0 {
        // Two-decimal percentage; integer arithmetic to avoid float drift in the
        // wire shape (× 10000 then / denom gives basis points → /100.0 → pct).
        let bps = (hits * 10_000) / denom;
        json!((bps as f64) / 100.0)
    } else {
        Value::Null
    };

    json!({
        "hit_rate_pct": hit_rate_pct,
        "hits": if any_cache_hit_known { json!(hits) } else { Value::Null },
        "executed": if any_cache_hit_known { json!(executed) } else { Value::Null },
        "saved_ms": if any_saved_known { json!(saved_ms) } else { Value::Null },
    })
}

/// PS-13 — the single read-path chokepoint fault. Distinguishes the two ways a
/// deserialised set of `[EventRecord, …]` can fail to become an authoritative
/// [`EventLog`]: a non-monotonic/gappy `seq` (`Rehydrate`) and a tampered/corrupt
/// hash chain (`ChainBroken`). Each disk loader maps this into its OWN error
/// type (`crate::porcelain::PorcelainError`, `intent::error::PorcelainError`,
/// `campaign::output::CampaignError`, or a `pr` `ExitCode`) — but NONE of them
/// re-implements the `EventLog::new() + push_record + verify_chain` sequence,
/// which lives in exactly ONE place: [`rehydrate_and_verify`].
#[derive(Debug)]
pub enum ChainLoadFault {
    /// A record's `seq` broke the append-only invariant on rehydrate.
    Rehydrate(String),
    /// The hash chain failed integrity verification (tampered/reordered).
    ChainBroken(String),
}

/// PS-13 — THE single verified-loader chokepoint. Turn an already-deserialised
/// `[EventRecord, …]` vector into a hash-chain-VERIFIED [`EventLog`].
///
/// This is the ONE production function in `hugit-cli` that runs the
/// `EventLog::new() + push_record + verify_chain` rehydrate-and-verify sequence
/// for a log read from disk. Every read verb (`checks`/`queue`/`tournament` via
/// [`load_event_log`]; `pr`, `campaign`, `intent new`, `intent list`, `export`,
/// `why`) routes its disk load through this function, so a NEW read verb cannot
/// project an unverified log without calling it — the recurring "a new read verb
/// forgot `verify_chain`" root (why/export R7, intent list R8) becomes
/// structurally unreachable. The `no_unverified_log_projection` source invariant
/// (test) holds the line: `verify_chain` for a disk load appears in this file
/// only.
///
/// A tampered chain is [`ChainLoadFault::ChainBroken`]; a non-monotonic `seq` is
/// [`ChainLoadFault::Rehydrate`]. The caller maps the fault into its own error
/// taxonomy (all surface exit-2).
pub(crate) fn rehydrate_and_verify(
    records: Vec<hugit_contracts::event_record::EventRecord>,
) -> Result<EventLog, ChainLoadFault> {
    let mut log = EventLog::new();
    for record in records {
        log.push_record(record)
            .map_err(|e| ChainLoadFault::Rehydrate(e.to_string()))?;
    }
    // Fail closed on a tampered / corrupt chain — the SINGLE verify gate every
    // disk read-path passes through.
    hugit_refstore::verify_chain(log.records())
        .map_err(|e| ChainLoadFault::ChainBroken(e.to_string()))?;
    Ok(log)
}

/// Read `path` (a JSON `[EventRecord, …]` array) into an [`EventLog`], rehydrating
/// **and verifying** the hash chain via the [`rehydrate_and_verify`] chokepoint.
/// Every fault is the canonical [`PorcelainError`] (exit `2`): `log_not_found`
/// (absent file — NEVER silently an empty world), `parse_log` (malformed JSON),
/// `io` (other read fault), `internal` (a record whose seq breaks the
/// append-only invariant), and `chain_broken` (the hash chain is tampered/corrupt).
///
/// WF-3: this loader (shared by `checks show` AND `queue show` — the latter
/// calls it through [`crate::queue`]; and `tournament --log`'s existence check)
/// previously SKIPPED the chain verification its siblings
/// (`pr`/`campaign`/`intent`) all run, so a tampered log projected as truth on
/// those two reads. It now fails closed exactly like the siblings: a tampered
/// chain is `chain_broken`/exit-2, never read.
pub fn load_event_log(path: &Path) -> Result<EventLog, PorcelainError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PorcelainError::log_not_found(path));
        }
        Err(e) => return Err(PorcelainError::io("read log", path, &e)),
    };
    load_event_log_from_bytes(&bytes, path)
}

/// Verify + rehydrate an event log from RAW BYTES — the SAME chokepoint as
/// [`load_event_log`] minus the filesystem read, so a non-file source (e.g. an
/// R2 object the `/v1` server fetches) is held to the identical integrity bar:
/// it routes through [`rehydrate_and_verify`], and a tampered chain fails CLOSED
/// (`chain_broken`). `source` is used only for error context (e.g. the R2 key).
pub fn load_event_log_from_bytes(bytes: &[u8], source: &Path) -> Result<EventLog, PorcelainError> {
    let records: Vec<hugit_contracts::event_record::EventRecord> =
        serde_json::from_slice(bytes).map_err(|e| PorcelainError::parse_log(source, &e))?;
    rehydrate_and_verify(records).map_err(|fault| match fault {
        ChainLoadFault::Rehydrate(e) => {
            PorcelainError::internal(format!("rehydrate log {}: {e}", source.display()))
        }
        ChainLoadFault::ChainBroken(e) => PorcelainError::new(
            "chain_broken",
            format!(
                "log {} failed integrity verification: {e}",
                source.display()
            ),
            "the --log file's hash chain is tampered or corrupt",
        ),
    })
}

/// A short (12-char) prefix of a memo key for scannable display. The full key is
/// always carried alongside; this is purely an at-a-glance affordance.
fn truncate_key(key: &str) -> String {
    key.chars().take(12).collect()
}

/// Read an optional string field from a JSON object, `None` if absent/non-string.
fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matches_the_engine_primitive() {
        let args = KeyArgs {
            tree: "aa".to_string(),
            def: "bb".to_string(),
            toolchain: "cc".to_string(),
        };
        let v = key(&args);
        // Parity: the printed key IS the engine's own computation, never a
        // re-transcription.
        let expected = hugit_refstore::compute_memo_key("aa", "bb", "cc");
        assert_eq!(v["memo_key"], expected);
        assert_eq!(v["axes"]["tree_hash"], "aa");
        assert_eq!(v["axes"]["def_digest"], "bb");
        assert_eq!(v["axes"]["toolchain_digest"], "cc");
    }

    #[test]
    fn empty_rows_give_null_kpis_never_zero() {
        let kpis = aggregate_kpis(&[]);
        assert!(kpis["hit_rate_pct"].is_null());
        assert!(kpis["hits"].is_null());
        assert!(kpis["executed"].is_null());
        assert!(kpis["saved_ms"].is_null());
    }

    #[test]
    fn hit_rate_aggregates_only_known_cache_hits() {
        let rows = vec![
            CheckRow::from_payload(&json!({"cache_hit": true, "duration_ms": 100})),
            CheckRow::from_payload(&json!({"cache_hit": true, "duration_ms": 50})),
            CheckRow::from_payload(&json!({"cache_hit": false, "duration_ms": 200})),
            // cache_hit unknown — must not skew the rate.
            CheckRow::from_payload(&json!({"duration_ms": 999})),
        ];
        let kpis = aggregate_kpis(&rows);
        // 2 hits, 1 executed → 2/3 = 66.66%
        assert_eq!(kpis["hits"], 2);
        assert_eq!(kpis["executed"], 1);
        assert_eq!(kpis["hit_rate_pct"], json!(66.66));
        // saved_ms is the summed duration of the HIT rows only (100+50).
        assert_eq!(kpis["saved_ms"], 150);
    }

    #[test]
    fn row_renders_ok_from_exit_and_honest_nulls() {
        let hit = CheckRow::from_payload(&json!({
            "name": "fmt",
            "exit": 0,
            "duration_ms": 42,
            "cache_hit": true,
            "memo_key": "0123456789abcdef0123",
            "pr_id": "PR-1",
        }))
        .to_json();
        assert_eq!(hit["name"], "fmt");
        assert_eq!(hit["ok"], true);
        assert_eq!(hit["cache_hit"], true);
        assert_eq!(hit["memo_key_short"], "0123456789ab");
        assert_eq!(hit["memo_key"], "0123456789abcdef0123");

        // A bare payload: every uncaptured field is null, ok unknown.
        let bare = CheckRow::from_payload(&json!({})).to_json();
        assert!(bare["name"].is_null());
        assert!(bare["ok"].is_null());
        assert!(bare["cache_hit"].is_null());
        assert!(bare["memo_key_short"].is_null());
    }

    #[test]
    fn row_surfaces_three_memo_axes_from_a_check_store_payload() {
        // A real `check --store` payload (the shape run.rs builds) carries the
        // three memo axes — the row must surface them verbatim, so an agent sees
        // WHY a key resolved the way it did.
        let row = CheckRow::from_payload(&json!({
            "name": "clippy",
            "exit": 0,
            "cache_hit": true,
            "memo_key": "aaaabbbbccccdddd",
            "tree_hash": "treehash00",
            "def_digest": "defdigest00",
            "toolchain_digest": "tcdigest00",
        }))
        .to_json();
        assert_eq!(row["axes"]["tree_hash"], "treehash00");
        assert_eq!(row["axes"]["def_digest"], "defdigest00");
        assert_eq!(row["axes"]["toolchain_digest"], "tcdigest00");
        // A payload that never recorded the axes (a pre-axis fixture) surfaces
        // honest null axes, never a fabricated digest.
        let bare = CheckRow::from_payload(&json!({"cache_hit": false})).to_json();
        assert!(bare["axes"]["tree_hash"].is_null());
        assert!(bare["axes"]["def_digest"].is_null());
        assert!(bare["axes"]["toolchain_digest"].is_null());
    }

    #[test]
    fn cache_verdict_is_hit_miss_or_honest_null() {
        let hit = CheckRow::from_payload(&json!({"cache_hit": true})).to_json();
        assert_eq!(hit["cache_verdict"], "HIT");
        let miss = CheckRow::from_payload(&json!({"cache_hit": false})).to_json();
        assert_eq!(miss["cache_verdict"], "MISS");
        // cache_hit unknown ⇒ verdict null (never assumed a MISS).
        let unknown = CheckRow::from_payload(&json!({})).to_json();
        assert!(unknown["cache_verdict"].is_null());
    }

    #[test]
    fn proof_is_null_on_a_miss_and_honest_na_on_a_hit_without_refs() {
        // A MISS executed for real — there is no cached proof to serve.
        let miss = CheckRow::from_payload(&json!({"cache_hit": false})).to_json();
        assert!(miss["proof"].is_null());

        // A HIT today carries NO blob refs (the recorder does not yet capture
        // them), so the proof block is present with honest-null refs and discloses
        // `recorded: false` — never an invented ref, never "no proof".
        let hit = CheckRow::from_payload(&json!({"cache_hit": true})).to_json();
        assert!(hit["proof"]["runner_ref"].is_null());
        assert!(hit["proof"]["stdout_ref"].is_null());
        assert!(hit["proof"]["stderr_ref"].is_null());
        assert_eq!(hit["proof"]["recorded"], false);

        // The instant a future recorder DOES capture a runner_ref, the field
        // lights up and `recorded` flips to true with zero code change here.
        let hit_with_ref = CheckRow::from_payload(&json!({
            "cache_hit": true,
            "runner_ref": "cas:runner:abc",
        }))
        .to_json();
        assert_eq!(hit_with_ref["proof"]["runner_ref"], "cas:runner:abc");
        assert_eq!(hit_with_ref["proof"]["recorded"], true);
        // An empty-string ref is treated as honest-absent (run.rs records "" for
        // the not-yet-captured CheckResult refs), not a real proof.
        let hit_empty = CheckRow::from_payload(&json!({
            "cache_hit": true,
            "runner_ref": "",
        }))
        .to_json();
        assert!(hit_empty["proof"]["runner_ref"].is_null());
        assert_eq!(hit_empty["proof"]["recorded"], false);
    }

    #[test]
    fn show_seeded_log_projects_axes_verdict_and_discloses_proof_gap() {
        // Seed a real chain-verified log carrying two `check.recorded` rows (a HIT
        // and a MISS) and assert `checks show` projects the memoization story. The
        // log is built through the `test-support` raw-append shim (the canonical
        // synthetic-log fixture path) so the chain hashes verify on read.
        let mut log = EventLog::new();
        for payload in [
            json!({
                "name": "fmt", "exit": 0, "cache_hit": false, "duration_ms": 120,
                "memo_key": "key_miss_0000", "tree_hash": "th0", "def_digest": "dd0",
                "toolchain_digest": "tc0",
            }),
            json!({
                "name": "fmt", "exit": 0, "cache_hit": true, "duration_ms": 120,
                "memo_key": "key_hit_1111", "tree_hash": "th1", "def_digest": "dd1",
                "toolchain_digest": "tc1",
            }),
        ] {
            log.append_for_test(
                CHECK_RECORDED_KIND,
                vec!["orchestrator:hugit".to_string()],
                payload.to_string(),
                0,
            );
        }
        let dir = std::env::temp_dir().join(format!("hugit-checks-show-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("checks-seed.json");
        std::fs::write(&log_path, serde_json::to_vec_pretty(log.records()).unwrap()).unwrap();

        let out = show(&ShowArgs {
            log: Some(log_path),
            pr: None,
        })
        .unwrap();

        assert_eq!(out["check_count"], 2);
        // Hit-rate over the two known cache_hit rows: 1 hit / (1+1) = 50%.
        assert_eq!(out["kpis"]["hit_rate_pct"], json!(50.0));
        // The three axes are surfaced on each row.
        let checks = out["checks"].as_array().unwrap();
        assert_eq!(checks[0]["cache_verdict"], "MISS");
        assert_eq!(checks[0]["axes"]["tree_hash"], "th0");
        assert_eq!(checks[1]["cache_verdict"], "HIT");
        assert_eq!(checks[1]["axes"]["toolchain_digest"], "tc1");
        // The HIT row carries a proof block with honest-null refs (not recorded).
        assert_eq!(checks[1]["proof"]["recorded"], false);
        // Top-level disclosure: no proof refs were recorded on this log.
        assert_eq!(out["proof_refs_recorded"], false);
    }
}
