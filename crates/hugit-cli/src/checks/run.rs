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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hugit_checks::client::ac::ActionCache;
use hugit_checks::client::executor::{self, CheckRunner, ExecError};
use hugit_checks::client::memo_key::FileContent;
use hugit_contracts::{CheckDef, CheckResult};
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{CHECK_RECORDED_KIND, CheckArgs, load_event_log};
use crate::porcelain::PorcelainError;
use crate::pr::filelock::{self, FileLock, LockError};

/// The default execution timeout (seconds) when `--timeout-secs` is not supplied.
/// A bounded ceiling is a SHIP-BLOCKER fix (WG-CHECK-ROBUST): an unbounded
/// `Command::output()` on a hanging command (`sleep infinity`) blocks forever
/// holding the `--log` lock. 300 s (5 min) comfortably covers a real gate run
/// (fmt/clippy/test) while turning a hang into a structured `check_timeout` error.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// How often the timeout watcher polls the child for completion. Small enough to
/// kill promptly on expiry, large enough not to busy-spin.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

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

/// The fallback toolchain marker used ONLY when `--toolchain` is omitted AND the
/// active toolchain identity cannot be probed (e.g. `rustc` is not on PATH). It is
/// deliberately distinct from any real digest so a probe failure is visible rather
/// than colliding with a hashed identity.
const TOOLCHAIN_PROBE_UNAVAILABLE: &str = "toolchain-unprobed";

/// Resolve the toolchain digest (third memo axis) for this run.
///
/// An explicit `--toolchain` wins verbatim (the caller content-addressed it). When
/// omitted, we compute a REAL digest of the ACTIVE toolchain identity — the
/// SHA-256 of `rustc --version --verbose` output — so changing the compiler busts
/// the memo key. (WG-CACHE: the prior `local-toolchain` CONSTANT made axis 3 fake,
/// so a green cached under Rust A wrongly HIT under Rust B.) If `rustc` cannot be
/// run, fall back to a distinct marker rather than fabricating a digest.
fn resolve_toolchain_digest(args: &CheckArgs) -> String {
    if let Some(tc) = args.toolchain.clone().filter(|t| !t.trim().is_empty()) {
        return tc;
    }
    default_toolchain_digest()
}

/// Compute the active-toolchain digest: `sha256(rustc --version --verbose)`,
/// lowercase hex. The verbose form embeds the release, commit hash, commit date,
/// host triple, and LLVM version — every byte that can change the gate's outcome —
/// so two distinct toolchains key distinctly. A probe failure returns the
/// [`TOOLCHAIN_PROBE_UNAVAILABLE`] marker (honest, never a fake hex digest).
fn default_toolchain_digest() -> String {
    match Command::new("rustc")
        .args(["--version", "--verbose"])
        .output()
    {
        Ok(out) if out.status.success() => {
            let mut hasher = Sha256::new();
            hasher.update(&out.stdout);
            hex::encode(hasher.finalize())
        }
        _ => TOOLCHAIN_PROBE_UNAVAILABLE.to_string(),
    }
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
        // The toolchain axis is a REAL digest when `--toolchain` is omitted — the
        // hash of the active `rustc --version --verbose` — so a toolchain change
        // busts the memo key (WG-CACHE: the old `local-toolchain` constant made
        // axis 3 fake, hitting a green cached under Rust A under Rust B).
        toolchain_ref: resolve_toolchain_digest(args),
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
///
/// The walk is cycle-safe (WH-CHECK): a directory symlink loop (`a/link → a`,
/// common in monorepos) is broken by a visited-real-path set AND a depth cap, so
/// it returns a structured snapshot rather than recursing into a SIGSEGV.
fn snapshot_tree(
    root: &Path,
    glob_set: &[String],
    excluded: &std::collections::HashSet<PathBuf>,
) -> BTreeMap<String, FileContent> {
    let mut files = BTreeMap::new();
    let mut visited = std::collections::HashSet::new();
    collect_files(root, root, glob_set, excluded, &mut files, &mut visited, 0);
    files
}

/// The set of CANONICAL paths to exclude from the tree axis: hugit's own
/// wedge-state files plus their `.lock`/`.tmp` sidecars. Canonicalized so the
/// match works regardless of how `--root` reaches them (the walk also
/// canonicalizes each candidate). A path that does not yet exist canonicalizes to
/// nothing — harmless, it simply contributes no exclusion (the file isn't there
/// to be hashed either). The `.tmp` sidecars are name-prefixed (`.<file>.tmp-…`)
/// so we exclude by the prefix check in [`collect_files`], not by exact path.
fn state_file_exclusions(state_files: &[&Path]) -> std::collections::HashSet<PathBuf> {
    let mut set = std::collections::HashSet::new();
    for f in state_files {
        // The file itself.
        if let Ok(c) = std::fs::canonicalize(f) {
            set.insert(c);
        }
        // Its `<file>.lock` sidecar (the FileLock advisory lock).
        let mut lock = f.as_os_str().to_os_string();
        lock.push(".lock");
        if let Ok(c) = std::fs::canonicalize(PathBuf::from(lock)) {
            set.insert(c);
        }
    }
    set
}

/// Hard depth ceiling for the tree walk. A real source tree is a handful of
/// levels deep; 64 is far above any legitimate layout while bounding even a
/// pathological non-symlink-but-deep tree (the visited-set already breaks true
/// symlink cycles — this is the belt-and-suspenders guard).
const MAX_WALK_DEPTH: usize = 64;

/// Recursively walk `dir`, collecting glob-matched files relative to `base`.
/// Skips `target/`, `.git/`, and the worktree scratch dir so the tree axis is the
/// SOURCE subtree, not build output (which would make every run a miss).
///
/// Cycle-safe (WH-CHECK SHIP-BLOCKER): a directory symlink loop would otherwise
/// recurse forever → stack overflow / SIGSEGV. We guard with two independent
/// caps: (1) a `visited` set of CANONICAL (symlink-resolved) directory paths so a
/// link that points back into an already-walked real directory is not re-entered;
/// (2) a `depth` ceiling ([`MAX_WALK_DEPTH`]) so even a canonicalize failure can
/// never recurse unboundedly. On either trip we simply stop descending that
/// branch — the snapshot is a faithful subset, never a crash.
#[allow(clippy::too_many_arguments)]
fn collect_files(
    base: &Path,
    dir: &Path,
    glob_set: &[String],
    excluded: &std::collections::HashSet<PathBuf>,
    out: &mut BTreeMap<String, FileContent>,
    visited: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    // Depth cap: a degenerate (or adversarial) tree never overflows the stack.
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    // Resolve this directory to its real path and refuse to re-enter one we have
    // already walked — this is what breaks a directory symlink cycle. A dir whose
    // real path cannot be resolved (a dangling link, a permission fault) is simply
    // not descended (fail-safe: skip, never crash).
    let Ok(real) = std::fs::canonicalize(dir) else {
        return;
    };
    if !visited.insert(real) {
        return; // already walked this real directory — a cycle/alias, stop.
    }

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
            collect_files(base, &path, glob_set, excluded, out, visited, depth + 1);
        } else if !is_excluded_state_file(&path, excluded)
            && let Ok(rel) = path.strip_prefix(base)
        {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if hugit_checks::client::glob::matches_any(glob_set, &rel)
                && let Ok(bytes) = std::fs::read(&path)
            {
                out.insert(rel, bytes);
            }
        }
    }
}

/// Whether `path` is one of hugit's own wedge-state files (the `--log`, the
/// `--ac`, or a `.lock`/`.tmp-…` sidecar) that must NOT contribute to the tree
/// axis (WH-CHECK cmd-memoize fix). Matches by canonical path (so the walk's
/// canonicalized candidates line up with the canonicalized exclusion set) and
/// additionally skips the transient atomic-write temp files, which are named
/// `.<file>.tmp-<pid>-<nanos>` and could be caught mid-rename.
fn is_excluded_state_file(path: &Path, excluded: &std::collections::HashSet<PathBuf>) -> bool {
    if let Ok(c) = std::fs::canonicalize(path)
        && excluded.contains(&c)
    {
        return true;
    }
    // The atomic-write temp sidecar pattern: `.<name>.tmp-<pid>-<nanos>`. These
    // are hugit's own crash-consistent write staging, never a check input.
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.') && n.contains(".tmp-"))
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
///
/// Execution is BOUNDED by `timeout` (WG-CHECK-ROBUST): a hanging command
/// (`sleep infinity`) is killed at the deadline and surfaces as
/// [`ExecError::Timeout`] rather than blocking forever holding the `--log` lock.
struct ProcessRunner {
    /// The per-check execution ceiling. On expiry the child is killed and the run
    /// is a structured timeout — never a memoized result.
    timeout: Duration,
}

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
        // Spawn (not `.output()`) so we keep the child handle and can kill it on
        // the deadline. stdout/stderr are piped so the wait-with-output drains
        // them without polluting the verb's JSON contract. The child is spawned in
        // its OWN process group (`process_group(0)` — std-only, Unix) so the whole
        // tree (including a backgrounded grandchild) can be killed as a unit on a
        // timeout (WH-CHECK lock-poison fix item b: `child.kill()` alone reaps only
        // the direct child, letting an orphan grandchild survive past the deadline
        // and a backgrounding command bypass the wall-time ceiling).
        let mut command = shell_command(&def.command);
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // The child becomes the leader of a fresh group whose pgid == its pid,
            // so `kill -<pid>` (negative pid = the group) reaps the whole subtree.
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|e| ExecError::Run(format!("spawn `{}`: {e}", def.command)))?;

        // Poll for completion up to the deadline; kill the whole GROUP + reap on
        // expiry (WH-CHECK: a backgrounding child must not outlive the ceiling).
        let exit = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code().unwrap_or(-1),
                Ok(None) => {
                    if start.elapsed() >= self.timeout {
                        // Bounded ceiling reached: kill the whole process GROUP (so
                        // orphan grandchildren die too), reap the direct child (no
                        // zombie), and surface a structured timeout. The result is
                        // NOT stored — a hang never poisons the cache.
                        kill_group(&mut child);
                        return Err(ExecError::Timeout(self.timeout.as_secs()));
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    kill_group(&mut child);
                    return Err(ExecError::Run(format!("wait `{}`: {e}", def.command)));
                }
            }
        };
        let duration_ms = start.elapsed().as_millis() as u64;

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

/// Kill the child's whole process GROUP and reap the direct child (no zombie).
///
/// On Unix the child was spawned with `process_group(0)` so its pgid equals its
/// pid; sending the signal to the negative pid (`kill -<pid>`) reaches every
/// descendant — including a backgrounded grandchild that `child.kill()` alone
/// would orphan past the timeout. We deliver SIGTERM then SIGKILL through the
/// `kill(1)` binary (std-only — no libc/nix dep), then `wait()` the direct child
/// so it is reaped. On non-Unix the std `child.kill()` is the best available.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id();
        // Negative pid targets the whole group. TERM first (graceful), then KILL
        // (so a TERM-ignoring child still dies). Best-effort: a failure only risks
        // an orphan the OS reaps on the parent's exit — never a correctness loss.
        let group = format!("-{pid}");
        let _ = Command::new("kill").arg("-TERM").arg(&group).status();
        let _ = Command::new("kill").arg("-KILL").arg(&group).status();
    }
    // Always also signal + reap the direct child (covers non-Unix and guarantees
    // no zombie even if the group signal raced the child's own exit).
    let _ = child.kill();
    let _ = child.wait();
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
///
/// The returned [`FileAc`] locks the cache file ONLY during each individual
/// `lookup`/`store` op (acquire→read/write→release), NOT across the unbounded
/// execute (WH-CHECK lock-poison fix item a). A hung/slow/orphaning command can
/// therefore never hold the `.ac` lock across its whole runtime and poison every
/// concurrent `hugit check`. The accepted cost is the small benign double-exec
/// window the original lookup-then-store TOCTOU already had — strictly better
/// than a lock-poison hang; the canonical `--log` append still serializes via its
/// own lock, and `check --store`'s memo_key dedup keeps the log idempotent so a
/// double-exec records at most ONE `check.recorded`. A live AC over HTTP needs no
/// local lock — that arm would not take one.
fn select_ac(args: &CheckArgs) -> Result<AcBackend, PorcelainError> {
    let store = args
        .ac
        .clone()
        .unwrap_or_else(|| with_extension(&args.log, "ac"));
    Ok(AcBackend::Local(FileAc::new(store)))
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

/// One tamper-evident on-disk cache entry: the stored [`CheckResult`] plus a
/// `self_hash` — the SHA-256 over the result's canonical bytes — captured at
/// store time. On lookup the hash is RECOMPUTED and compared; a mismatch (someone
/// edited the `.ac` file to flip `exit`/`ok` into a forged green) is treated as a
/// MISS, so the check RE-EXECUTES rather than serving the tampered entry.
///
/// # Local-trust boundary (honest scope)
///
/// This detects ACCIDENTAL or LOCAL tampering of the on-disk cache — the
/// `.ac`-file false-green vector. It is NOT cross-tenant cryptographic
/// authenticity: an attacker who can edit the cache can recompute the self-hash.
/// Cross-tenant authenticity is the P2 CoreLink AC (HMAC/auth over the wire) —
/// Seam A. The two layers compose: this closes the LOCAL hole today; P2 closes
/// the REMOTE one. Both lookups additionally run `verify_hit` (the axis↔key
/// content-address guard), so a record keyed to a different action is rejected
/// regardless of the self-hash.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedEntry {
    /// The memoized check result.
    result: CheckResult,
    /// SHA-256 (lowercase hex) over the result's canonical JSON bytes, captured at
    /// store time. Recomputed on lookup; a mismatch ⇒ tampered ⇒ MISS.
    self_hash: String,
}

/// Compute the tamper-evidence self-hash of a [`CheckResult`]: `sha256` over its
/// compact `serde_json::to_vec` bytes. Single-sourced so store and lookup hash
/// IDENTICAL bytes — serde emits fields in struct-declaration order (a fixed,
/// deterministic schema), so the digest round-trips stably store→disk→lookup.
fn entry_self_hash(result: &CheckResult) -> String {
    let bytes = serde_json::to_vec(result).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hex::encode(hasher.finalize())
}

/// A file-backed local Action Cache implementing the reference [`ActionCache`]
/// semantics (content-keyed, store-then-hit) over a single JSON file holding a
/// `{ memo_key: CachedEntry }` map. It is the local default so the wedge survives
/// across processes WITHOUT P2; the live HTTP client is the P2 swap behind the
/// same trait.
///
/// # Lock scope (WH-CHECK lock-poison fix)
///
/// The advisory [`FileLock`] is taken ONLY around each individual cache file op —
/// `lookup` (acquire→read→release) and `store` (acquire→read-modify-write→release)
/// — NOT across the unbounded execute that happens between them. A hung/slow or
/// process-orphaning command therefore can NEVER hold the `.ac` lock across its
/// whole runtime and poison every concurrent `hugit check` with `ac_busy`. The
/// prior design held the lock for the whole lookup→execute→store op; that closed
/// a TOCTOU but opened a lock-poison hang, which is strictly worse. We accept the
/// small benign double-exec window the original lookup-then-store TOCTOU already
/// had (two concurrent cold misses may both execute) because the canonical-log
/// append still serializes via its own lock AND `check --store` dedups on
/// `memo_key`, so a double-exec records at most ONE `check.recorded` — no KPI
/// inflation. Writes go through [`atomic_write`](crate::pr::filelock::atomic_write)
/// so on-disk state is crash-consistent. Every entry is tamper-evident
/// ([`CachedEntry`]).
struct FileAc {
    path: PathBuf,
}

impl FileAc {
    /// Construct a file-backed AC over `path`. No lock is held by the backend
    /// itself — each `lookup`/`store` op takes the lock only for its own short
    /// critical section (the lock-poison fix), so the execute between them runs
    /// UNLOCKED.
    fn new(path: PathBuf) -> Self {
        FileAc { path }
    }

    /// Read the `{memo_key: CachedEntry}` map from disk; an absent/empty/corrupt
    /// file is an empty cache (a fresh cache is legitimately absent — this is the
    /// local-state seam, not the canonical event log whose absence is an error).
    /// A file in the OLD bare-`CheckResult` shape (no `self_hash`) fails to decode
    /// into `CachedEntry` and is therefore treated as empty — a forward-only
    /// migration that fails SAFE (re-execute), never serving an unverifiable entry.
    fn read_map(&self) -> BTreeMap<String, CachedEntry> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        }
    }
}

impl ActionCache for FileAc {
    fn lookup(&self, key: &str) -> Result<Option<CheckResult>, hugit_checks::client::ac::AcError> {
        // Lock only the read of the cache file (acquire→read→release): a concurrent
        // store must not observe a half-written map. The lock is dropped the moment
        // this scope ends, BEFORE any execute — never held across the command.
        let map = {
            let _lock = acquire_ac_lock(&self.path)?;
            self.read_map()
        };
        let Some(entry) = map.get(key) else {
            return Ok(None);
        };
        // Tamper-evidence (WG-CACHE): recompute the self-hash over the stored
        // result's canonical bytes. A mismatch means the `.ac` file was edited
        // (e.g. `exit`→0 to forge a green) — treat it as a MISS so the check
        // RE-EXECUTES; NEVER serve the tampered entry as an authoritative hit.
        if entry_self_hash(&entry.result) != entry.self_hash {
            return Ok(None);
        }
        // Content-address guard (WG-CACHE wiring): the stored record's three memo
        // axes must key to the looked-up key. A record keyed to a different action
        // is a MISS, not a blind hit — the same law the HTTP client enforces.
        if hugit_checks::client::ac::verify_hit(key, &entry.result).is_err() {
            return Ok(None);
        }
        Ok(Some(entry.result.clone()))
    }

    fn store(&self, result: &CheckResult) -> Result<(), hugit_checks::client::ac::AcError> {
        use hugit_checks::client::ac::AcError;
        // Lock the whole read-modify-write of the cache file so two concurrent
        // stores merge instead of clobbering, then release. The lock is taken HERE
        // (per-op), never held across the execute that produced `result`.
        let _lock = acquire_ac_lock(&self.path)?;
        let mut map = self.read_map();
        let entry = CachedEntry {
            self_hash: entry_self_hash(result),
            result: result.clone(),
        };
        map.insert(result.memo_key.clone(), entry);
        let bytes = serde_json::to_vec_pretty(&map)
            .map_err(|e| AcError::Decode(format!("AC store serialize: {e}")))?;
        filelock::atomic_write(&self.path, &bytes)
            .map_err(|e| AcError::Transport(format!("AC store write: {e}")))?;
        Ok(())
    }
}

/// How many times a per-op AC-file lock acquisition retries on a LIVE holder
/// before giving up. Because the lock is now held ONLY for a sub-millisecond
/// read or read-modify-write (NOT across the unbounded execute — WH-CHECK), a
/// contender almost always clears within a few short sleeps, so a transient
/// per-op collision resolves silently instead of surfacing `ac_busy`.
const AC_LOCK_RETRIES: u32 = 50;

/// Sleep between AC-file lock retries. 50 × 4 ms = 200 ms total budget — ample
/// for a sub-ms critical section to clear, far below any real command runtime.
const AC_LOCK_RETRY_SLEEP: Duration = Duration::from_millis(4);

/// Acquire the per-op AC-file lock, retrying briefly on a LIVE holder.
///
/// The lock now guards only the cache file's own short read / read-modify-write
/// (never the execute), so two concurrent `hugit check` collide only for the
/// microseconds one is touching the `.ac` map. We retry a bounded number of times
/// so that benign per-op contention is invisible; only a genuinely stuck holder
/// (which the age-based stale-lock takeover also reclaims) exhausts the budget
/// and surfaces as a retryable `ac_busy` through `ExecError::Ac`. An I/O fault is
/// surfaced immediately.
fn acquire_ac_lock(path: &Path) -> Result<FileLock, hugit_checks::client::ac::AcError> {
    use hugit_checks::client::ac::AcError;
    for _ in 0..AC_LOCK_RETRIES {
        match FileLock::acquire(path) {
            Ok(lock) => return Ok(lock),
            Err(LockError::Busy { .. }) => std::thread::sleep(AC_LOCK_RETRY_SLEEP),
            Err(e @ LockError::Io { .. }) => {
                return Err(AcError::Transport(format!("AC store lock: {e}")));
            }
        }
    }
    Err(AcError::Transport(format!(
        "ac_busy: the AC store {} stayed locked by another hugit check",
        path.display()
    )))
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
    // Axis 3 is taken from the resolved def's `toolchain_ref` — the SAME value
    // `resolve_def` baked into axis 2 (`compute_def_digest` hashes `toolchain_ref`)
    // — so the two axes can never disagree. When `--toolchain` is omitted this is
    // the REAL active-toolchain digest, so a compiler change busts the key.
    let toolchain_digest = def.toolchain_ref.clone();

    // log-not-found law (WG-CHECK-ROBUST): a `--log` that does not exist is the
    // explicit `log_not_found`/exit-2 error REGARDLESS of `--store` — a check
    // against a typo'd log is an error, never a silent dry green. (`--store` later
    // re-loads it under the lock; this is the early, store-independent guard.)
    if !args.log.exists() {
        return Err(PorcelainError::log_not_found(&args.log));
    }

    let root = args
        .root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Exclude hugit's OWN wedge-state files from the tree axis (WH-CHECK
    // cmd-memoize fix). An ad-hoc def globs `**/*`, which would otherwise match the
    // `--log`, the `--ac` cache, and their `.lock`/`.tmp` sidecars when they live
    // under `--root` — so storing the cold result MUTATES the tree the very next
    // run hashes, busting the memo key and making every re-run a MISS. These files
    // are hugit STATE, never a check INPUT, so they must not contribute to the key.
    let ac_path = args
        .ac
        .clone()
        .unwrap_or_else(|| with_extension(&args.log, "ac"));
    let excluded = state_file_exclusions(&[&args.log, &ac_path]);
    let files = snapshot_tree(&root, &def.glob_set, &excluded);
    let ac = select_ac(args)?;
    let runner = ProcessRunner {
        timeout: Duration::from_secs(args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS)),
    };

    // The wedge: derive the three-axis key, look it up, execute-on-miss + store.
    // The `ac` backend locks the cache file ONLY per-op (lookup/store), never
    // across the execute (WH-CHECK lock-poison fix), so a hung command can never
    // poison the AC lock for concurrent checks.
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
    // Stamp the optional PR id without `unwrap` (advisory #5: a non-panicking
    // form). `payload` is a freshly-built `json!` object, so `as_object_mut` is
    // always `Some`; the `if let` keeps that fact a SAFE no-op rather than a
    // latent panic if the literal ever changes shape.
    if let (Some(pr), Some(obj)) = (&args.pr, payload.as_object_mut()) {
        obj.insert("pr_id".to_string(), json!(pr));
    }

    // `--store` records the row onto the canonical log; omit for a dry run.
    // The payload Value is scrubbed-on-append inside `record_on_log` (WG-SCRUB):
    // `--def` (`name`)/`--pr` (`pr_id`)/`--principal` are user strings — they
    // CANNOT reach the forever-log unredacted.
    //
    // Idempotency (WH-CHECK): `record_on_log` dedups on `memo_key` — if this
    // memo_key already has a `check.recorded` row on the log, it appends NOTHING
    // and reports `already_recorded:true`. N identical `check --store` runs thus
    // leave EXACTLY ONE row, so `checks show`'s count + hit_rate cannot be
    // inflated by re-runs (the same `already_*` law verdict/pr/intent follow).
    let already_recorded = if args.store {
        !record_on_log(args, &payload)?
    } else {
        false
    };

    // `--cmd` is honoured for an ad-hoc def but IGNORED for a built-in (the frozen
    // gate command wins). Surface `cmd_ignored:true` so the agent KNOWS its `--cmd`
    // had no effect, rather than silently running a different command (WG-CHECK-
    // ROBUST: the smaller honest fix over hard-erroring).
    let cmd_ignored = builtin_command(&args.def).is_some()
        && args.cmd.as_ref().is_some_and(|c| !c.trim().is_empty());

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
        // True when `--store` found this memo_key already on the log and appended
        // NOTHING (idempotent re-run) — the `already_*` law (WH-CHECK dedup).
        "already_recorded": already_recorded,
        // True when a built-in `--def` ignored a supplied `--cmd` (honest signal).
        "cmd_ignored": cmd_ignored,
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
///
/// Returns `Ok(true)` when a NEW `check.recorded` row was appended, `Ok(false)`
/// when this `memo_key` was already on the log so NOTHING was appended (the
/// idempotency dedup — WH-CHECK). Both are exit-0 successes; the difference is
/// surfaced as `already_recorded` to the agent.
fn record_on_log(args: &CheckArgs, payload: &serde_json::Value) -> Result<bool, PorcelainError> {
    let path = &args.log;
    // Hold the advisory exclusive lock across the whole read-modify-write so a
    // concurrent verb on the same --log gets `log_busy`, never a clobber. Holding
    // it across the dedup SCAN too makes the check-then-append atomic: two
    // concurrent `check --store` for the same memo_key cannot both pass the scan
    // and both append (one serializes on the log lock, then sees the other's row).
    let _lock = FileLock::acquire(path).map_err(map_lock_error)?;
    let mut log = load_event_log(path)?;

    // Idempotency dedup (WH-CHECK [HIGH]): if a `check.recorded` event with THIS
    // (memo_key, cache_hit) pair is already on the log, append NOTHING and report
    // not-newly-recorded. N identical `check --store` runs thus leave at most TWO
    // rows total — one cold MISS and one warm HIT — NEVER N, so `check_count` +
    // `hit_rate_pct` cannot be inflated by re-runs (the fabrication harm is closed:
    // the KPIs are stable no matter how many times you re-run the same check).
    //
    // The pair (not memo_key alone) is the honest key because the wedge's value is
    // PRECISELY that the same action recorded once as a MISS and once as a HIT —
    // collapsing both to one row would erase the hit-rate the wedge exists to show.
    // A third+ identical run (also a HIT, same key) IS deduped — that is the
    // re-run that would otherwise fabricate. A real input/toolchain change keys
    // differently and is a legitimately new row.
    if let Some(memo_key) = payload.get("memo_key").and_then(Value::as_str) {
        let cache_hit = payload.get("cache_hit").and_then(Value::as_bool);
        if check_already_recorded(&log, memo_key, cache_hit) {
            return Ok(false);
        }
    }

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

    persist_log(path, &log)?;
    Ok(true)
}

/// Whether a `check.recorded` event with this `(memo_key, cache_hit)` pair is
/// already on `log` (the idempotency predicate — WH-CHECK).
///
/// The recorded `memo_key` is a content-addressed digest exempt from the WG-SCRUB
/// redaction, so it survives onto the log verbatim and a re-run's freshly-derived
/// memo_key matches it byte-for-byte. Pairing it with `cache_hit` lets the wedge
/// keep BOTH a cold-MISS row and a warm-HIT row for the same action (the hit-rate
/// the wedge demonstrates) while deduping any FURTHER identical run — so the KPIs
/// are stable no matter how many times the same check is re-run.
fn check_already_recorded(log: &EventLog, memo_key: &str, cache_hit: Option<bool>) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == CHECK_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| {
            v.get("memo_key").and_then(Value::as_str) == Some(memo_key)
                && v.get("cache_hit").and_then(Value::as_bool) == cache_hit
        })
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
        // A hang that hit the bounded ceiling: structured `check_timeout` (exit 2),
        // the child was killed and the lock released — never an infinite block.
        ExecError::Timeout(secs) => PorcelainError::new(
            "check_timeout",
            format!("the check command exceeded the {secs}s timeout and was killed"),
            "the check ran past its deadline; raise --timeout-secs if it legitimately \
             needs longer, or fix the command if it hangs",
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
            timeout_secs: None,
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
        let runner = ProcessRunner {
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        };
        let def = CheckDef {
            def_digest: String::new(),
            command: "true".to_string(),
            inputs: vec![],
            toolchain_ref: "tc".to_string(),
            env_manifest: String::new(),
            glob_set: vec![],
        };
        let r = runner.run(&def, "key123", "tree", "def", "tc").unwrap();
        assert_eq!(r.exit, 0);
        assert_eq!(r.memo_key, "key123");
        assert_eq!(r.tree_hash, "tree");

        let fail = CheckDef {
            command: "false".to_string(),
            ..def
        };
        let r2 = runner.run(&fail, "k2", "t", "d", "tc").unwrap();
        assert_ne!(r2.exit, 0, "a failing command reports a non-zero exit");
    }

    #[test]
    fn process_runner_times_out_a_hanging_command() {
        // A command that hangs is killed at the deadline and surfaces a structured
        // timeout — it does NOT block forever (WG-CHECK-ROBUST). 1 s keeps the unit
        // test fast while still proving the kill path.
        let runner = ProcessRunner {
            timeout: Duration::from_secs(1),
        };
        let def = CheckDef {
            def_digest: String::new(),
            command: "sleep 30".to_string(),
            inputs: vec![],
            toolchain_ref: "tc".to_string(),
            env_manifest: String::new(),
            glob_set: vec![],
        };
        let start = Instant::now();
        let err = runner.run(&def, "k", "t", "d", "tc").unwrap_err();
        assert!(
            matches!(err, ExecError::Timeout(1)),
            "a hanging command times out: {err:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the timeout fired promptly, not after the full sleep"
        );
    }

    #[test]
    fn tamper_evident_cache_treats_a_flipped_exit_as_a_miss() {
        // WG-CACHE core: editing the stored `.ac` to flip `exit`→0 (forge a green)
        // is detected by the self-hash mismatch → lookup returns MISS, never the
        // tampered entry.
        let dir = std::env::temp_dir().join(format!("hugit-wg-cache-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ac_path = dir.join("ac.json");

        // Store a genuine RED result (exit 1) through the locked backend.
        let key = hugit_refstore::compute_memo_key("tree", "def", "tc");
        let red = CheckResult {
            memo_key: key.clone(),
            tree_hash: "tree".to_string(),
            def_digest: "def".to_string(),
            toolchain_digest: "tc".to_string(),
            exit: 1,
            artifacts: vec![],
            stdout_ref: String::new(),
            stderr_ref: String::new(),
            duration_ms: 5,
            runner_ref: "local".to_string(),
            produced_at: 0,
        };
        {
            let ac = FileAc::new(ac_path.clone());
            ac.store(&red).unwrap();
            // A clean lookup is a HIT carrying the stored exit:1.
            let hit = ac.lookup(&key).unwrap().expect("stored entry hits");
            assert_eq!(hit.exit, 1, "untampered entry serves the real red result");
        } // per-op lock released after each call

        // Tamper: flip `exit`→0 in the on-disk entry WITHOUT updating the self-hash.
        let raw = std::fs::read_to_string(&ac_path).unwrap();
        let forged = raw.replace("\"exit\": 1", "\"exit\": 0");
        assert_ne!(forged, raw, "the tamper edit changed the bytes");
        std::fs::write(&ac_path, &forged).unwrap();

        // Lookup now recomputes the self-hash over the forged bytes; mismatch ⇒
        // MISS. The forged green is NEVER served.
        let ac = FileAc::new(ac_path.clone());
        assert!(
            ac.lookup(&key).unwrap().is_none(),
            "a tampered (flipped exit) entry is a MISS, not a forged green hit"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
