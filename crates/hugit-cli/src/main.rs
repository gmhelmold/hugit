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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use hugit_cli::campaign::{self, CampaignArgs};
use hugit_cli::capture::{self, CaptureArgs};
use hugit_cli::checks::{self, CheckArgs};
use hugit_cli::ctx::{self, CtxArgs};
use hugit_cli::diag::{self, DiagArgs};
use hugit_cli::dock::{self, DockArgs};
use hugit_cli::export::{self, AccountState, Corpus};
use hugit_cli::fleet::{self, FleetArgs};
use hugit_cli::health::{self, HealthArgs};
use hugit_cli::impact::{ImpactQuery, compute_impact};
use hugit_cli::init::{self, AttachArgs, InitArgs};
use hugit_cli::intent::{self, IntentArgs};
use hugit_cli::issue::{self, IssueArgs};
use hugit_cli::land::{self, LandArgs};
use hugit_cli::ledger::{self, LedgerArgs};
use hugit_cli::meta::{self, MetaArgs};
use hugit_cli::note::{self, NoteArgs};
use hugit_cli::policy::{self, PolicyArgs};
use hugit_cli::porcelain::PorcelainError;
use hugit_cli::pr::{self, PrArgs};
use hugit_cli::queue::{self, QueueArgs};
use hugit_cli::review::{self, ReviewArgs};
use hugit_cli::setup::{self};
use hugit_cli::symbol::{self, SymbolArgs};
use hugit_cli::tournament::{MAX_N_POLICY, produce_candidates};
use hugit_cli::undo::{self, UndoArgs};
use hugit_cli::verdict::{self, VerdictArgs};
use hugit_cli::watch::{self, WatchArgs};
use hugit_cli::why::resolver::LogEntry;
use hugit_cli::why::{
    PreciseLineResolution, WhyQuery, resolve_precise_line, resolve_why, resolve_why_chain,
};

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
    /// One-time install: configure git's global init.templateDir so every future
    /// `git init` auto-ships the hugit hooks (the boot ceremony, no per-repo init).
    Setup(setup::SetupArgs),
    /// Attach hugit hooks to an existing Git repository.
    Attach(AttachArgs),
    /// Remove only hugit-owned hooks from an existing Git repository.
    Detach(InitArgs),
    /// Inspect local hook and event-log health without mutating repository state.
    Health(HealthArgs),
    /// Campaign lifecycle: open / close (seal) / show.
    Campaign(CampaignArgs),
    /// Internal hook-only capture. Normal workflows use Git, never this command.
    Capture(CaptureArgs),
    /// Intent ceremony: new / show / list.
    Intent(IntentArgs),
    /// Issue lifecycle: transition — move an issue's state.
    Issue(IssueArgs),
    /// Pull-request lifecycle: open / queue / land / show / list / abandon.
    Pr(PrArgs),
    /// Batch land: run the real union-test + bisect + memoize engine over the queue.
    Land(LandArgs),
    /// Repo metadata: `meta set` records visibility + owning tenant.
    Meta(MetaArgs),
    /// Landing-queue state: show the union-batch queue.
    Queue(QueueArgs),
    /// Memoized CI checks: run a check, show the hit-rate, or predict its memo key.
    Check(CheckArgs),
    /// Adversarial verdict: convene a review panel, or record an approve/reject.
    Verdict(VerdictArgs),
    /// Undo an operation as a compensating event (human-only).
    Undo(UndoArgs),
    /// Declarative gate management: preview the house gates against a context.
    Policy(PolicyArgs),
    /// Append a session note onto the canonical log.
    Note(NoteArgs),
    /// Bisect a red check history into a structured diagnosis (read-only).
    Diag(DiagArgs),
    /// The default forge history view: asked → done → proven per campaign.
    Ledger(LedgerArgs),
    /// Machine-readable fleet state: workspaces + agents, versioned schema.
    Fleet(FleetArgs),
    /// Replay the classified, redacted forge event stream.
    Watch(WatchArgs),
    /// Outline a local source file's symbols (semantic index).
    Symbol(SymbolArgs),
    /// Short-horizon session resume: `ctx resume` reconstructs from session notes.
    Ctx(CtxArgs),
    /// Grounded-evidence Q&A over the log: cite real check/verdict evidence or refuse.
    Review(ReviewArgs),
    /// Worktree-dock (ADR-0005): coin the physical binding at checkout time.
    Dock(DockArgs),
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
    /// Repository holding the selected committed tree. Required for exact line
    /// and symbol modes; ignored by legacy path-only mode.
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Commit whose tree is queried in exact line and symbol modes.
    #[arg(long, default_value = "HEAD")]
    commit: String,
    /// Walk the FULL provenance chain (every captured event that touched the
    /// path, most-recent first) instead of only the origin. The chain is the
    /// "why did this file evolve" answer the single origin cannot give.
    #[arg(long)]
    walk: bool,
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

const MAX_SYMBOL_LINES: u32 = 10_000;

struct CachedSymbol {
    name: String,
    start_line: u32,
    end_line: u32,
}

fn run_why(args: WhyArgs) -> Result<String, PorcelainError> {
    // `hugit why` accepts BOTH the canonical log (a bare `[EventRecord, ...]`
    // — what the silent hooks + `hugit capture` write) AND the legacy wrapper
    // shape (`[{record, attestation?, sidecar?}, ...]`). Detect by item shape:
    // an item carrying a top-level `kind` is a bare EventRecord; an item with a
    // nested `record` is the wrapper. Fail-closed on anything else.
    let bytes = std::fs::read(&args.log).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PorcelainError::log_not_found(&args.log)
        } else {
            PorcelainError::io("read log", &args.log, &e)
        }
    })?;
    let raw: Vec<WhyLogEntryInput> = {
        let resolved: Vec<serde_json::Value> = serde_json::from_slice(&bytes).map_err(|e| {
            PorcelainError::new(
                "parse_log",
                format!("--log file {} is not valid JSON: {e}", args.log.display()),
                "the --log file must be a canonical [EventRecord, ...] or the why wrapper \
                 [{record, ...}, ...]",
            )
        })?;
        resolved
            .into_iter()
            .map(|item| {
                if item.get("kind").is_some() {
                    // Bare EventRecord: wrap it with no attestation/sidecar.
                    serde_json::from_value::<WhyLogEntryInput>(serde_json::json!({
                        "record": item,
                    }))
                } else {
                    serde_json::from_value::<WhyLogEntryInput>(item)
                }
            })
            .collect::<Result<_, _>>()
            .map_err(|e| {
                PorcelainError::new(
                    "parse_log",
                    format!("--log items are neither EventRecord nor why-wrapper: {e}"),
                    "the --log file must be [EventRecord, ...] or [{record, ...}, ...]",
                )
            })?
    };

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

    let repo = args.repo;
    let commit_arg = args.commit;
    let query = WhyQuery {
        path: args.path,
        line: args.line,
        symbol: args.symbol,
    };
    let projection = hugit_cli::projection::views::load(&args.log).map_err(|error| {
        PorcelainError::new(
            "projection_invalid",
            error,
            "repair projection status or re-run capture; why never invents derived state",
        )
    })?;
    if let Some(line) = query.line {
        let repo = repo.as_deref().ok_or_else(|| {
            PorcelainError::new(
                "invalid_argument",
                "why --line requires --repo so provenance never reads implicit worktree bytes",
                "pass --repo <git working tree> and optionally --commit <oid>",
            )
        })?;
        let commit = git_commit(repo, &commit_arg)?;
        git_blob_exists(repo, &commit, &query.path)?;
        let blamed = git_blame_line(repo, &commit, &query.path, line)?;
        let resolution =
            resolve_precise_line(&query.path, line, &blamed, &entries).map_err(|error| {
                PorcelainError::new(
                    "bad_log",
                    error.to_string(),
                    "repair malformed event payloads before querying provenance",
                )
            })?;
        return serde_json::to_string(&precise_line_json(&query.path, line, resolution)).map_err(
            |error| PorcelainError::internal(format!("serialise precise why answer: {error}")),
        );
    }
    if let Some(symbol) = query.symbol.as_deref() {
        let repo = repo.as_deref().ok_or_else(|| {
            PorcelainError::new(
                "invalid_argument",
                "why --symbol requires --repo",
                "pass --repo <git working tree> and optionally --commit <oid>",
            )
        })?;
        let commit = git_commit(repo, &commit_arg)?;
        let matches = cached_symbols(repo, &commit, &query.path)?
            .into_iter()
            .filter(|item| item.name == symbol)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(PorcelainError::new(
                "symbol_not_found",
                format!("symbol `{symbol}` does not exist in `{}`", query.path),
                "pass an exact symbol name from supported committed source",
            ));
        }
        if matches.len() > 1 {
            return serde_json::to_string(&json!({ "observed": { "commit": commit, "path": query.path }, "status": "ambiguous_symbol" })).map_err(|error| PorcelainError::internal(format!("serialise symbol answer: {error}")));
        }
        let symbol = matches.into_iter().next().expect("nonempty after check");
        if symbol.end_line - symbol.start_line >= MAX_SYMBOL_LINES {
            return Err(PorcelainError::new(
                "symbol_too_large",
                format!(
                    "symbol `{}` exceeds {MAX_SYMBOL_LINES} attributable lines",
                    symbol.name
                ),
                "query a smaller symbol or add a bounded range selection",
            ));
        }
        let mut contributors = BTreeMap::new();
        for (line, blamed) in git_blame_range(
            repo,
            &commit,
            &query.path,
            symbol.start_line as u64,
            symbol.end_line as u64,
        )? {
            let resolution =
                resolve_precise_line(&query.path, line, &blamed, &entries).map_err(|error| {
                    PorcelainError::new(
                        "bad_log",
                        error.to_string(),
                        "repair malformed event payloads before querying provenance",
                    )
                })?;
            contributors
                .entry(blamed)
                .or_insert_with(|| precise_line_json(&query.path, line, resolution));
        }
        let contributors = contributors.into_values().collect::<Vec<_>>();
        let status = contributors
            .iter()
            .filter_map(|contributor| contributor.get("status").and_then(|status| status.as_str()))
            .find(|status| *status != "attributed")
            .unwrap_or("attributed");
        return serde_json::to_string(&json!({ "observed": { "commit": commit, "path": query.path, "range": [symbol.start_line, symbol.end_line] }, "contributors": contributors, "status": status })).map_err(|error| PorcelainError::internal(format!("serialise symbol answer: {error}")));
    }
    // The walk (--walk) projects the FULL chain, most recent first; the origin
    // answer is the head of that chain for the "single attribution" read.
    if args.walk {
        let _ = projection;
        return serde_json::to_string(&resolve_why_chain(&query, &entries))
            .map_err(|e| PorcelainError::internal(format!("serialise why chain: {e}")));
    }
    let answer = resolve_why(&query, &entries).map_err(|e| {
        PorcelainError::new(
            "unresolved",
            e.to_string(),
            "query a path/line/symbol attributed by a record on the --log file; \
             why never widens or fabricates an answer",
        )
    })?;
    let mut answer = serde_json::to_value(answer)
        .map_err(|e| PorcelainError::internal(format!("serialise why answer: {e}")))?;
    answer["projection"] = serde_json::to_value(projection)
        .map_err(|e| PorcelainError::internal(format!("serialise projection view: {e}")))?;
    serde_json::to_string(&answer)
        .map_err(|e| PorcelainError::internal(format!("serialise why answer: {e}")))
}

fn git_commit(repo: &std::path::Path, commit: &str) -> Result<String, PorcelainError> {
    if commit.starts_with('-') {
        return Err(PorcelainError::new(
            "invalid_argument",
            "--commit must not start with '-'",
            "pass a commit oid or ref name",
        ));
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", &format!("{commit}^{{commit}}")])
        .output()
        .map_err(|error| PorcelainError::io("resolve commit", repo, &error))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "commit_not_found",
            format!("commit `{commit}` is not available in {}", repo.display()),
            "pass a commit reachable in --repo",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_blob_exists(repo: &std::path::Path, commit: &str, path: &str) -> Result<(), PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "-e", &format!("{commit}:{path}")])
        .output()
        .map_err(|error| PorcelainError::io("read committed blob", repo, &error))?;
    if output.status.success() {
        return Ok(());
    }
    Err(PorcelainError::new(
        "path_not_found",
        format!("path `{path}` does not exist at commit `{commit}`"),
        "pass a path present in selected committed tree",
    ))
}

fn git_blame_line(
    repo: &std::path::Path,
    commit: &str,
    path: &str,
    line: u64,
) -> Result<String, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "blame",
            "--line-porcelain",
            "-L",
            &format!("{line},{line}"),
            commit,
            "--",
            path,
        ])
        .output()
        .map_err(|error| PorcelainError::io("blame committed line", repo, &error))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "line_not_found",
            format!("line {line} of `{path}` does not exist at commit `{commit}`"),
            "pass a 1-based line within selected committed blob",
        ));
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .filter(|oid| !oid.is_empty())
        .map(String::from)
        .ok_or_else(|| {
            PorcelainError::new(
                "blame_failed",
                "git blame returned no commit oid",
                "verify repository object integrity",
            )
        })
}

fn git_blame_range(
    repo: &std::path::Path,
    commit: &str,
    path: &str,
    start: u64,
    end: u64,
) -> Result<Vec<(u64, String)>, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "blame",
            "--line-porcelain",
            "-L",
            &format!("{start},{end}"),
            commit,
            "--",
            path,
        ])
        .output()
        .map_err(|error| PorcelainError::io("blame committed symbol", repo, &error))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "line_not_found",
            format!("symbol range {start}..{end} of `{path}` does not exist at commit `{commit}`"),
            "query a symbol in selected committed tree",
        ));
    }
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if !(fields.len() == 3 || fields.len() == 4)
                || fields[0].len() != 40
                || !fields[0].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return None;
            }
            let line = fields[2].parse::<u64>().ok()?;
            Some((line, fields[0].to_string()))
        })
        .collect::<Vec<_>>();
    if lines.len() == (end - start + 1) as usize {
        return Ok(lines);
    }
    Err(PorcelainError::new(
        "blame_failed",
        "git blame returned an incomplete symbol range",
        "verify repository object integrity",
    ))
}

fn cached_symbols(
    repo: &std::path::Path,
    commit: &str,
    path: &str,
) -> Result<Vec<CachedSymbol>, PorcelainError> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let lang = hugit_symbols::lang_for_ext(ext).ok_or_else(|| {
        PorcelainError::new(
            "unsupported_language",
            format!("path `{path}` has no supported source language"),
            "query a supported source extension",
        )
    })?;
    let spec = format!("{commit}:{path}");
    let oid = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", &spec])
        .output()
        .map_err(|error| PorcelainError::io("resolve committed blob", repo, &error))?;
    if !oid.status.success() {
        return Err(PorcelainError::new(
            "path_not_found",
            format!("path `{path}` does not exist at commit `{commit}`"),
            "pass a path present in selected committed tree",
        ));
    }
    let oid = String::from_utf8_lossy(&oid.stdout).trim().to_string();
    // `.hugit` may come from an untrusted checkout. Do not persist derived
    // symbol data there: a cache is never worth a provenance write primitive.
    let blob = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "blob", &oid])
        .output()
        .map_err(|error| PorcelainError::io("read committed blob", repo, &error))?;
    if !blob.status.success() {
        return Err(PorcelainError::new(
            "blob_not_found",
            format!("blob `{oid}` is unavailable"),
            "verify repository object integrity",
        ));
    }
    let symbols = hugit_symbols::outline_blob_ranges(lang, &blob.stdout)
        .into_iter()
        .map(|symbol| CachedSymbol {
            name: symbol.name,
            start_line: symbol.start_line,
            end_line: symbol.end_line,
        })
        .collect::<Vec<_>>();
    Ok(symbols)
}

fn precise_line_json(
    path: &str,
    line: u64,
    resolution: PreciseLineResolution,
) -> serde_json::Value {
    match resolution {
        PreciseLineResolution::Attributed { commit, answer } => json!({
            "observed": { "commit": commit, "path": path, "range": [line, line], "event_hash": answer.event_hash },
            "declared": answer.intent_id.map(|intent| json!({ "intent": intent, "charter": answer.charter })),
            "attested": if answer.model.is_empty() && answer.cost.is_empty() { None } else { Some(json!({ "model": answer.model, "cost": answer.cost })) },
            "status": "attributed",
        }),
        PreciseLineResolution::Unattributed { commit } => {
            json!({ "observed": { "commit": commit, "path": path, "range": [line, line] }, "status": "unattributed" })
        }
        PreciseLineResolution::RangeUnavailable { commit } => {
            json!({ "observed": { "commit": commit, "path": path, "range": [line, line] }, "status": "range_unavailable" })
        }
        PreciseLineResolution::RangeMismatch { commit } => {
            json!({ "observed": { "commit": commit, "path": path, "range": [line, line] }, "status": "range_mismatch" })
        }
    }
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
    let artifact = export::export(&corpus, &args.out, AccountState::Active).map_err(|e| match e {
        export::ExportError::SensitiveCanonicalEventPayload { .. } => PorcelainError::new(
            "sensitive_canonical_event",
            e.to_string(),
            "scrub sensitive payloads before appending canonical events; export cannot redact them without breaking the hash chain",
        ),
        e => PorcelainError::new(
            "export_failed",
            e.to_string(),
            "fix the corpus/out path the error names; export fails closed, \
             never writing a partial artifact",
        ),
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
    // NOTE (N-6 + TTY ergo): `hugit` with no subcommand is context-sensitive.
    // For a HUMAN at an interactive terminal it should be friendly — print the
    // help and exit 0, like `git` with no args. For a MACHINE (piped/redirected
    // stdout — an orchestrating agent's botched dispatch) the structured
    // `{"error":{"kind":"invalid_argument",…}}` envelope + exit 2 is preserved so
    // the agent can detect the error. We branch on `stdout().is_terminal()`.
    // `--help`/`-h` and `--version` are always real successes (exit 0).
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            use std::io::IsTerminal;
            // `--help` / `--version` explicitly requested: clap renders the text
            // into the error. These are the SUCCESS path — print verbatim to stdout
            // and exit 0.
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                print!("{e}");
                return ExitCode::SUCCESS;
            }
            // No subcommand at an interactive TTY: a human just typed `hugit`.
            // Print the human help (clap renders it into the error string) and
            // exit 0 — never spit a JSON error at a person. Only when stdout is a
            // TTY: a piped/redirected stream is a machine and keeps the envelope.
            if matches!(
                e.kind(),
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) && std::io::stdout().is_terminal()
            {
                print!("{e}");
                return ExitCode::SUCCESS;
            }
            // Every other clap error (missing subcommand when piped, bad flag, bad
            // value, …): render the canonical envelope on STDOUT + exit 2 (the one
            // error law). `kind:"invalid_argument"` — the same stable spelling as
            // domain validation errors (singular, unified — N-6 F-1).
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
        Command::Setup(a) => return setup::run(a),
        Command::Attach(a) => return init::attach_run(a),
        Command::Detach(a) => return init::detach_run(a),
        Command::Health(a) => return health::run(a),
        Command::Campaign(a) => return campaign::run(a),
        Command::Capture(a) => return capture::run(a),
        Command::Intent(a) => return intent::run(a),
        Command::Issue(a) => return issue::run(a),
        Command::Pr(a) => return pr::run(a),
        Command::Land(a) => return land::run(a),
        Command::Meta(a) => return meta::run(a),
        Command::Queue(a) => return queue::run(a),
        // `check` and `verdict` own their own exit code (the WB0 one-exit law)
        // and dispatch their subcommands internally (check run|show|key,
        // verdict record|approve|reject).
        Command::Check(a) => return checks::run(a),
        Command::Verdict(a) => return verdict::run(a),
        // Stakeholder verbs (W3) — REAL-wired, own their exit code (the one law).
        Command::Undo(a) => return undo::run(a),
        Command::Policy(a) => return policy::run(a),
        Command::Note(a) => return note::run(a),
        Command::Diag(a) => return diag::run(a),
        Command::Ledger(a) => return ledger::run(a),
        Command::Fleet(a) => return fleet::run(a),
        Command::Watch(a) => return watch::run(a),
        Command::Symbol(a) => return symbol::run(a),
        Command::Ctx(a) => return ctx::run(a),
        Command::Review(a) => return review::run(a),
        Command::Dock(a) => return dock::run(a),
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
