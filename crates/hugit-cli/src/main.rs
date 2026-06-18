//! The real `hugit` binary (WP-R-cli defect #1; converged onto the one error /
//! exit law by WP-WB0).
//!
//! A thin clap dispatch shell over the existing `hugit_cli` library verbs. The
//! binary OWNS no behavior — it parses arguments, calls the library, emits the
//! result as STABLE JSON on stdout, and maps the outcome to the process exit
//! code under the one exit-code law (`0` success · `2` structured user/domain
//! error · `1` internal fault). The wedge EXECUTE verbs (`check` / `verdict`)
//! are dispatched at W0 as honest NOT-IMPLEMENTED stubs (bodies land per
//! W-CHECK / W-VERDICT). The wired verbs (`why`, `impact`, `tournament`,
//! `export`) run end-to-end against the real library functions and emit the
//! canonical `{"error":{…}}` envelope on failure — the SAME law the flow
//! porcelain (`campaign`/`intent`/`pr`) already uses.
//!
//! Git-proximate by mandate: every verb token is drawn from
//! [`hugit_cli::HUGIT_VERBS`], the single canonical registry the namespace-law
//! invariant (WP-X5) also consumes, so the CLI surface and the invariant can
//! never drift.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use hugit_cli::campaign::{self, CampaignArgs};
use hugit_cli::checks::{self, CheckArgs, ChecksArgs};
use hugit_cli::export::{self, AccountState, Corpus};
use hugit_cli::impact::{ImpactQuery, compute_impact};
use hugit_cli::intent::{self, IntentArgs};
use hugit_cli::issue::{self, IssueArgs};
use hugit_cli::policy::{self, PolicyArgs};
use hugit_cli::porcelain::PorcelainError;
use hugit_cli::pr::{self, PrArgs};
use hugit_cli::queue::{self, QueueArgs};
use hugit_cli::repo::{self, RepoArgs};
use hugit_cli::tournament::{MAX_N_POLICY, produce_candidates};
use hugit_cli::undo::{self, UndoArgs};
use hugit_cli::verdict::{self, VerdictArgs};
use hugit_cli::why::resolver::LogEntry;
use hugit_cli::why::{WhyQuery, resolve_why};

use hugit_checks::affected::{BuildGraph, Ecosystem, PackageNode};
use hugit_contracts::{AttestationChain, EventRecord, IntentSidecar};

use serde::Deserialize;
use serde_json::json;

/// `hugit` — the git-compatible, LLM-native forge CLI.
#[derive(Parser, Debug)]
#[command(name = "hugit", version, about = "hugit — the git-native LLM forge", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The hugit verb surface. Token names MUST stay in lockstep with
/// [`hugit_cli::HUGIT_VERBS`] (asserted by the bin's own oracle).
#[derive(Subcommand, Debug)]
enum Command {
    /// Resolve a line/symbol to its originating intent + provenance.
    Why(WhyArgs),
    /// Compute the build-graph blast radius of changed paths.
    Impact(ImpactArgs),
    /// Fan an intent out into N candidates (budget-bounded).
    Tournament(TournamentArgs),
    /// Dump a git artifact + JSON envelope (anti-lock-in exit proof).
    Export(ExportArgs),
    /// Campaign lifecycle: open / close (seal) / show (WP-PC1).
    Campaign(CampaignArgs),
    /// Intent ceremony: new / show (WP-PC2).
    Intent(IntentArgs),
    /// Issue lifecycle: transition — move an issue's state (roadmap W2).
    Issue(IssueArgs),
    /// Pull-request lifecycle: open / land / show (WP-PC3).
    Pr(PrArgs),
    /// Repo authz metadata: `meta set` records `repo.meta` (visibility + owner_tenant).
    Repo(RepoArgs),
    /// Memoized-CI checks: show / key — make the CI wedge visible (WP-WB2 stub).
    Checks(ChecksArgs),
    /// Landing-queue state: show — make the union-batch wedge visible (WP-WB2 stub).
    Queue(QueueArgs),
    /// Run a memoized CI check for real (W-CHECK stub — the wedge EXECUTE path).
    Check(CheckArgs),
    /// Convene an adversarial verdict panel (W-VERDICT stub — EXECUTE path).
    Verdict(VerdictArgs),
    /// Undo an operation as a compensating event (Human-only — roadmap W3).
    Undo(UndoArgs),
    /// Declarative gate management: test — preview the house gates (roadmap W3).
    Policy(PolicyArgs),
}

// ── why ────────────────────────────────────────────────────────────────────

#[derive(clap::Args, Debug)]
struct WhyArgs {
    /// Path to a JSON file holding the event-log entries to resolve against.
    #[arg(long)]
    log: PathBuf,
    /// The file path to attribute.
    #[arg(long)]
    path: String,
    /// Optional 1-based line number within the file.
    #[arg(long)]
    line: Option<u64>,
    /// Optional symbol name to attribute.
    #[arg(long)]
    symbol: Option<String>,
}

/// The on-disk JSON shape for a single `why` log entry (the CLI's input
/// contract — mirrors the library [`LogEntry`] without re-exporting it).
#[derive(Deserialize)]
struct WhyLogEntryInput {
    record: EventRecord,
    #[serde(default)]
    attestation: Option<AttestationChain>,
    #[serde(default)]
    sidecar: Option<IntentSidecar>,
}

/// Read a `--log`/input file under the one input-error law: a missing FILE is an
/// explicit `log_not_found` (NEVER silently an empty world — the P5 finding); a
/// malformed/truncated file is a `parse_log` error. Both are exit-2 structured
/// errors. `parse` deserialises the read bytes into the verb's input type.
///
/// `shape` is the verb's OWN expected on-disk shape (P3, P-WHY-FORMAT): `why`
/// reads a `[{record, attestation?, sidecar?}, …]` array and `export` reads an
/// `{events:[…]}` object — NEITHER is the canonical `[EventRecord, …]` array the
/// flow porcelain shares. The shared error helper used to claim the canonical
/// shape for both, misleading the agent about the file format; the caller now
/// supplies the truthful shape so the `fix` hint is correct per verb.
fn read_log<T: for<'de> Deserialize<'de>>(
    path: &std::path::Path,
    shape: &str,
) -> Result<T, PorcelainError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PorcelainError::log_not_found(path));
        }
        Err(e) => return Err(PorcelainError::io("read log", path, &e)),
    };
    serde_json::from_slice(&bytes).map_err(|e| {
        PorcelainError::new(
            "parse_log",
            format!("--log file {} is not valid JSON: {e}", path.display()),
            format!("the --log file for this verb must be {shape}"),
        )
        .with_context("path", json!(path.display().to_string()))
    })
}

/// The on-disk shape `hugit why` reads — its own wrapper array, NOT the shared
/// canonical `[EventRecord, …]` log (P3 / P-WHY-FORMAT honest hint).
const WHY_LOG_SHAPE: &str = "a JSON array of why-log entries \
     [{\"record\":<EventRecord>, \"attestation\"?:<chain>, \"sidecar\"?:<IntentSidecar>}, …] \
     — note this is `why`'s own wrapper shape, NOT the bare [EventRecord, …] array \
     the flow porcelain (campaign/intent/pr) shares";

fn run_why(args: WhyArgs) -> Result<String, PorcelainError> {
    let raw: Vec<WhyLogEntryInput> = read_log(&args.log, WHY_LOG_SHAPE)?;

    // K-CHAIN: verify the hash chain of the embedded EventRecords BEFORE
    // projecting provenance. Every other read verb (campaign show, checks show,
    // queue show, pr, intent) already calls verify_chain on read; `why` was the
    // lone gap. A tampered/reordered/dropped-record log must never be projected
    // as authoritative provenance — fail closed exactly like the siblings.
    {
        let records: Vec<&EventRecord> = raw.iter().map(|e| &e.record).collect();
        let owned: Vec<hugit_contracts::EventRecord> = records.into_iter().cloned().collect();
        hugit_refstore::verify_chain(&owned).map_err(|e| {
            PorcelainError::new(
                "chain_broken",
                format!(
                    "log {} failed integrity verification: {e}",
                    args.log.display()
                ),
                "the --log file's hash chain is tampered or corrupt",
            )
        })?;
    }

    let entries: Vec<LogEntry> = raw
        .into_iter()
        .map(|r| LogEntry {
            record: r.record,
            attestation: r.attestation,
            sidecar: r.sidecar,
        })
        .collect();

    let query = WhyQuery {
        path: args.path,
        line: args.line,
        symbol: args.symbol,
    };
    let answer = resolve_why(&query, &entries).map_err(|e| {
        PorcelainError::new(
            "unresolved",
            e.to_string(),
            "query a path/line/symbol attributed by a record on the --log file; \
             why never widens or fabricates an answer",
        )
    })?;
    serde_json::to_string(&answer)
        .map_err(|e| PorcelainError::internal(format!("serialise why answer: {e}")))
}

// ── impact ───────────────────────────────────────────────────────────────────

#[derive(clap::Args, Debug)]
struct ImpactArgs {
    /// Path to a JSON file describing the build graph.
    #[arg(long)]
    graph: PathBuf,
    /// One or more changed paths (repeatable).
    #[arg(long = "path", required = true)]
    paths: Vec<String>,
}

/// The on-disk JSON shape for the build graph (the CLI input contract; the
/// library `BuildGraph` is not itself `Deserialize`, so we own this seam).
#[derive(Deserialize)]
struct GraphInput {
    ecosystem: String,
    root_manifests: Vec<String>,
    packages: Vec<PackageInput>,
}

#[derive(Deserialize)]
struct PackageInput {
    name: String,
    path: String,
    #[serde(default)]
    direct_deps: Vec<String>,
}

fn run_impact(args: ImpactArgs) -> Result<String, PorcelainError> {
    // The build graph is a JSON file under the same input-error law: a missing
    // FILE is explicit (never an empty graph silently), a malformed one is a
    // parse error. Both exit 2.
    let bytes = match std::fs::read(&args.graph) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PorcelainError::new(
                "graph_not_found",
                format!("--graph file does not exist: {}", args.graph.display()),
                "point --graph at an existing build-graph JSON file",
            )
            .with_context("path", json!(args.graph.display().to_string())));
        }
        Err(e) => return Err(PorcelainError::io("read graph", &args.graph, &e)),
    };
    let input: GraphInput = serde_json::from_slice(&bytes).map_err(|e| {
        PorcelainError::new(
            "parse_graph",
            format!(
                "--graph file {} is not valid JSON: {e}",
                args.graph.display()
            ),
            "the --graph file must be a JSON build-graph \
             {ecosystem, root_manifests, packages[]} object",
        )
        .with_context("path", json!(args.graph.display().to_string()))
    })?;

    let ecosystem = match input.ecosystem.as_str() {
        "cargo" | "Cargo" => Ecosystem::Cargo,
        "pnpm" | "Pnpm" => Ecosystem::Pnpm,
        "turbo" | "Turbo" => Ecosystem::Turbo,
        other => Ecosystem::Unknown(other.to_string()),
    };
    let graph = BuildGraph {
        ecosystem,
        root_manifests: input.root_manifests,
        packages: input
            .packages
            .into_iter()
            .map(|p| PackageNode {
                name: p.name,
                path: p.path,
                direct_deps: p.direct_deps,
            })
            .collect(),
    };

    let query = ImpactQuery {
        changed_paths: args.paths,
    };
    let result = compute_impact(&query, &graph).map_err(|e| {
        PorcelainError::new(
            "empty_graph",
            e.to_string(),
            "supply a non-empty build graph (at least one package node)",
        )
    })?;
    serde_json::to_string(&result)
        .map_err(|e| PorcelainError::internal(format!("serialise impact result: {e}")))
}

// ── tournament ────────────────────────────────────────────────────────────────

#[derive(clap::Args, Debug)]
struct TournamentArgs {
    /// Number of candidates to fan out (policy-capped).
    #[arg(short = 'n', long = "candidates")]
    n: usize,
    /// The intent id to fan out.
    #[arg(long)]
    intent: String,
    /// Optional canonical event log (`[EventRecord, …]`). When provided, the
    /// `--intent` id MUST exist on it (be landed) — a nonexistent intent is a
    /// structured `intent_not_found`/exit-2 error, never a fabricated fan-out.
    /// Omit it to keep the log-less fan-out (the fixture/smoke path).
    #[arg(long)]
    log: Option<std::path::PathBuf>,
}

fn run_tournament(args: TournamentArgs) -> Result<String, PorcelainError> {
    if args.n == 0 {
        return Err(PorcelainError::new(
            "invalid_argument",
            "tournament requires -n >= 1",
            "pass -n with a value between 1 and the policy cap",
        ));
    }
    if args.n > MAX_N_POLICY {
        return Err(PorcelainError::new(
            "policy_cap_exceeded",
            format!("tournament -n {} exceeds policy cap {MAX_N_POLICY}", args.n),
            format!("pass -n at most {MAX_N_POLICY}"),
        )
        .with_context("requested", json!(args.n))
        .with_context("cap", json!(MAX_N_POLICY)));
    }
    // Existence check (only when a --log is provided): a tournament over a
    // nonexistent intent must NOT fabricate candidates. Read the canonical log
    // through the same loader/projection the flow porcelain uses (a missing
    // file is `log_not_found`, a tampered chain `chain_broken`, an absent id
    // `intent_not_found`) — never a silent exit-0 fan-out over a ghost intent.
    if let Some(log_path) = &args.log {
        let log = checks::load_event_log(log_path)?;
        let projected = hugit_refstore::intent::intents_from_log(&log).map_err(|e| {
            PorcelainError::new(
                "bad_log",
                format!("event log does not project intents: {e}"),
                "fix the malformed intent.landed payload in the --log file",
            )
        })?;
        let exists = projected
            .intents()
            .iter()
            .any(|i| i.intent_id == args.intent);
        if !exists {
            // The intent did not resolve, so it is echoed back in the error —
            // route it through the redaction engine first so a prefixed/JWT/PEM/
            // conn-string secret smuggled as `--intent` never lands raw in the
            // error message or context (WJ-INT, Round-6 residual; mirrors the
            // WJ-VERDICT intent_not_found fix). An unresolved id is not a lookup
            // key, so the full free-text engine is safe — a real address still
            // survives for a genuine-typo diagnostic.
            let safe_intent = hugit_cli::redaction::scrub(&args.intent);
            return Err(PorcelainError::new(
                "intent_not_found",
                format!(
                    "intent '{safe_intent}' is not on the --log file (no landed intent by that id)"
                ),
                "create/land the intent first (`hugit intent new --log <path> …`) \
                 or pass an --intent id that exists on the log",
            )
            .with_context("intent", json!(safe_intent))
            .with_context("log", json!(log_path.display().to_string())));
        }
    }
    let intent = IntentSidecar {
        intent_id: args.intent,
        charter: String::new(),
        acceptance: vec![],
        context_ref: String::new(),
        authoritative: false,
    };
    // Deterministic strategy labels: strat-0..strat-(n-1).
    let labels: Vec<String> = (0..args.n).map(|i| format!("strat-{i}")).collect();
    let strategies: Vec<&str> = labels.iter().map(String::as_str).collect();
    let candidates = produce_candidates(&intent, &strategies);
    // Candidate is not a serde type; emit a stable JSON report (was plain text).
    let report = json!({
        "intent": intent.intent_id,
        "candidates": candidates.len(),
        "fanout": candidates.iter().map(|c| json!({
            "index": c.index,
            "strategy": c.strategy,
            "candidate_ref": c.candidate_ref,
            "selected": c.selected,
        })).collect::<Vec<_>>(),
    });
    Ok(report.to_string())
}

// ── export ────────────────────────────────────────────────────────────────────

#[derive(clap::Args, Debug)]
struct ExportArgs {
    /// Path to a canonical JSON event log (`[EventRecord, …]`) — the same format
    /// every porcelain verb writes through (`intent new`, `pr open`, …). The
    /// chain is verified before exporting; a tampered log is `chain_broken`/exit-2.
    #[arg(long)]
    log: PathBuf,
    /// Output directory for the artifact + envelope.
    #[arg(long)]
    out: PathBuf,
}

fn run_export(args: ExportArgs) -> Result<String, PorcelainError> {
    // K-CHAIN: verify the hash chain of the input log BEFORE exporting. The
    // previous path rebuilt an EventLog via raw `.append()` over an
    // `{events:[…]}` input that carried no hashes, so a forged/arbitrary input
    // could enter the export corpus unchecked. The fix: read the CANONICAL
    // `[EventRecord, …]` format through the same `load_event_log` every other
    // read verb uses. `load_event_log` runs `verify_chain` and returns
    // `chain_broken`/exit-2 on any tampered/reordered/corrupt input — the same
    // boundary as `checks show`, `queue show`, `tournament --log`, etc.
    //
    // Callers building an export log use the same porcelain write path every
    // other verb does (`intent new --log L`, `pr open --log L`, …), so the
    // canonical `[EventRecord, …]` format is the natural input shape here too.
    let event_log = checks::load_event_log(&args.log)?;
    let corpus = Corpus {
        event_log,
        ..Corpus::default()
    };
    let artifact = export::export(&corpus, &args.out, AccountState::Active).map_err(|e| {
        PorcelainError::new(
            "export_failed",
            e.to_string(),
            "fix the corpus/out path the error names; export fails closed, \
             never writing a partial artifact",
        )
    })?;
    // The success line is now stable JSON: the artifact paths + the redaction
    // manifest's content digest (the seal) + the bounded-memory proof.
    let report = json!({
        "exported": {
            "git_dir": artifact.git_dir.display().to_string(),
            "envelope_json": artifact.json_path.display().to_string(),
            "redaction_manifest": artifact.manifest_path.display().to_string(),
            "schema_version": artifact.envelope.schema.version,
            "peak_buffered": artifact.peak_buffered,
            "peak_serialize_scratch": artifact.peak_serialize_scratch,
        }
    });
    Ok(report.to_string())
}

// ── dispatch ──────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    // Route clap's own parse failures through the ONE error law (Round-8 C6 F-2;
    // N-6 ergo: no-args / missing subcommand is also a usage error → exit 2).
    // `Cli::parse()` would let clap print a bare English `error:` to STDERR and
    // exit before `main`'s match — bypassing the envelope choke-point, so an
    // orchestrating agent parsing STDOUT for `{"error":{"kind",…}}` gets nothing.
    // `try_parse()` returns the error here so a bad invocation emits the SAME
    // structured `{"error":{"kind":"invalid_argument",…}}` envelope on stdout +
    // exit 2 as every other user/domain error.
    //
    // NOTE (N-6): `DisplayHelpOnMissingArgumentOrSubcommand` is intentionally NOT
    // in the exit-0 branch below. `hugit` with no subcommand is a botched dispatch
    // from an orchestrating agent — the agent MUST receive the structured envelope
    // + exit 2 so it can detect the error. Only `--help`/`-h` (explicitly requested
    // help) and `--version` are real successes that stay exit 0.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            // `--help` / `--version` explicitly requested: clap renders the text
            // into the error. These are the SUCCESS path — print verbatim to stdout
            // and exit 0.  `DisplayHelpOnMissingArgumentOrSubcommand` (no-args /
            // missing subcommand) is NOT here — that's a usage error → exit 2.
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                print!("{e}");
                return ExitCode::SUCCESS;
            }
            // Every other clap error (missing subcommand, bad flag, bad value, …):
            // render the canonical envelope on STDOUT + exit 2 (the one error law).
            // `kind:"invalid_argument"` — the same stable spelling as domain
            // validation errors (singular, unified — N-6 F-1).
            let message = e
                .to_string()
                .trim_end_matches('\n')
                .replace('\n', " ")
                .trim()
                .to_string();
            let err = PorcelainError::new(
                "invalid_argument",
                message,
                "run `hugit --help` (or `hugit <verb> --help`) for the correct usage",
            );
            println!("{}", err.to_json());
            return err.exit_code();
        }
    };
    // ONE law for every verb. The flow-porcelain verbs (campaign/intent/pr) and
    // the wedge stubs (checks/queue) own their own exit code internally and
    // return an ExitCode directly. The legacy library verbs (why/impact/
    // tournament/export) now ALSO converge on the law: each returns
    // `Result<String /*stable JSON*/, PorcelainError>`, emitted here as JSON on
    // stdout — success line OR the canonical `{"error":{…}}` envelope — under
    // the one exit-code law (0 success · 2 user/domain error · 1 internal).
    let result = match cli.command {
        Command::Why(a) => run_why(a),
        Command::Impact(a) => run_impact(a),
        Command::Tournament(a) => run_tournament(a),
        Command::Export(a) => run_export(a),
        Command::Campaign(a) => return campaign::run(a),
        Command::Intent(a) => return intent::run(a),
        Command::Issue(a) => return issue::run(a),
        Command::Pr(a) => return pr::run(a),
        Command::Repo(a) => return repo::run(a),
        Command::Checks(a) => return checks::run(a),
        Command::Queue(a) => return queue::run(a),
        // Wedge EXECUTE verbs (W0 scaffold): thin → the owning module's runner,
        // which returns the honest NOT-IMPLEMENTED stub until W-CHECK/W-VERDICT
        // land the bodies. They own their own exit code (the WB0 one-exit law).
        Command::Check(a) => return checks::run_check(a),
        Command::Verdict(a) => return verdict::run(a),
        // Stakeholder verbs (W3) — REAL-wired, own their exit code (the one law).
        Command::Undo(a) => return undo::run(a),
        Command::Policy(a) => return policy::run(a),
    };
    match result {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            // The error is JSON on STDOUT (the agent parses stdout, never a fake
            // success), exit per the law.
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}
