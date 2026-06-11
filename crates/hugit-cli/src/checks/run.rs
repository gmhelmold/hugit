//! `hugit check` — EXECUTE the memoized-CI wedge for real (WP-W-INT, ported
//! from W-CHECK's executor onto W0's frozen `check` entry point).
//!
//! This is the verb that makes the wedge END-TO-END: resolve a named (or ad-hoc)
//! [`CheckDef`], compute the three-axis memo key, consult the Action Cache
//! through the engine's own [`run_memoized`](hugit_checks::client::executor::run_memoized),
//! and — with `--store` — **append a `check.recorded` event** to the canonical
//! `--log` carrying `{memo_key, cache_hit, duration_ms, exit}`. `hugit checks
//! show` then projects a REAL, non-null hit-rate over those rows — the same
//! projection that was an honest-null negative-control before any seam recorded
//! checks.
//!
//! # Honest cold/warm semantics (the wedge, proven)
//!
//! - A **cold** run (the key is absent from the AC) EXECUTES the def's command
//!   once, captures a real [`CheckResult`] (real exit, real wall-clock
//!   `duration_ms`), stores it in the AC, and records `cache_hit:false`.
//! - A **warm** re-run with byte-identical inputs (same tree subtree, same def,
//!   same toolchain) is a HIT: ZERO local execution, `cache_hit:true`, the
//!   verb's local `duration_ms` is `0` (no wall-clock re-spent), and the
//!   memoized cost surfaces as `saved_ms` (the wall-clock the wedge avoided).
//!
//! The zero-execution-on-hit guarantee is structural — it is enforced inside
//! `run_memoized`, which returns BEFORE the runner is ever touched on a hit.
//!
//! # AC selection (local default, live-swappable)
//!
//! The cache backend is chosen behind the [`ActionCache`] seam — never
//! hardcoded. The default is a **file-backed local AC** ([`FileAc`]) so the wedge
//! works WITHOUT P2 AND a warm re-run in a SEPARATE process is still a hit (an
//! in-memory cache would lose its entries between binary invocations). The live
//! [`HttpAcClient`](hugit_checks::client::ac::HttpAcClient) over CoreLink swaps
//! in behind the same trait when the P2 tenant + PAT exist — the surrounding
//! `run_memoized` logic does not change. [`select_ac`] is the one place the
//! backend is chosen.
//!
//! # The canonical-log seam (D14-guarded, atomic, lock-serialized)
//!
//! The `check.recorded` append (only when `--store` is set) rides the SAME seam
//! every porcelain verb shares: the advisory exclusive [`FileLock`] over the
//! `--log` path, an [`append_authorized`](hugit_refstore::EventLog::append_authorized)
//! under `Endpoint::Push` (the universal verb — every principal class may push;
//! a check recording is fleet/CI provenance), and an atomic temp-file-then-rename
//! [`atomic_write`](crate::pr::filelock::atomic_write). A read-modify-write race
//! surfaces the structured `log_busy`, never a clobber.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hugit_checks::client::ac::ActionCache;
use hugit_checks::client::executor::{self, CheckRunner, ExecError};
use hugit_checks::client::memo_key::FileContent;
use hugit_contracts::{CheckDef, CheckResult};
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::{Value, json};

use super::{CHECK_RECORDED_KIND, CheckArgs, load_event_log};
use crate::porcelain::PorcelainError;
use crate::pr::filelock::{self, FileLock, LockError};

/// The fixed local toolchain marker used when `--toolchain` is not supplied — a
/// deterministic stand-in for a content-addressed toolchain digest so the wedge
/// is reproducible without P2's toolchain CAS. A real digest swaps in unchanged.
const LOCAL_TOOLCHAIN_DIGEST: &str = "local-toolchain";

/// Resolve a built-in check def by name into its frozen gate command. The
/// built-ins are the gate set every hugit/CoreLink crate runs.
///
/// Returns `None` for an unknown name — the caller then requires `--cmd` (the
/// generic ad-hoc form the `CheckDef` contract naturally supports via its
/// `command` field).
fn builtin_command(name: &str) -> Option<&'static str> {
    match name {
        "fmt" => Some("cargo fmt --all --check"),
        "clippy" => Some("cargo clippy --workspace --all-targets --locked -- -D warnings"),
        "test" => Some("cargo test --workspace --locked"),
        _ => None,
    }
}

/// The default input glob for the built-in defs — Rust sources + the manifests
/// that change the gate's outcome. Scopes the `tree_hash` axis so an edit inside
/// the source tree is a MISS and an edit outside it is a HIT.
fn builtin_glob_set() -> Vec<String> {
    vec![
        "**/*.rs".to_string(),
        "**/Cargo.toml".to_string(),
        "Cargo.lock".to_string(),
    ]
}

/// Build the [`CheckDef`] for this run from `--def` (+ optional `--cmd`).
///
/// A built-in name uses its frozen gate command + the Rust glob set; an unknown
/// name is the ad-hoc form and REQUIRES `--cmd` (fail-closed: a def with no
/// command can never execute, so we refuse rather than memoize an empty action).
/// The `def_digest` is canonicalized by the parser so the memo key is honest.
fn resolve_def(args: &CheckArgs) -> Result<CheckDef, PorcelainError> {
    let (command, glob_set) = match builtin_command(&args.def) {
        Some(cmd) => (cmd.to_string(), builtin_glob_set()),
        None => {
            let cmd = args
                .cmd
                .clone()
                .filter(|c| !c.trim().is_empty())
                .ok_or_else(|| {
                    PorcelainError::new(
                        "unknown_def",
                        format!(
                            "`{}` is not a built-in check def (fmt|clippy|test) and no \
                             --cmd was given",
                            args.def
                        ),
                        "pass a built-in --def (fmt|clippy|test) or supply --cmd \"<shell>\" \
                         for an ad-hoc check",
                    )
                    .with_context("def", json!(args.def))
                })?;
            // An ad-hoc check scopes its tree axis to everything by default; the
            // command itself decides what it reads. Keeping the glob broad means
            // any workspace edit is a MISS — honest (never a stale false hit).
            (cmd, vec!["**/*".to_string()])
        }
    };

    let def = CheckDef {
        def_digest: String::new(),
        command,
        inputs: Vec::new(),
        toolchain_ref: args
            .toolchain
            .clone()
            .unwrap_or_else(|| LOCAL_TOOLCHAIN_DIGEST.to_string()),
        env_manifest: String::new(),
        glob_set,
    };
    // Normalize the def_digest canonically through the parser/validator so a
    // self-reported digest can never smuggle a different body past the memo key.
    hugit_checks::client::parser::validate(def).map_err(|e| {
        PorcelainError::new(
            "invalid_def",
            format!("the resolved check def is invalid: {e}"),
            "this is an internal def-construction fault; report it",
        )
    })
}

/// Snapshot the workspace files under `root` whose path matches `glob_set` into
/// the `(rel_path, content)` map [`run_memoized`] hashes for the tree axis.
///
/// The paths are `/`-separated relative to `root` so the digest is stable across
/// absolute-prefix changes. A missing/inaccessible file is skipped (it cannot
/// contribute content); the digest is over what is present + matched, exactly as
/// the engine's `scoped_tree_root` consumes it.
fn snapshot_tree(root: &Path, glob_set: &[String]) -> BTreeMap<String, FileContent> {
    let mut files = BTreeMap::new();
    collect_files(root, root, glob_set, &mut files);
    files
}

/// Recursively walk `dir`, collecting glob-matched files relative to `base`.
/// Skips `target/`, `.git/`, and the worktree scratch dir so the tree axis is the
/// SOURCE subtree, not build output (which would make every run a miss).
fn collect_files(
    base: &Path,
    dir: &Path,
    glob_set: &[String],
    out: &mut BTreeMap<String, FileContent>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Prune build/VCS/scratch dirs — they are never check INPUTS.
            if name == "target" || name == ".git" || name == ".claude" {
                continue;
            }
            collect_files(base, &path, glob_set, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if hugit_checks::client::glob::matches_any(glob_set, &rel)
                && let Ok(bytes) = std::fs::read(&path)
            {
                out.insert(rel, bytes);
            }
        }
    }
}

/// The real, process-spawning [`CheckRunner`]: executes the def's `command` via
/// the platform shell ONCE on a cache MISS, capturing the real exit code and
/// wall-clock `duration_ms` into a fully-formed [`CheckResult`] with the three
/// memo axes + the precomputed key stamped (so the stored record self-keys).
///
/// Output is captured (not streamed) so a check's stdout/stderr never pollutes
/// the verb's stable-JSON stdout contract; the digests of the captured streams
/// are not content-addressed here (that is the runner-side B2b concern) — the
/// refs are left empty, honestly absent rather than faked.
struct ProcessRunner;

impl CheckRunner for ProcessRunner {
    fn run(
        &self,
        def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, ExecError> {
        let start = Instant::now();
        let output = shell_command(&def.command)
            .output()
            .map_err(|e| ExecError::Run(format!("spawn `{}`: {e}", def.command)))?;
        let duration_ms = start.elapsed().as_millis() as u64;
        let exit = output.status.code().unwrap_or(-1);
        Ok(CheckResult {
            memo_key: memo_key.to_string(),
            tree_hash: tree_root.to_string(),
            def_digest: def_digest.to_string(),
            toolchain_digest: toolchain_digest.to_string(),
            exit,
            artifacts: Vec::new(),
            stdout_ref: String::new(),
            stderr_ref: String::new(),
            duration_ms,
            runner_ref: "local".to_string(),
            produced_at: now_unix_ms(),
        })
    }
}

/// Build the shell invocation for a check command (`sh -c <cmd>` on Unix,
/// `cmd /C <cmd>` on Windows). A single shell string is the `CheckDef::command`
/// contract, so the shell is the faithful interpreter.
fn shell_command(cmd: &str) -> Command {
    #[cfg(windows)]
    {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    }
}

/// Unix epoch milliseconds (best-effort; `0` if the clock is before the epoch).
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The Action-Cache backend chosen for this run, behind the [`ActionCache`] seam.
///
/// The local default ([`FileAc`]) makes the wedge work WITHOUT P2 and survives
/// across processes. The live `HttpAcClient` is the same trait — a `Live` arm is
/// the documented swap-in point once P2 is provisioned; until then it is not
/// constructed here (the verb never silently makes a network call).
enum AcBackend {
    /// File-backed local AC (default) — persists across binary invocations so a
    /// warm re-run is a real cross-process HIT.
    Local(FileAc),
}

impl ActionCache for AcBackend {
    fn lookup(&self, key: &str) -> Result<Option<CheckResult>, hugit_checks::client::ac::AcError> {
        match self {
            AcBackend::Local(ac) => ac.lookup(key),
        }
    }
    fn store(&self, result: &CheckResult) -> Result<(), hugit_checks::client::ac::AcError> {
        match self {
            AcBackend::Local(ac) => ac.store(result),
        }
    }
}

/// Choose the AC backend (the ONE place the backend is selected — never
/// hardcoded at the call site). Defaults to the file-backed local AC at `--ac`
/// (or `<log>.ac`). The live CoreLink `HttpAcClient::from_runtime` swaps in
/// behind the same [`ActionCache`] trait at P2 — `run_memoized` does not change.
fn select_ac(args: &CheckArgs) -> AcBackend {
    let store = args
        .ac
        .clone()
        .unwrap_or_else(|| with_extension(&args.log, "ac"));
    AcBackend::Local(FileAc::new(store))
}

/// Append `.<ext>` to a path's existing file name (so `log.json` → `log.json.ac`,
/// never clobbering an unrelated `log.ac`).
fn with_extension(path: &Path, ext: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "log".to_string());
    name.push('.');
    name.push_str(ext);
    path.with_file_name(name)
}

/// A file-backed local Action Cache implementing the reference [`ActionCache`]
/// semantics (content-keyed, store-then-hit) over a single JSON file holding a
/// `{ memo_key: CheckResult }` map. It is the local default so the wedge survives
/// across processes WITHOUT P2; the live HTTP client is the P2 swap behind the
/// same trait.
///
/// Each op re-reads/re-writes the whole map under the porcelain [`FileLock`] +
/// [`atomic_write`](crate::pr::filelock::atomic_write) discipline — fine at the
/// wedge's scale (a handful of checks), and it keeps the on-disk state
/// crash-consistent.
struct FileAc {
    path: PathBuf,
}

impl FileAc {
    fn new(path: PathBuf) -> Self {
        FileAc { path }
    }

    /// Read the `{memo_key: CheckResult}` map from disk; an absent/empty/corrupt
    /// file is an empty cache (a fresh cache is legitimately absent — this is the
    /// local-state seam, not the canonical event log whose absence is an error).
    fn read_map(&self) -> BTreeMap<String, CheckResult> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        }
    }
}

impl ActionCache for FileAc {
    fn lookup(&self, key: &str) -> Result<Option<CheckResult>, hugit_checks::client::ac::AcError> {
        Ok(self.read_map().get(key).cloned())
    }

    fn store(&self, result: &CheckResult) -> Result<(), hugit_checks::client::ac::AcError> {
        use hugit_checks::client::ac::AcError;
        // Lock the store path across the read-modify-write so two checks racing
        // the same store file serialize rather than clobber.
        let _lock = FileLock::acquire(&self.path)
            .map_err(|e| AcError::Transport(format!("AC store lock: {e}")))?;
        let mut map = self.read_map();
        map.insert(result.memo_key.clone(), result.clone());
        let bytes = serde_json::to_vec_pretty(&map)
            .map_err(|e| AcError::Decode(format!("AC store serialize: {e}")))?;
        filelock::atomic_write(&self.path, &bytes)
            .map_err(|e| AcError::Transport(format!("AC store write: {e}")))?;
        Ok(())
    }
}

/// `hugit check` — resolve → memoize → execute-on-miss → (with `--store`) record.
///
/// The full wedge in one verb: derive the def + tree snapshot, run it through
/// [`run_memoized`] (HIT ⇒ 0 execution / `duration_ms:0`; MISS ⇒ execute once +
/// store), then — when `--store` is set — append a `check.recorded` event to the
/// canonical log through the guarded, lock-serialized, atomic seam. Returns the
/// stable-JSON outcome (the recorded row + the cache verdict) for the agent.
pub fn run(args: &CheckArgs) -> Result<Value, PorcelainError> {
    let def = resolve_def(args)?;
    let toolchain_digest = args
        .toolchain
        .clone()
        .unwrap_or_else(|| LOCAL_TOOLCHAIN_DIGEST.to_string());
    let root = args
        .root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let files = snapshot_tree(&root, &def.glob_set);
    let ac = select_ac(args);
    let runner = ProcessRunner;

    // The wedge: derive the three-axis key, look it up, execute-on-miss + store.
    let outcome = executor::run_memoized(
        &ac,
        &runner,
        &def,
        files.iter().map(|(p, c)| (p.as_str(), c)),
        &toolchain_digest,
    )
    .map_err(map_exec_error)?;

    let result = &outcome.result;
    let ok = result.exit == 0;

    // `result.duration_ms` is the work the def COST when it last actually ran:
    // freshly measured on a MISS, or the memoized figure on a HIT. The wedge
    // splits this into two honest numbers:
    //   - `duration_ms` (this invocation's LOCAL wall-clock) = `0` on a HIT (zero
    //     local execution — the wedge), the measured time on a MISS.
    //   - `saved_ms` (the wall-clock the wedge AVOIDED) = the memoized duration on
    //     a HIT, `0` on a MISS (nothing was saved — we just paid it).
    // `checks show` sums the HIT rows' recorded `duration_ms` as `saved_ms`, so
    // the row carries the memoized cost (the avoided time), not a zero.
    let local_duration_ms = if outcome.from_cache {
        0
    } else {
        result.duration_ms
    };
    let saved_ms = if outcome.from_cache {
        result.duration_ms
    } else {
        0
    };

    // Build the `check.recorded` payload: the CheckResult identity fields plus
    // the executor provenance `checks show` aggregates the hit-rate over. The
    // recorded `duration_ms` is the memoized cost (so `saved_ms` over the HIT
    // rows reflects the real avoided wall-clock).
    let mut payload = json!({
        "name": args.def,
        "memo_key": result.memo_key,
        "tree_hash": result.tree_hash,
        "def_digest": result.def_digest,
        "toolchain_digest": result.toolchain_digest,
        "exit": result.exit,
        "cache_hit": outcome.from_cache,
        "duration_ms": result.duration_ms,
    });
    if let Some(pr) = &args.pr {
        payload
            .as_object_mut()
            .unwrap()
            .insert("pr_id".to_string(), json!(pr));
    }

    // `--store` records the row onto the canonical log; omit for a dry run.
    // The payload Value is scrubbed-on-append inside `record_on_log` (WG-SCRUB):
    // `--def` (`name`)/`--pr` (`pr_id`)/`--principal` are user strings — they
    // CANNOT reach the forever-log unredacted.
    if args.store {
        record_on_log(args, &payload)?;
    }

    Ok(json!({
        "def": args.def,
        "memo_key": result.memo_key,
        "cache_hit": outcome.from_cache,
        "local_executions": outcome.local_executions,
        // Local wall-clock spent THIS run: 0 on a hit (the zero-execution wedge),
        // the measured time on a miss.
        "duration_ms": local_duration_ms,
        // Wall-clock the wedge avoided re-spending THIS run (the memoized cost on
        // a hit; 0 on a miss).
        "saved_ms": saved_ms,
        "exit": result.exit,
        "ok": ok,
        "stored": args.store,
        "recorded_kind": CHECK_RECORDED_KIND,
        "log": args.log.display().to_string(),
    }))
}

/// Append the `check.recorded` event to the canonical `--log` through the SAME
/// guarded seam the other porcelain verbs use: the advisory [`FileLock`] held
/// across load→append→persist, an [`append_authorized`](EventLog::append_authorized)
/// under `Endpoint::Push` (the universal verb — every class may push; a check
/// recording is fleet/CI provenance), and an atomic
/// [`atomic_write`](crate::pr::filelock::atomic_write).
///
/// The `--log` MUST exist (it is the canonical event log — its absence is the
/// explicit `log_not_found`, never silently an empty world). The principal is
/// the caller's `--principal` (default `orchestrator:hugit`).
fn record_on_log(args: &CheckArgs, payload: &serde_json::Value) -> Result<(), PorcelainError> {
    let path = &args.log;
    // Hold the advisory exclusive lock across the whole read-modify-write so a
    // concurrent verb on the same --log gets `log_busy`, never a clobber.
    let _lock = FileLock::acquire(path).map_err(map_lock_error)?;
    let mut log = load_event_log(path)?;

    let principal = args
        .principal
        .clone()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "orchestrator:hugit".to_string());
    // The principal chain is hash-chained too (`--principal` is a user string),
    // so scrub it on the same WG-SCRUB seam — a secret in `--principal` never
    // reaches the forever-log.
    let principal = crate::redaction::scrub(&principal);
    // Scrub-on-append (WG-SCRUB): every user string VALUE in the payload is
    // redacted BEFORE the bytes reach the hash chain; the digest fields
    // (`memo_key`/`tree_hash`/`*_digest`) survive by the helper's exemption.
    let payload = crate::porcelain::scrub_to_canonical(payload.clone());

    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Push,
        CHECK_RECORDED_KIND.to_string(),
        vec![principal],
        payload,
        now_unix_ms(),
    )
    .map_err(|denied| {
        // Unreachable for Push (every class is allowed), but mapped honestly so a
        // future matrix change fails closed with a structured error.
        PorcelainError::new(
            "authz_denied",
            format!(
                "check recording denied by D14 guard: {}",
                denied.reason.code()
            ),
            "a check recording must be driven by a recognized principal",
        )
    })?;

    persist_log(path, &log)
}

/// Persist the canonical event log back to `path` as a pretty `[EventRecord, …]`
/// array (the shape [`load_event_log`] reads) via the atomic temp-then-rename
/// write — a reader/crash sees the whole old or whole new log, never a truncation.
fn persist_log(path: &Path, log: &EventLog) -> Result<(), PorcelainError> {
    let bytes = serde_json::to_vec_pretty(log.records())
        .map_err(|e| PorcelainError::internal(format!("serialize log {}: {e}", path.display())))?;
    filelock::atomic_write(path, &bytes).map_err(map_lock_error)
}

/// Map a [`FileLock`] error into the canonical porcelain envelope: a live holder
/// → `log_busy` (retry-able), an I/O fault → `io`.
fn map_lock_error(e: LockError) -> PorcelainError {
    match e {
        LockError::Busy { path } => PorcelainError::new(
            "log_busy",
            format!(
                "the --log file {} is locked by another hugit verb",
                path.display()
            ),
            "another `hugit` process holds the log lock; retry once it releases \
             (a stale lock is auto-reclaimed after a short window)",
        ),
        LockError::Io { .. } => PorcelainError::new(
            "io",
            e.to_string(),
            "check the --log path is on a writable directory",
        ),
    }
}

/// Map an executor [`ExecError`] into the canonical porcelain envelope.
fn map_exec_error(e: ExecError) -> PorcelainError {
    match e {
        ExecError::Ac(ac) => PorcelainError::new(
            "ac_error",
            format!("the Action Cache layer failed: {ac}"),
            "the local AC store is unreadable/unwritable, or the live AC is \
             misconfigured; check --ac and the AC config",
        ),
        ExecError::Run(msg) => PorcelainError::new(
            "exec_failed",
            format!("the check command could not run: {msg}"),
            "ensure the check command exists on PATH and is executable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(def: &str, cmd: Option<&str>) -> CheckArgs {
        CheckArgs {
            def: def.to_string(),
            log: PathBuf::from("x"),
            store: false,
            cmd: cmd.map(str::to_string),
            root: None,
            toolchain: None,
            pr: None,
            principal: None,
            ac: None,
        }
    }

    #[test]
    fn builtins_resolve_to_real_commands() {
        assert!(builtin_command("fmt").unwrap().contains("fmt"));
        assert!(builtin_command("clippy").unwrap().contains("clippy"));
        assert!(builtin_command("test").unwrap().contains("test"));
        assert!(builtin_command("nope").is_none());
    }

    #[test]
    fn ad_hoc_def_requires_cmd() {
        let err = resolve_def(&args_for("echo-check", None)).unwrap_err();
        assert_eq!(err.kind(), "unknown_def");
    }

    #[test]
    fn ad_hoc_def_with_cmd_builds_a_valid_def() {
        let def = resolve_def(&args_for("echo-check", Some("true"))).unwrap();
        assert_eq!(def.command, "true");
        // The def_digest was canonicalized (non-empty) by the validator.
        assert!(!def.def_digest.is_empty());
    }

    #[test]
    fn store_path_defaults_alongside_the_log() {
        assert_eq!(
            with_extension(Path::new("/tmp/run.json"), "ac"),
            PathBuf::from("/tmp/run.json.ac")
        );
    }

    #[test]
    fn process_runner_captures_real_exit_and_duration() {
        let def = CheckDef {
            def_digest: String::new(),
            command: "true".to_string(),
            inputs: vec![],
            toolchain_ref: "tc".to_string(),
            env_manifest: String::new(),
            glob_set: vec![],
        };
        let r = ProcessRunner
            .run(&def, "key123", "tree", "def", "tc")
            .unwrap();
        assert_eq!(r.exit, 0);
        assert_eq!(r.memo_key, "key123");
        assert_eq!(r.tree_hash, "tree");

        let fail = CheckDef {
            command: "false".to_string(),
            ..def
        };
        let r2 = ProcessRunner.run(&fail, "k2", "t", "d", "tc").unwrap();
        assert_ne!(r2.exit, 0, "a failing command reports a non-zero exit");
    }
}
