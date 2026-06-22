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
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hugit_checks::client::ac::{ActionCache, HttpAcClient};
use hugit_checks::client::executor::{self, CheckRunner, ExecError};
use hugit_checks::client::memo_key::{FileContent, frame_file_with_mode};
use hugit_contracts::{CheckDef, CheckResult};
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{CHECK_RECORDED_KIND, CheckRunArgs, load_event_log};
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

/// Per-stream capture ceiling for the drain threads (WJ-CHECK-DRAIN). A
/// flooding command (`yes`, `cat /dev/urandom`) can emit output without bound;
/// reading it all into memory would OOM the agent. We cap at 4 MiB per stream:
/// enough for any real gate's diagnostic output, harmless for a `yes`-style
/// flood (the drain discards bytes past the cap while still consuming from the
/// pipe so the child is never blocked). The captured bytes are not stored — they
/// are silently discarded per the verb's JSON-contract rule (a check's output
/// never pollutes `hugit`'s own stdout). The cap is a safety budget, not a
/// truncation of meaningful content: gate diagnostics rarely exceed 1 MiB.
const PIPE_CAPTURE_CAP: usize = 4 * 1024 * 1024; // 4 MiB per stream

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

/// The input glob for a built-in def — Rust sources + the manifests + the
/// per-gate TOOLCHAIN-CONFIG files that change the gate's outcome. Scopes the
/// `tree_hash` axis so an edit to any result-affecting input is a MISS and an
/// unrelated edit (docs, fixtures the gate does not read) is a HIT.
///
/// # WO-GLOBMISS — the bounded config files are first-class inputs (not optional)
///
/// The prior set was `["**/*.rs", "**/Cargo.toml", "Cargo.lock"]`. But the
/// built-in gate COMMANDS read result-affecting config OUTSIDE that set, so a
/// change to one left the memo key unchanged → a warm HIT served a STALE GREEN
/// where a cold run now FAILS (lead repro: add `rustfmt.toml(max_width=1)` under
/// `--def fmt`, the warm re-run still served `exit 0` while `cargo fmt --check`
/// is `exit 1`). These config files are BOUNDED, KNOWN, result-affecting inputs
/// of the frozen gate commands, so they MUST be in the tree axis — a change to
/// one is now a MISS. This is the SAME failure class as the N-1 mode-bit P0 (a
/// real per-input the tree axis omitted), on the glob-scope axis the mode fix did
/// not touch.
///
/// The set is PER-DEF, not one widened shared set: `fmt` only reads the rustfmt
/// config (adding the clippy/cargo files would needlessly bust a fmt hit when an
/// unrelated `clippy.toml` changes); `clippy`/`test` additionally read the lint
/// config, the cargo build config (rustflags etc.), and the toolchain pin. The
/// glob stays otherwise NARROW (NOT `**/*`) so the wedge's hit-rate is preserved:
/// a doc/`*.md`/fixture-the-gate-does-not-read edit is still a HIT.
///
/// ## Honest residual — the unbounded-fixture vector is NOT closed here (P2 seam)
///
/// This closes the BOUNDED toolchain-config vectors. It does NOT (and cannot by
/// enumeration) close the UNBOUNDED case: a built-in `test` can read an ARBITRARY
/// fixture file (e.g. `tests/data/foo.json`, an `include_str!`/`include_bytes!`
/// target, a `build.rs`-emitted path) that no bounded glob can predict. A change
/// to such a file changes the gate's outcome while leaving the memo key unchanged
/// → a stale green. That unbounded-read case is the SAME class as the disclosed P2
/// hermetic-execution / files-outside-the-captured-tree seam (the runner-side
/// isolated rootfs of the runner fabric — where the action physically cannot read
/// outside the seeded tree axis): it is NOT closable locally by enumerating more
/// globs. We do NOT pretend the enumeration is complete for `test`'s fixtures;
/// the bounded config vectors are closed, the unbounded-fixture vector stays the
/// P2 seam (for the lead to track in pending-seams).
fn builtin_glob_set(name: &str) -> Vec<String> {
    // Shared base: every built-in gate's outcome depends on the Rust sources and
    // the workspace manifests + lock.
    let mut set = vec![
        "**/*.rs".to_string(),
        "**/Cargo.toml".to_string(),
        "Cargo.lock".to_string(),
    ];
    match name {
        // `cargo fmt --all --check` reads ONLY the rustfmt config (both the
        // canonical and dotfile names, at any depth). It does NOT read the lint /
        // cargo-build / toolchain config, so we deliberately do NOT capture those
        // for fmt — adding them would bust a fmt HIT on an unrelated change.
        "fmt" => {
            set.push("**/rustfmt.toml".to_string());
            set.push("**/.rustfmt.toml".to_string());
        }
        // `cargo clippy`/`cargo test` are influenced by the lint config
        // (`clippy.toml`), the cargo build config (`.cargo/config.toml` /
        // `.cargo/config` — `rustflags`, lints, `[build]` target dir, registry),
        // and the toolchain pin (`rust-toolchain` / `rust-toolchain.toml` —
        // changing the active compiler/components). All bounded + result-affecting.
        "clippy" | "test" => {
            set.push("**/clippy.toml".to_string());
            set.push("**/.clippy.toml".to_string());
            set.push("**/.cargo/config.toml".to_string());
            set.push("**/.cargo/config".to_string());
            set.push("**/rust-toolchain.toml".to_string());
            set.push("**/rust-toolchain".to_string());
        }
        // A name with no built-in command never reaches here (the caller takes the
        // ad-hoc `**/*` path); the base set is a safe, narrow default regardless.
        _ => {}
    }
    set
}

/// The KNOWN, BOUNDED toolchain-config filenames a built-in gate reads from a
/// directory, PER DEF (Wave P FIX A). Cargo and rustfmt search UPWARD from the
/// invocation dir to the filesystem root for these, so an ANCESTOR copy of one
/// (above `--root`) is just as result-affecting as an in-root copy — but the
/// `--root`-relative tree glob ([`builtin_glob_set`]) never sees it. We mirror the
/// per-def split of the glob set so an ancestor change busts ONLY the gates that
/// actually read it (fmt does not read clippy/cargo config — capturing them would
/// needlessly bust a fmt HIT). These are SPECIFIC, ENUMERATED filenames — NOT an
/// arbitrary-file read — so this stays in the bounded toolchain-config class the
/// glob fix already closed in-root, not the disclosed unbounded P2 fixture seam.
///
/// `.cargo/config{,.toml}` is special: it lives in a `.cargo/` subdir of each
/// ancestor, so it is probed as the relative segment `.cargo/config.toml`. All
/// other names are probed directly in the ancestor dir.
fn ancestor_config_names(name: &str) -> &'static [&'static str] {
    match name {
        // `cargo fmt` reads the rustfmt config; the active toolchain pin selects
        // WHICH rustfmt runs. Both search upward.
        "fmt" => &[
            "rustfmt.toml",
            ".rustfmt.toml",
            "rust-toolchain.toml",
            "rust-toolchain",
        ],
        // `cargo clippy`/`cargo test` read the lint config, the cargo build config
        // (rustflags / lints / [build]), and the toolchain pin — all upward-searched.
        // Wave Q (5th/final bounded wedge stale-green): also add `Cargo.toml` and
        // `Cargo.lock` — cargo reads the WORKSPACE-root `Cargo.toml` (which can live
        // in an ANCESTOR dir above `--root`) for `[workspace.lints]`, `[profile]`,
        // `[patch]`, and the `Cargo.lock` for resolved deps — all result-affecting for
        // `clippy`/`test`. This completes the BOUNDED ancestor config set for these
        // two gates: {Cargo.toml, Cargo.lock, .cargo/config{,.toml}, rust-toolchain{,.toml},
        // clippy.toml/.clippy.toml}. The unbounded-fixture / outside-root-read residual
        // stays the disclosed P2 hermetic-execution seam — NOT closable by enumeration.
        "clippy" | "test" => &[
            ".cargo/config.toml",
            ".cargo/config",
            "clippy.toml",
            ".clippy.toml",
            "rust-toolchain.toml",
            "rust-toolchain",
            "Cargo.toml",
            "Cargo.lock",
        ],
        // An unknown (ad-hoc) name never reaches here (the caller takes the `**/*`
        // path and resolves no ancestor config).
        _ => &[],
    }
}

/// Compute the ANCESTOR toolchain-config digest folded into the memo key (Wave P
/// FIX A — the 4th wedge stale-green close; Wave Q extends clippy/test to also
/// capture ancestor `Cargo.toml`/`Cargo.lock` — the 5th and final bounded close).
///
/// cargo & rustfmt discover their config by walking UP from the invocation dir to
/// the filesystem root (plus `$CARGO_HOME/config.toml`). [`builtin_glob_set`]
/// captured the in-`--root` copies; this captures the ones in ANCESTOR directories
/// of `--root` (strictly above it — the in-root copies are the glob's job) plus
/// `$CARGO_HOME/config.toml`, so an ancestor-config change BUSTS the key.
///
/// Discovery replicates the tools' DETERMINISTIC search: from the canonical
/// `--root`, take each ancestor (parent, grandparent, … up to the filesystem
/// root), and at each one probe the per-def [`ancestor_config_names`]. The result
/// is a single SHA-256 over a canonical pre-image: for every (ancestor-depth,
/// relative-name) that EXISTS, frame `LP(label) ‖ LP(content)`, sorted by label,
/// count-prefixed. The label is `depth`-stable (the integer number of `..` hops
/// above `--root`) + the relative name, NOT an absolute path, so the digest is
/// invariant to where `--root` sits in the filesystem (two checkouts at different
/// absolute prefixes with the same ancestor config compute the SAME digest — the
/// hit-rate is preserved cross-machine). `$CARGO_HOME/config.toml` is labelled by
/// a fixed `cargo-home/config.toml` tag (its absolute location is captured in the
/// env axis via `CARGO_HOME`/`HOME`, so only its CONTENT needs folding here).
///
/// A def with no ancestor config names (ad-hoc, or none found) yields the empty
/// digest sentinel — folded identically, so "no ancestor config" is itself a
/// stable, distinct key input (adding the first ancestor config is a MISS).
///
/// This reads SPECIFIC BOUNDED KNOWN filenames outside `--root` — NOT arbitrary
/// files; the unbounded outside-root read stays the disclosed P2 hermetic seam.
fn ancestor_config_digest(name: &str, canonical_root: &Path) -> String {
    let names = ancestor_config_names(name);
    // (canonical label, content) for every existing probed file, deterministically
    // ordered by label below.
    let mut found: Vec<(String, Vec<u8>)> = Vec::new();

    if !names.is_empty() {
        // Walk strictly UPWARD: depth 1 = parent, depth 2 = grandparent, … The
        // in-`--root` copies (depth 0) are captured by the tree-axis glob, so we
        // start at the parent to avoid double-counting (harmless if we did, but
        // this keeps the two axes cleanly disjoint).
        let mut depth = 1usize;
        let mut cursor = canonical_root.parent();
        while let Some(dir) = cursor {
            for rel in names {
                let candidate = dir.join(rel);
                if let Some(bytes) = read_snapshot_content(&candidate) {
                    // Label by the ancestor DEPTH + the relative name — stable
                    // across absolute-prefix changes, so two checkouts in
                    // different locations with the same ancestor config key alike.
                    found.push((format!("anc{depth}/{rel}"), bytes));
                }
            }
            depth += 1;
            cursor = dir.parent();
        }
    }

    // `$CARGO_HOME/config.toml` (default `~/.cargo/config.toml`) is a further
    // ancestor-of-sorts cargo always consults for clippy/test. Its absolute path
    // is env-captured (CARGO_HOME/HOME), so only its CONTENT is folded, under a
    // fixed label. fmt does not consult it (no cargo config), so guard by def.
    if matches!(name, "clippy" | "test")
        && let Some(cargo_home) = cargo_home_dir()
    {
        let candidate = cargo_home.join("config.toml");
        if let Some(bytes) = read_snapshot_content(&candidate) {
            found.push(("cargo-home/config.toml".to_string(), bytes));
        }
        // cargo also accepts the extensionless `config` in CARGO_HOME.
        let candidate_noext = cargo_home.join("config");
        if let Some(bytes) = read_snapshot_content(&candidate_noext) {
            found.push(("cargo-home/config".to_string(), bytes));
        }
    }

    // Canonical pre-image: sorted by label, count-prefixed, each entry
    // `LP(label) ‖ LP(content)` — the same framing discipline as the tree axis, so
    // a changed/added/removed ancestor config all change the digest (a MISS).
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&(found.len() as u32).to_be_bytes());
    for (label, content) in &found {
        buf.extend_from_slice(&(label.len() as u32).to_be_bytes());
        buf.extend_from_slice(label.as_bytes());
        buf.extend_from_slice(&(content.len() as u32).to_be_bytes());
        buf.extend_from_slice(content);
    }
    hex::encode(Sha256::digest(&buf))
}

/// Resolve `$CARGO_HOME` (the dir holding cargo's global `config.toml`): the
/// `CARGO_HOME` env var if set, else `~/.cargo` derived from `HOME`. Both inputs
/// are themselves captured in the env axis, so this only locates the file whose
/// CONTENT [`ancestor_config_digest`] folds.
fn cargo_home_dir() -> Option<PathBuf> {
    if let Some(ch) = std::env::var_os("CARGO_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(ch));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".cargo"))
}

/// The fallback toolchain marker used ONLY when `--toolchain` is omitted AND the
/// active toolchain identity cannot be probed (e.g. `rustc` is not on PATH). It is
/// deliberately distinct from any real digest so a probe failure is visible rather
/// than colliding with a hashed identity.
const TOOLCHAIN_PROBE_UNAVAILABLE: &str = "toolchain-unprobed";

// ── hermetic execution env (Round-8 C3) ────────────────────────────────────────
//
// The memo wedge memoizes a NON-HERMETIC `sh -c` — a process free to read cwd,
// the whole environment, PATH, and the entire filesystem. Trying to ENUMERATE the
// inputs of such a process is impossible (the input set is open), so an uncaptured
// input (cwd, an unlisted env var, PATH) produced a STALE GREEN: a warm HIT served
// `exit:0` where a real run would FAIL. The class-killing fix is to make the spawn
// HERMETIC for the LOCAL scope so the captured axes ARE the complete input set by
// construction:
//   1. cwd is PINNED to the canonical `--root` (the tree-axis root).
//   2. the env is CLEARED then reconstructed from ONLY the captured allowlist, so
//      a var the check reads is either IN the key (captured) or ABSENT (reads
//      empty, deterministically) — never an ambient leak.
//   3. PATH is pinned to the resolved ambient PATH and FOLDED (hashed) into the
//      env-manifest axis, so a PATH change busts the memo key.
// The captured set is folded into `def.env_manifest` → `compute_def_digest` → the
// memo key, so any change to a captured var/PATH is a MISS by construction.
//
// OUT OF LOCAL SCOPE (disclosed P2 runner-sandbox seam — NOT closed here):
// whole-filesystem confinement (files outside `--root`, pruned dirs), network, and
// clock. Those require the runner-side isolated rootfs (the runner fabric) where the
// action physically cannot read outside the seeded tree axis. The local executor
// mirrors that contract's SOUNDNESS for cwd/env/PATH; it does not exceed it.

/// Exact env-var NAMES that can change a Rust gate's RESULT and are therefore
/// captured into the memo key (folded into `env_manifest`) AND set on the hermetic
/// spawn. An ambient var NOT on this list (and not matching a captured prefix) is
/// CLEARED before the spawn, so it can never silently change a check's outcome
/// off-key. Sorted lookups keep the manifest deterministic.
const RESULT_AFFECTING_ENV_EXACT: &[&str] = &[
    // `HOME` is captured (not cleared): the toolchain derives `CARGO_HOME`/
    // `RUSTUP_HOME` defaults (`~/.cargo`, `~/.rustup`) from it, so clearing it
    // would break the built-in `cargo` gates. Pinned into the key (its value is a
    // captured axis), so a HOME change is a MISS — present AND in-key.
    "HOME",
    "CC",
    "CXX",
    "AR",
    // Native-build flags (merged from K-RUN's allowlist): result-affecting for any
    // `cc`/`c++`/link step a check shells out to. Captured (not cleared) so a flag
    // change busts the memo key AND native (cc-rs) gates still build.
    "CFLAGS",
    "CXXFLAGS",
    "LDFLAGS",
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_BOOTSTRAP",
    "SOURCE_DATE_EPOCH",
    "LANG",
    "LC_ALL",
    "TZ",
];

/// Env-var NAME PREFIXES that can change a Rust gate's result (the `CARGO_*`,
/// `RUST_*`, `CARGO_BUILD_*`, `RUSTUP_*`, `LC_*` families). A var whose name starts
/// with any of these is captured + set; everything else is cleared.
const RESULT_AFFECTING_ENV_PREFIXES: &[&str] =
    &["CARGO_", "CARGO_BUILD_", "RUST_", "RUSTUP_", "LC_"];

/// Whether an env-var name is in the result-affecting allowlist (exact OR prefix).
/// `PATH` is handled separately (pinned + hashed into the axis), so it is NOT in
/// this predicate.
fn is_result_affecting_env(name: &str) -> bool {
    RESULT_AFFECTING_ENV_EXACT.contains(&name)
        || RESULT_AFFECTING_ENV_PREFIXES
            .iter()
            .any(|p| name.starts_with(p))
}

/// The captured, hermetic env for a spawn: every ambient var whose name is in the
/// result-affecting allowlist, PLUS `PATH` (pinned), PLUS any caller-declared
/// `--env-axis` var (`declared`, PS-11), sorted by name for determinism. This is
/// the EXACT set that is (a) folded into the memo-key axis via [`env_manifest_axis`]
/// and (b) set on the spawned `Command` after `env_clear()`. Because (a) and (b)
/// come from the same source, "captured == present": the memo key's env axis is the
/// complete spawn env, so a declared var is keyed AND passed (never a stale green).
fn captured_hermetic_env(declared: &[String]) -> Vec<(String, String)> {
    captured_hermetic_env_from(std::env::vars(), declared)
}

/// Pure core of [`captured_hermetic_env`] over an explicit `(name, value)` source,
/// so the capture rule is unit-testable without mutating the process-global env (a
/// cross-test data race — sweep T-3). A var is captured iff it is `PATH`, on the
/// result-affecting allowlist, OR caller-declared via `--env-axis`. A declared var
/// that is NOT present in `vars` simply does not appear (so toggling it on later is
/// a MISS — adding a captured pair changes the manifest).
fn captured_hermetic_env_from<I>(vars: I, declared: &[String]) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut pairs: Vec<(String, String)> = vars
        .into_iter()
        .filter(|(k, _)| {
            k == "PATH" || is_result_affecting_env(k) || declared.iter().any(|d| d == k)
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

/// Serialize the captured hermetic env into the deterministic `env_manifest` axis
/// string folded into `compute_def_digest`. PATH's VALUE is hashed (it is long and
/// machine-specific, but a change must bust the key — F-MK5), so the manifest
/// carries `PATH=<sha256(value)>`; every other captured var carries its literal
/// `NAME=VALUE`. Newline-joined over the sorted pairs so the axis is canonical: a
/// changed value, a changed PATH, an added or removed captured var all change the
/// axis → a different memo key → a MISS (never a stale green).
fn env_manifest_axis(captured: &[(String, String)]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(captured.len());
    for (k, v) in captured {
        if k == "PATH" {
            let digest = hex::encode(Sha256::digest(v.as_bytes()));
            lines.push(format!("PATH=sha256:{digest}"));
        } else {
            lines.push(format!("{k}={v}"));
        }
    }
    lines.join("\n")
}

/// Resolve the toolchain digest (third memo axis) for this run.
///
/// An explicit `--toolchain` wins verbatim (the caller content-addressed it). When
/// omitted, we compute a REAL digest of the ACTIVE toolchain identity — the
/// SHA-256 of `rustc --version --verbose` output — so changing the compiler busts
/// the memo key. (WG-CACHE: the prior `local-toolchain` CONSTANT made axis 3 fake,
/// so a green cached under Rust A wrongly HIT under Rust B.) If `rustc` cannot be
/// run, fall back to a distinct marker rather than fabricating a digest.
fn resolve_toolchain_digest(args: &CheckRunArgs) -> String {
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
///
/// `env_manifest` carries the captured hermetic env axis (Round-8 C3): the set of
/// result-affecting env vars + the hashed PATH that the hermetic spawn will set, so
/// a change to any captured var/PATH busts the memo key. It MUST be set before
/// validation (the digest folds it).
///
/// `ancestor_config_digest` (Wave P FIX A) is a single SHA-256 over the ANCESTOR
/// toolchain-config files the built-in gate reads from ABOVE `--root` (cargo &
/// rustfmt search upward). It is folded into `inputs` (a `def_digest` axis), so an
/// ancestor `.cargo/config.toml`/`rustfmt.toml`/`rust-toolchain*` change BUSTS the
/// memo key — closing the 4th wedge stale-green. For a built-in def it is the
/// `ancestor:<digest>` line; for an ad-hoc def it is empty (the `**/*` glob already
/// makes any in-tree edit a MISS and the ancestor-config class is built-in-only).
fn resolve_def(
    args: &CheckRunArgs,
    env_manifest: String,
    ancestor_config_digest: String,
) -> Result<CheckDef, PorcelainError> {
    let (command, glob_set) = match builtin_command(&args.def) {
        Some(cmd) => (cmd.to_string(), builtin_glob_set(&args.def)),
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

    // FIX A: fold the ANCESTOR toolchain-config digest into `inputs` (a
    // `def_digest` axis via `compute_def_digest`'s `push_vec`). A non-empty digest
    // means a built-in gate's effective ancestor config (cargo/rustfmt/toolchain
    // config ABOVE `--root`, plus `$CARGO_HOME/config.toml`) is captured, so a
    // change to it busts the memo key. The `ancestor:` label keeps it a discrete,
    // self-describing entry that can never collide with a future declared input.
    let inputs = if ancestor_config_digest.is_empty() {
        Vec::new()
    } else {
        vec![format!("ancestor:{ancestor_config_digest}")]
    };

    let def = CheckDef {
        def_digest: String::new(),
        command,
        inputs,
        // The toolchain axis is a REAL digest when `--toolchain` is omitted — the
        // hash of the active `rustc --version --verbose` — so a toolchain change
        // busts the memo key (WG-CACHE: the old `local-toolchain` constant made
        // axis 3 fake, hitting a green cached under Rust A under Rust B).
        toolchain_ref: resolve_toolchain_digest(args),
        // The hermetic env axis (Round-8 C3, built on K-RUN's allowlist): a
        // canonical sorted manifest of the result-affecting env vars PLUS the
        // hashed PATH the spawn will set. The check command runs through `sh -c`,
        // so a result-affecting env var (e.g. RUSTFLAGS) or a PATH change alters
        // the gate's outcome; folding them into this axis (→ def_digest → memo_key
        // via the validator below) makes any such change a MISS, while an UNLISTED
        // var leaves the hit-rate intact. Supersedes K-RUN's inline
        // `result_affecting_env_manifest()` — the value now also pins PATH so a
        // stale GREEN can no longer be served when the resolved binary changes.
        env_manifest,
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

// NOTE (Wave-L integration): K-RUN's `result_affecting_env_manifest()` and its
// duplicate `RESULT_AFFECTING_ENV_EXACT`/`RESULT_AFFECTING_ENV_PREFIXES` consts
// were removed here — superseded by L-C's hermetic env path above
// (`captured_hermetic_env` + `env_manifest_axis`), which sets the spawn env
// hermetically (`env_clear` + the captured allowlist) AND folds that SAME
// captured set + the hashed PATH into the memo-key axis, so "captured == present"
// by construction. The native-build vars K-RUN covered (CFLAGS/CXXFLAGS/LDFLAGS)
// were merged into the single `RESULT_AFFECTING_ENV_EXACT` allowlist above.

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

/// The result-affecting POSIX mode bits of `path` folded into the tree axis (N-1,
/// narrowed by Wave P FIX B to the EXECUTABLE bits only).
///
/// On unix we read `st_mode` and keep ONLY the executable bits `mode & 0o111`
/// (owner/group/other exec). Exec-vs-not is the result-affecting part — the
/// failure the N-1 fold closed needed only the exec bit (`./gate.sh` flips from
/// `exit 0` to `exit 126` on `chmod -x`, SAME content). The OTHER permission bits
/// (the read/write bits, e.g. group-write `0664` vs `0644`) are a function of the
/// runner's UMASK, which git does NOT track and which does not change a gate's
/// outcome; folding the full `0o7777` leaked the umask into the memo key, so two
/// runners with different umasks computed DIFFERENT keys for byte-identical,
/// same-exec-bit sources → a cross-runner cache MISS on the fleet-shared AC for
/// identical work (Round 11 N-1 over-capture: correctness-SAFE, a hit-rate loss).
/// Folding only `0o111` keeps `chmod -x` a MISS (the N-1 P0 stays closed) while
/// making the key umask-invariant. A stat failure folds a fixed sentinel
/// (`MODE_UNREAD`) rather than guessing — deterministic, never a silent off-key
/// input.
///
/// On non-unix the mode bits are not meaningful (Windows has no `st_mode`
/// exec bit), so we fold a fixed sentinel: the framing stays stable and
/// cross-platform, and the Windows build is unaffected.
#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    /// Sentinel when the mode cannot be stat'd — distinct from any real
    /// `mode & 0o111` value.
    const MODE_UNREAD: u32 = u32::MAX;
    match std::fs::metadata(path) {
        // FIX B: fold ONLY the executable bits — exec-vs-not is the
        // result-affecting axis; the umask-dependent read/write bits are not.
        Ok(meta) => meta.mode() & 0o111,
        Err(_) => MODE_UNREAD,
    }
}

/// Non-unix fold: a fixed sentinel (mode bits are not meaningful on Windows), so
/// the canonical framing is stable cross-platform without breaking the build.
#[cfg(not(unix))]
fn file_mode(_path: &Path) -> u32 {
    /// Fixed cross-platform sentinel folded on non-unix (no POSIX mode there).
    const MODE_NON_UNIX: u32 = 0;
    MODE_NON_UNIX
}

/// Peak-memory ceiling (bytes) for reading a single snapshotted file WHOLE into
/// the memo key (PS-17, defensive). A file at or under the cap is folded as its
/// raw bytes — exactly as before, so every realistic source/config input
/// (Cargo.toml/rustfmt.toml are KB; source files MB) keeps its folded
/// representation byte-for-byte and the memo key + hit-rate are UNCHANGED. Only a
/// pathological file ABOVE the cap (e.g. a 200 MB adversarial ancestor config →
/// ~400 MB peak under the old whole-read) is folded differently — and even then
/// soundly (below).
///
/// 64 MiB is far above any legitimate check input while bounding the peak.
const MAX_SNAPSHOT_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Read a file's content for snapshotting into the memo key with a bounded peak
/// memory (PS-17). Delegates to [`read_snapshot_content_capped`] with the
/// production cap; the inner form takes the cap as a parameter so the soundness
/// invariants can be exercised by a unit test without materializing a >64 MiB
/// fixture.
fn read_snapshot_content(path: &Path) -> Option<Vec<u8>> {
    read_snapshot_content_capped(path, MAX_SNAPSHOT_FILE_BYTES)
}

/// The capped read. A file `<= cap` bytes is returned as its raw bytes (folded
/// IDENTICALLY to the old bare `fs::read`, so the key is unchanged for every
/// real-repo file). A file `> cap` is folded as a deterministic
/// `OVERSIZE:<len>:<streamed-sha256>` sentinel computed WITHOUT holding the whole
/// file in memory (a 1 MiB streaming buffer) — soundness is preserved (any change
/// to the oversized file changes its length or hash → the sentinel changes → a
/// MISS that re-executes), so the snapshot never serves a stale green off an
/// oversized input, while peak memory stays bounded. Any I/O fault returns `None`,
/// which the callers treat as "file absent" — the same fail-safe the bare
/// `fs::read(..).ok()` had (a fault never crashes and never silently fabricates a
/// key input).
fn read_snapshot_content_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() <= cap {
        return std::fs::read(path).ok();
    }
    // Oversized: stream-hash so the peak stays at one chunk, not the whole file.
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Some(format!("OVERSIZE:{total}:{}", hex::encode(hasher.finalize())).into_bytes())
}

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
                && let Some(bytes) = read_snapshot_content(&path)
            {
                // Fold the POSIX mode into the snapshotted byte field so a mode
                // change (e.g. `chmod -x gate.sh`, SAME content) busts the tree
                // axis → a MISS that RE-EXECUTES (N-1 stale-green close). The
                // mode is a result-affecting input (an unexecutable `./gate.sh`
                // fails `exit 126`), so content alone is an incomplete key. The
                // canonical framing lives ONCE in `frame_file_with_mode`.
                let mode = file_mode(&path);
                out.insert(rel, frame_file_with_mode(mode, &bytes));
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
    /// The canonical `--root` the check's cwd is PINNED to (Round-8 C3 hermetic
    /// exec). The spawn's cwd == the tree-axis root, so a relative-path check reads
    /// the SAME files the tree axis hashed — never a different file per ambient
    /// cwd. `None` keeps the legacy (inherited-cwd) behaviour for the in-process
    /// unit tests that do not exercise cwd sensitivity.
    root: Option<PathBuf>,
    /// The captured hermetic env: the EXACT `(name, value)` set folded into the
    /// memo key's env axis (`env_manifest`). The spawn is `env_clear()`ed then
    /// reconstructed from ONLY these, so a var the check reads is either captured
    /// (in the key) or absent (reads empty) — never an off-key ambient leak.
    /// `None` keeps the legacy inherited-env behaviour for the unit tests.
    env: Option<Vec<(String, String)>>,
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
        // the deadline. stdout/stderr are piped and CONCURRENTLY DRAINED by two
        // background threads (WJ-CHECK-DRAIN): a command emitting more than the
        // 64 KiB OS pipe-buffer would otherwise block on its next write(), causing
        // `try_wait()` to return `Ok(None)` forever until the 300 s timeout. The
        // drain threads read each pipe to completion (capped at PIPE_CAPTURE_CAP
        // per stream to prevent OOM from a `yes`/`cat /dev/urandom` flood), so the
        // child is never blocked on a full pipe and the timeout fires only for a
        // genuinely hung (non-writing) command. The captured bytes are discarded —
        // a check's output never pollutes the verb's JSON-contract stdout.
        //
        // The child is spawned in its OWN process group (`process_group(0)` —
        // std-only, Unix) so the whole tree (including a backgrounded grandchild)
        // can be killed as a unit on a timeout (WH-CHECK lock-poison fix item b:
        // `child.kill()` alone reaps only the direct child, letting an orphan
        // grandchild survive past the deadline and a backgrounding command bypass
        // the wall-time ceiling).
        let mut command = shell_command(&def.command);
        command
            // HERMETIC stdin (Round-8 C3 / Round-9 close): stdin is NULL, never
            // inherited. An inherited stdin is an uncaptured, result-affecting input
            // — a check that reads it (`read x; …`) would otherwise produce a
            // STALE GREEN (a warm HIT served when the ambient stdin flips), since
            // stdin is not in the memo key. Nulling it makes a stdin read a
            // deterministic EOF, so stdin can never change a check's outcome off-key.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // HERMETIC EXECUTION (Round-8 C3): pin cwd + clear/reconstruct env so the
        // captured memo axes ARE the complete input set for the local scope.
        //
        // (1) Pin cwd to the canonical `--root`. The tree axis snapshots files
        //     under `--root`; pinning the spawn's cwd there means a check using
        //     RELATIVE paths reads exactly those files — never a different file per
        //     ambient cwd (closes F-MK1). `--root` is canonicalized by the caller.
        if let Some(root) = &self.root {
            command.current_dir(root);
        }
        // (2) Clear the inherited environment, then set ONLY the captured allowlist
        //     vars + the pinned PATH. After this, any var the check reads is either
        //     IN the memo key (captured into `env_manifest`) or ABSENT (reads
        //     empty, deterministically) — never a silent off-key ambient input
        //     (closes F-MK2 for unlisted vars and F-MK5 for PATH, whose value is
        //     also hashed into the env axis so a PATH change busts the key).
        if let Some(env) = &self.env {
            command.env_clear();
            for (k, v) in env {
                command.env(k, v);
            }
        }
        // NOTE: the child is intentionally NOT placed in its own process group /
        // session. The timeout path (`kill_group`) signals only single, POSITIVE
        // PIDs — the direct child plus (on linux) its descendants enumerated from
        // `/proc` — never a process group, because a process-group signal is unsafe
        // on a GitHub-hosted linux runner / the linux engine container (it took
        // down the whole CI job, even with `setsid` isolation). See `kill_group`.
        let mut child = command
            .spawn()
            .map_err(|e| ExecError::Run(format!("spawn `{}`: {e}", def.command)))?;

        // Take the pipe handles BEFORE the poll loop so the drain threads can
        // consume them independently. Taking them here (before any try_wait) is
        // required: once `child.wait()` is called the handles are consumed.
        //
        // Each drain thread reads up to PIPE_CAPTURE_CAP bytes then discards the
        // rest (still consuming, so the child is never blocked). On a normal
        // command the thread reads until EOF (pipe closed when the child exits).
        // On a flooding command it reads until the cap, then discards the rest via
        // a small fixed-size scratch buffer until EOF — the child is STILL
        // unblocked. On TIMEOUT we kill only the direct child, so a shell-forked
        // grandchild may keep the pipe open; the drain-thread join is SKIPPED on the
        // timeout path (the threads detach), so a lingering pipe never delays return.
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        let drain = |mut pipe: Box<dyn std::io::Read + Send + 'static>| {
            std::thread::spawn(move || {
                let mut buf = Vec::with_capacity(PIPE_CAPTURE_CAP.min(64 * 1024));
                let mut total = 0usize;
                let mut scratch = [0u8; 4096];
                loop {
                    if total < PIPE_CAPTURE_CAP {
                        let room = PIPE_CAPTURE_CAP - total;
                        let chunk = room.min(scratch.len());
                        match pipe.read(&mut scratch[..chunk]) {
                            Ok(0) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&scratch[..n]);
                                total += n;
                            }
                            Err(_) => break,
                        }
                    } else {
                        // Cap reached: drain and discard to keep the pipe unblocked.
                        match pipe.read(&mut scratch) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                }
                // The captured bytes are intentionally discarded (not returned).
                // A check's output never surfaces in hugit's JSON-contract stdout.
                drop(buf);
            })
        };

        let stdout_thread = stdout_handle.map(|h| drain(Box::new(h)));
        let stderr_thread = stderr_handle.map(|h| drain(Box::new(h)));

        // Poll for completion up to the deadline; kill + reap the direct child on
        // expiry (WH-CHECK: the check must not outlive the ceiling). The drain
        // threads run concurrently throughout: the child can never block on a full
        // pipe regardless of how much output it emits (WJ-CHECK-DRAIN).
        let poll_result = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.code().unwrap_or(-1)),
                Ok(None) => {
                    if start.elapsed() >= self.timeout {
                        // Bounded ceiling reached: SIGKILL the direct child (a single
                        // PID — never its process group, which is unsafe on a linux
                        // runner/the engine; see `kill_group`) and reap it (no
                        // zombie), then surface a structured timeout. The result is
                        // NOT stored — a hang never poisons the cache. On linux
                        // `kill_group` ALSO reaps any shell-forked/backgrounded
                        // grandchild by single PID (enumerated from `/proc` before the
                        // child dies), so a `foo &` cannot outlive the deadline; the
                        // drain-thread join below is still SKIPPED on this path so it
                        // can never block on a not-yet-reaped pipe holder.
                        kill_group(&mut child);
                        break Err(ExecError::Timeout(self.timeout.as_secs()));
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    kill_group(&mut child);
                    break Err(ExecError::Run(format!("wait `{}`: {e}", def.command)));
                }
            }
        };

        // Join the drain threads ONLY when the child exited on its own — then the
        // pipes are already at EOF and the join is immediate. On a TIMEOUT/error
        // `kill_group` SIGKILLs the direct child + (on linux) its enumerated
        // descendants by single PID — never a process group (that is unsafe on a
        // linux runner/the engine; see `kill_group`). Reaping is best-effort/async
        // w.r.t. the OS delivering SIGKILL, so a grandchild (linux `sh -c` forks
        // `sleep`; macOS execs it) MIGHT still hold the pipe write-end open for a
        // beat. Joining there could block until it exits, defeating the timeout's
        // promptness. The captured bytes are discarded regardless, so on
        // timeout/error we DETACH the drain threads (drop the handles; they exit
        // when the pipe holder or our process does) and return promptly. They still
        // drained the pipe DURING the run (the no-block-on-full-pipe guarantee is
        // the loop above, not this join).
        if poll_result.is_ok() {
            if let Some(t) = stdout_thread {
                let _ = t.join();
            }
            if let Some(t) = stderr_thread {
                let _ = t.join();
            }
        }

        let exit = poll_result?;
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

/// Kill the timed-out child, reap it (no zombie), then PROMPTLY reap any
/// backgrounded descendant by SINGLE PID (Task #36 — orphan reaping restored
/// WITHOUT a process-group signal).
///
/// SAFETY (the load-bearing invariant): a check timeout must NEVER be able to
/// signal anything beyond its own command's subtree — not `hugit check` itself,
/// not the CI job, and (in prod) NOT the linux engine container. We therefore
/// NEVER send a process-group (negative-pid) signal: a group kill took down the
/// WHOLE CI job on a GitHub-hosted linux runner even with the child in its own
/// `setsid` session (POSIX says that should isolate it; the hosted runner's
/// process model cancels the job regardless), and the engine is linux too. The
/// signal surface is bounded to single POSITIVE pids by construction, so a group
/// signal is structurally impossible.
///
/// The sequence (order matters — see the PID-reuse guard):
///   1. (linux) Enumerate the child's transitive descendants from `/proc` while
///      the tree is intact.
///   2. (linux) SIGKILL each enumerated descendant by its single positive PID,
///      BUT re-verify — immediately before signalling — that the PID's `/proc`
///      ancestry STILL terminates at the direct child. This is done BEFORE
///      reaping the child (step 3) so the subtree linkage is still live for the
///      re-verification. (PID-reuse guard: between enumeration and signalling a
///      descendant can exit and its PID be recycled by an unrelated process; the
///      ancestry re-check ensures we only ever SIGKILL a PID still inside our own
///      subtree, never a stranger — the load-bearing safety invariant.)
///   3. SIGKILL the direct child by its single PID and reap it (`child.wait()`).
///      The child is an `sh -c <cmd>`; on linux `sh` may FORK the command (and a
///      `foo &` grandchild), on macOS it EXECs it.
///
/// The OS still reaps any straggler on hugit's exit; the descendant sweep just
/// makes it PROMPT so a backgrounded grandchild cannot outlive the deadline.
/// Best-effort: a failed signal is ignored (never a crash). The sweep is
/// linux-only: macOS `sh -c` execs the command (no forked grandchild to reap) and
/// has no `/proc`. The direct-child kill suffices there.
fn kill_group(child: &mut std::process::Child) {
    // (1)+(2) Enumerate + reap descendants WHILE the child is still alive, so the
    // ancestry re-check in `reap_pids` can confirm each PID is still in our subtree
    // (PID-reuse guard). Reaping before the child also catches a direct child of
    // `sh` that is NOT `sh` itself (which `child.kill()` alone would orphan).
    #[cfg(target_os = "linux")]
    {
        let root = child.id();
        let descendants = linux_descendant_pids(root);
        reap_pids(&descendants, root);
    }

    // (3) SIGKILL + reap the direct child (single PID — never a process group).
    let _ = child.kill();
    let _ = child.wait();
}

/// Enumerate the transitive descendant PIDs of `root` from `/proc` (linux).
///
/// Builds the PID→PPid map by reading the PPid field of every `/proc/<pid>/stat`,
/// then BFS-collects every PID transitively parented by `root` (excluding `root`
/// itself — the caller kills it directly). The `comm` field (the 2nd stat field)
/// is wrapped in parentheses and may itself contain spaces or `)`, so we split
/// AFTER the LAST `)` to read `state` then `ppid` — the canonical robust
/// `/proc/<pid>/stat` parse. Any unreadable/short-lived entry is skipped
/// (best-effort: a missed descendant is OS-reaped on hugit's exit, never a crash).
#[cfg(target_os = "linux")]
fn linux_descendant_pids(root: u32) -> Vec<u32> {
    // pid -> ppid for every process currently visible in /proc.
    let mut parent_of: BTreeMap<u32, u32> = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        // Only numeric `/proc/<pid>` entries are processes (skip `self`, `cpuinfo`…).
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue; // process exited between read_dir and open — skip.
        };
        // `<pid> (comm) <state> <ppid> …` — comm can contain spaces and ')', so
        // parse the fixed fields AFTER the last ')'.
        let Some(after) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
            continue;
        };
        let mut fields = after.split_whitespace();
        let _state = fields.next();
        if let Some(ppid) = fields.next().and_then(|p| p.parse::<u32>().ok()) {
            parent_of.insert(pid, ppid);
        }
    }

    // BFS from `root` over the parent map: collect every transitive descendant.
    let mut descendants: Vec<u32> = Vec::new();
    let mut frontier = vec![root];
    while let Some(cur) = frontier.pop() {
        for (&pid, &ppid) in &parent_of {
            if ppid == cur && pid != root && !descendants.contains(&pid) {
                descendants.push(pid);
                frontier.push(pid);
            }
        }
    }
    descendants
}

/// SIGKILL each PID by its single positive value (`kill -KILL <pid>…`, linux),
/// AFTER re-verifying its `/proc` ancestry still terminates at `root`.
///
/// Positive pids ONLY — a process-group (negative-pid) signal is structurally
/// impossible here, preserving the load-bearing safety invariant in [`kill_group`].
///
/// PID-reuse guard: a PID enumerated earlier may have exited and been recycled by
/// an UNRELATED process before we signal it; SIGKILLing it blind would violate the
/// "never signal beyond our own subtree" invariant. So each candidate is
/// re-checked with [`is_descendant_of`] (a fresh upward PPid walk) immediately
/// before signalling, and only those still rooted at `root` are killed. The caller
/// runs this WHILE `root` is still alive, so the linkage is live for the check.
/// There remains a microscopic TOCTOU between the check and the `kill` subprocess,
/// but the window is now check-then-immediately-kill (was: kill child, then a much
/// later sweep), and the OS-reaps-on-exit backstop covers any straggler.
///
/// One subprocess for the whole batch; best-effort (a failure to signal is ignored).
/// Using the `kill` binary (already the cleanup primitive in the acceptance tests)
/// avoids a new `libc` direct dependency + its `unsafe` and the X4 exact-pin obligation.
#[cfg(target_os = "linux")]
fn reap_pids(pids: &[u32], root: u32) {
    let live: Vec<String> = pids
        .iter()
        .filter(|&&pid| is_descendant_of(pid, root))
        .map(|pid| pid.to_string())
        .collect();
    if live.is_empty() {
        return;
    }
    let mut cmd = Command::new("kill");
    cmd.arg("-KILL");
    for pid in &live {
        cmd.arg(pid);
    }
    let _ = cmd.status();
}

/// Read the PPid (parent PID) of `pid` from `/proc/<pid>/stat` (linux). Returns
/// `None` if the process is gone / unreadable. The `comm` field is parenthesised
/// and may contain spaces or `)`, so we parse the fixed fields AFTER the last `)`.
#[cfg(target_os = "linux")]
fn ppid_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.rfind(')').map(|i| &stat[i + 1..])?;
    let mut fields = after.split_whitespace();
    let _state = fields.next();
    fields.next().and_then(|p| p.parse::<u32>().ok())
}

/// Whether `pid`'s current `/proc` ancestry chain terminates at `root` (linux) —
/// i.e. `pid` is STILL a transitive descendant of `root` right now. Walks PPid
/// upward, bounded (a cap guards against a cycle/races), stopping at `root` (true),
/// PID 0/1/unreadable (false). Re-reads `/proc` live, so a recycled PID whose new
/// parent is outside our subtree returns false (the PID-reuse guard).
#[cfg(target_os = "linux")]
fn is_descendant_of(mut pid: u32, root: u32) -> bool {
    if pid == root {
        return false; // root itself is killed directly, never via the sweep.
    }
    for _ in 0..1024 {
        match ppid_of(pid) {
            Some(ppid) if ppid == root => return true,
            Some(ppid) if ppid > 1 => pid = ppid,
            _ => return false, // reached init/kernel/unreadable without hitting root.
        }
    }
    false
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
    /// Live CoreLink AC over HTTP — the hot, shared, content-addressed cache.
    /// Selected only when the CoreLink runtime config is fully present (explicit
    /// opt-in via `HUGIT_CORELINK_AC_URL` + `HUGIT_CORELINK_TENANT` + the PAT
    /// file); never a silent network call on an unconfigured box.
    Live(HttpAcClient),
}

impl ActionCache for AcBackend {
    fn lookup(&self, key: &str) -> Result<Option<CheckResult>, hugit_checks::client::ac::AcError> {
        match self {
            AcBackend::Local(ac) => ac.lookup(key),
            AcBackend::Live(ac) => ac.lookup(key),
        }
    }
    fn store(&self, result: &CheckResult) -> Result<(), hugit_checks::client::ac::AcError> {
        match self {
            AcBackend::Local(ac) => ac.store(result),
            AcBackend::Live(ac) => ac.store(result),
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
fn select_ac(args: &CheckRunArgs) -> Result<AcBackend, PorcelainError> {
    // Prefer the live CoreLink AC when its runtime config is fully present
    // (HUGIT_CORELINK_AC_URL + HUGIT_CORELINK_TENANT + the PAT file). This is an
    // explicit opt-in — `from_runtime()` returns NotConfigured on an unset box, in
    // which case we fall back silently to the file-backed local AC (so the wedge
    // still works without P2 and the verb never makes a network call unconfigured).
    // An explicit `--ac <path>` forces the local file AC (operator override).
    if args.ac.is_none()
        && let Ok(live) = HttpAcClient::from_runtime()
    {
        return Ok(AcBackend::Live(live));
    }
    let store = args
        .ac
        .clone()
        .unwrap_or_else(|| with_extension(&args.log_path(), "ac"));
    Ok(AcBackend::Local(FileAc::new(store)))
}

/// The default file-backed Action-Cache path for a `--log`: `<log>.ac`. The ONE
/// place the local AC path is derived, shared by `check run` and `land --queue`
/// so both warm the SAME on-disk wedge state (a `check run --store` primes a hit
/// the batch land reads, and vice versa).
pub fn default_ac_path(log_path: &Path) -> PathBuf {
    with_extension(log_path, "ac")
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
pub struct FileAc {
    path: PathBuf,
}

impl FileAc {
    /// Construct a file-backed AC over `path`. No lock is held by the backend
    /// itself — each `lookup`/`store` op takes the lock only for its own short
    /// critical section (the lock-poison fix), so the execute between them runs
    /// UNLOCKED.
    pub fn new(path: PathBuf) -> Self {
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
        // WRITE-BOUNDARY GUARD (WK-AC + PS-10): before any bytes touch the `.ac`
        // file, REFUSE to persist a CheckResult whose memo axes carry a
        // structural-secret shape. An axis is a content-address the cache keys on
        // — it CANNOT be scrubbed (that would bust `verify_hit`), so the only safe
        // action is to refuse the write entirely. The DOOR (`validate_axis` in
        // `run`) is the primary line; this is the belt-and-suspenders. PS-10
        // moved the guard onto the SHARED `hugit_checks` layer so EVERY backend
        // (file/in-memory/HTTP) enforces it identically — the deny-by-default
        // `is_safe_identifier_shape` predicate is byte-equivalent to the prior
        // `structural_secret_scrub(v) != v` (a value is a secret iff it is not a
        // provable safe-address shape), so no behavior changed here.
        hugit_checks::client::ac::guard_axes_not_secret(result)?;
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
    Err(AcError::Busy {
        detail: format!(
            "the AC store {} stayed locked by another hugit check",
            path.display()
        ),
    })
}

/// Reject a memo-axis flag value (`--def`/`--toolchain`) that carries a
/// structural-secret shape (WK-AC door). Reuses the ONE shared detector
/// ([`crate::ident::validate_identifier`] → `structural_secret_scrub`) the
/// identifier verbs use, so the door can never drift weaker than the engine.
/// An axis is an ADDRESS the cache keys on — it is stored unredacted (scrubbing
/// it would bust every hit), so a credential-shaped value is refused at input
/// with the structured `secret_in_identifier`/exit-2 error rather than persisted.
fn validate_axis(value: &str, field_name: &str) -> Result<(), PorcelainError> {
    crate::ident::validate_identifier(value, field_name)
        .map_err(|e| PorcelainError::new(e.kind, e.message, e.fix))
}

/// `hugit check` — resolve → memoize → execute-on-miss → (with `--store`) record.
///
/// The full wedge in one verb: derive the def + tree snapshot, run it through
/// [`run_memoized`] (HIT ⇒ 0 execution / `duration_ms:0`; MISS ⇒ execute once +
/// store), then — when `--store` is set — append a `check.recorded` event to the
/// canonical log through the guarded, lock-serialized, atomic seam. Returns the
/// stable-JSON outcome (the recorded row + the cache verdict) for the agent.
pub fn run(args: &CheckRunArgs) -> Result<Value, PorcelainError> {
    // DOOR (WK-AC, primary): `--toolchain` and `--def` are memo AXES, not free
    // text — a structurally-secret value cannot be SCRUBBED at rest (it is the
    // content-address `verify_hit` recomputes the memo key from; scrubbing it
    // would destroy every cache hit). So we REJECT a secret-shaped axis at the
    // door with the SAME structured exit-2 `secret_in_identifier` error the
    // identifier verbs emit, reusing the ONE shared detector
    // (`crate::ident::validate_identifier` → `structural_secret_scrub`) so the
    // door can never drift weaker than the engine ("an exemption is a hole").
    //
    // `--cmd` is a SHELL COMMAND (free text — may legitimately reference a
    // token); it is NOT validated here — it is scrubbed at rest on the `--log`.
    // `--toolchain` is OPTIONAL: only validate when explicitly given + non-empty
    // (omitted ⇒ the real active-toolchain digest, a computed hash, never a
    // secret). A legit toolchain digest (hex sha256, `rustc 1.96.0 (hash)`)
    // passes because the structural detector is bare-hex/entropy-EXEMPT and only
    // trips on credential PREFIXES.
    validate_axis(&args.def, "--def")?;
    if let Some(tc) = args.toolchain.as_deref().filter(|t| !t.trim().is_empty()) {
        validate_axis(tc, "--toolchain")?;
    }

    // Capture the hermetic env ONCE (Round-8 C3): the result-affecting allowlist
    // vars + PATH the spawn will set. Fold it into `env_manifest` so a change to any
    // captured var (or PATH, whose value is hashed) busts the memo key — the same
    // set is set on the spawn below, so "captured == present".
    let captured_env = captured_hermetic_env(&args.env_axis);
    let env_manifest = env_manifest_axis(&captured_env);

    let root = args
        .root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    // The canonical `--root` the hermetic spawn pins its cwd to (Round-8 C3) AND
    // the anchor the FIX A ancestor-config walk climbs from. Canonicalize ONCE; a
    // canonicalize failure (a `--root` that does not exist) falls back to the raw
    // path so the spawn still has a defined cwd and the ancestor walk a defined
    // start rather than silently inheriting the ambient one.
    let canonical_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());

    // FIX A — the 4th wedge stale-green close: capture the effective ANCESTOR
    // toolchain config (cargo/rustfmt/toolchain config ABOVE `--root`, plus
    // `$CARGO_HOME/config.toml`) as an additional memo-key input. cargo & rustfmt
    // search upward, so an ancestor `.cargo/config.toml`/`rustfmt.toml` flips the
    // gate's result while the `--root`-relative tree glob never sees it; folding
    // its digest into `def.inputs` (→ `def_digest` → memo key) makes such a change
    // a MISS. Computed from `canonical_root` so the per-def names are probed from
    // the SAME real path the tree axis snapshotted.
    let ancestor_cfg = ancestor_config_digest(&args.def, &canonical_root);

    // resolve_def runs BEFORE the log-not-found check so an unknown def with no
    // `--cmd` surfaces its OWN `unknown_def` fault first (the W0 dispatch contract:
    // dispatch reaches the resolver, not a premature log guard).
    let def = resolve_def(args, env_manifest, ancestor_cfg)?;
    // Axis 3 is taken from the resolved def's `toolchain_ref` — the SAME value
    // `resolve_def` baked into axis 2 (`compute_def_digest` hashes `toolchain_ref`)
    // — so the two axes can never disagree. When `--toolchain` is omitted this is
    // the REAL active-toolchain digest, so a compiler change busts the key.
    let toolchain_digest = def.toolchain_ref.clone();

    // log-not-found law (WG-CHECK-ROBUST): a `--log` that does not exist is the
    // explicit `log_not_found`/exit-2 error REGARDLESS of `--store` — a check
    // against a typo'd log is an error, never a silent dry green. (`--store` later
    // re-loads it under the lock; this is the early, store-independent guard.)
    if !args.log_path().exists() {
        return Err(PorcelainError::log_not_found(&args.log_path()));
    }

    // Exclude hugit's OWN wedge-state files from the tree axis (WH-CHECK
    // cmd-memoize fix). An ad-hoc def globs `**/*`, which would otherwise match the
    // `--log`, the `--ac` cache, and their `.lock`/`.tmp` sidecars when they live
    // under `--root` — so storing the cold result MUTATES the tree the very next
    // run hashes, busting the memo key and making every re-run a MISS. These files
    // are hugit STATE, never a check INPUT, so they must not contribute to the key.
    let ac_path = args
        .ac
        .clone()
        .unwrap_or_else(|| with_extension(&args.log_path(), "ac"));
    let excluded = state_file_exclusions(&[&args.log_path(), &ac_path]);
    let files = snapshot_tree(&root, &def.glob_set, &excluded);
    let ac = select_ac(args)?;
    let runner = ProcessRunner {
        timeout: Duration::from_secs(args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS)),
        root: Some(canonical_root),
        env: Some(captured_env),
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
        "log": args.log_path().display().to_string(),
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
fn record_on_log(args: &CheckRunArgs, payload: &serde_json::Value) -> Result<bool, PorcelainError> {
    let path = args.log_path();
    let path = &path;
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
///
/// Retryability is a TYPE, not a string convention (Round-8 C6 root fix; this
/// supersedes the K-RUN `starts_with("ac_busy:")` band-aid). The AC layer's
/// retryable `Busy` variant is matched STRUCTURALLY here and surfaces as a
/// retryable `ac_busy` kind; every other `AcError` is a terminal `ac_error`. The
/// exhaustive inner match means the compiler forces every NEW `AcError` variant to
/// be consciously classified retryable-or-terminal — a retryable case can never
/// again silently collapse to a terminal kind under fleet-shared-cache contention
/// (the flaky-gate failure mode that broke the contention loser's expected kind
/// under `--workspace` parallel load).
fn map_exec_error(e: ExecError) -> PorcelainError {
    use hugit_checks::client::ac::AcError;
    match e {
        // RETRYABLE: AC-file lock exhaustion or an HTTP 429/503 from the live AC.
        // A typed arm — no `starts_with("ac_busy:")` string-sniff (deleted).
        ExecError::Ac(AcError::Busy { detail }) => PorcelainError::new(
            "ac_busy",
            format!("the Action Cache is busy (retryable): {detail}"),
            "another hugit check holds the AC store, or the live AC is rate-limited; \
             retry shortly",
        ),
        // TERMINAL: every other AcError is a genuine fault, not contention. The
        // explicit variant list (not a wildcard) keeps the exhaustiveness guard:
        // a future variant fails to compile until it is classified above or here.
        ExecError::Ac(
            ac @ (AcError::NotWired(_)
            | AcError::Transport(_)
            | AcError::NotConfigured(_)
            | AcError::Status(_)
            | AcError::Decode(_)
            | AcError::DigestMismatch { .. }
            | AcError::InvalidKey(_)),
        ) => PorcelainError::new(
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

    fn args_for(def: &str, cmd: Option<&str>) -> CheckRunArgs {
        CheckRunArgs {
            def: def.to_string(),
            log: Some(PathBuf::from("x")),
            store: false,
            cmd: cmd.map(str::to_string),
            root: None,
            toolchain: None,
            pr: None,
            principal: None,
            ac: None,
            timeout_secs: None,
            env_axis: Vec::new(),
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
        let err =
            resolve_def(&args_for("echo-check", None), String::new(), String::new()).unwrap_err();
        assert_eq!(err.kind(), "unknown_def");
    }

    #[test]
    fn ad_hoc_def_with_cmd_builds_a_valid_def() {
        let def = resolve_def(
            &args_for("echo-check", Some("true")),
            String::new(),
            String::new(),
        )
        .unwrap();
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
            root: None,
            env: None,
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
            root: None,
            env: None,
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

    // ── Round-8 C6: typed ac_busy taxonomy ──────────────────────────────────────

    #[test]
    fn ac_busy_lock_exhaustion_maps_to_retryable_kind() {
        // The lock-exhaustion path returns the TYPED `AcError::Busy` variant; the
        // mapper classifies it as the RETRYABLE `ac_busy` kind via a typed arm —
        // no string-sniffing (`starts_with("ac_busy:")` is deleted).
        use hugit_checks::client::ac::AcError;
        let e = ExecError::Ac(AcError::Busy {
            detail: "the AC store stayed locked".to_string(),
        });
        let p = map_exec_error(e);
        assert_eq!(p.kind(), "ac_busy", "a busy AC is retryable, not terminal");
    }

    #[test]
    fn ac_http_429_503_map_to_retryable_busy() {
        // The HTTP fleet-shared-cache 429/503 conditions are TYPED `Busy` and so
        // map to the retryable `ac_busy` kind — the exact collapse-to-terminal the
        // Round-7/8 audits flagged. (These are produced at the ureq boundary as
        // `AcError::Busy`; here we assert the CLI-side classification.)
        use hugit_checks::client::ac::AcError;
        for detail in ["AC GET returned HTTP 429", "AC PUT returned HTTP 503"] {
            let p = map_exec_error(ExecError::Ac(AcError::Busy {
                detail: detail.to_string(),
            }));
            assert_eq!(p.kind(), "ac_busy", "{detail} is retryable");
        }
    }

    #[test]
    fn terminal_ac_errors_map_to_ac_error() {
        // A NON-busy AcError (e.g. a 401 bad-PAT status) is TERMINAL `ac_error`.
        use hugit_checks::client::ac::AcError;
        let p = map_exec_error(ExecError::Ac(AcError::Status(401)));
        assert_eq!(p.kind(), "ac_error", "a 401 is a terminal fault, not busy");
        let p2 = map_exec_error(ExecError::Ac(AcError::Transport("tls".into())));
        assert_eq!(p2.kind(), "ac_error", "a transport fault is terminal");
    }

    // ── Round-8 C3: hermetic env axis ───────────────────────────────────────────

    #[test]
    fn env_manifest_axis_hashes_path_and_busts_on_change() {
        // PATH's VALUE is hashed into the axis (F-MK5): two different PATHs yield
        // two different axes, so the memo key differs → a MISS, never a stale hit.
        let a = env_manifest_axis(&[("PATH".to_string(), "/binok:/usr/bin".to_string())]);
        let b = env_manifest_axis(&[("PATH".to_string(), "/binbad:/usr/bin".to_string())]);
        assert!(a.starts_with("PATH=sha256:"), "PATH value is hashed: {a}");
        assert_ne!(a, b, "a PATH change changes the env axis (busts the key)");
    }

    #[test]
    fn env_manifest_axis_captures_allowlisted_value_change() {
        // An allowlisted var's value is folded literally; a change busts the key.
        let a = env_manifest_axis(&[("RUSTFLAGS".to_string(), "-C opt-level=0".to_string())]);
        let b = env_manifest_axis(&[("RUSTFLAGS".to_string(), "-C opt-level=3".to_string())]);
        assert_ne!(a, b, "a RUSTFLAGS change changes the env axis");
    }

    #[test]
    fn unallowlisted_env_var_is_not_captured() {
        // A var off the allowlist (and not PATH) is NOT in the captured set, so it
        // is cleared at the spawn — never an off-key ambient input.
        assert!(!is_result_affecting_env("GATE_MODE"));
        assert!(!is_result_affecting_env("SSH_AUTH_SOCK"));
        // Allowlisted names + families ARE captured.
        assert!(is_result_affecting_env("RUSTFLAGS"));
        assert!(is_result_affecting_env("CARGO_HOME")); // CARGO_ prefix
        assert!(is_result_affecting_env("RUSTUP_TOOLCHAIN")); // RUSTUP_ prefix
        assert!(is_result_affecting_env("HOME"));
    }

    #[test]
    fn write_boundary_guard_refuses_a_secret_toolchain_axis() {
        // LAYER 2 (WK-AC): the FileAc store is fail-closed independently of the
        // door. A CheckResult carrying a structural-secret `toolchain_digest` is
        // REFUSED at the write boundary — never persisted to the `.ac` — even
        // though the door (validate_axis) was never invoked here.
        let dir = std::env::temp_dir().join(format!("hugit-wkac-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ac_path = dir.join("ac.json");

        let secret_tc = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let key = hugit_refstore::compute_memo_key("tree", "def", secret_tc);
        let leaky = CheckResult {
            memo_key: key,
            tree_hash: "tree".to_string(),
            def_digest: "def".to_string(),
            toolchain_digest: secret_tc.to_string(),
            exit: 0,
            artifacts: vec![],
            stdout_ref: String::new(),
            stderr_ref: String::new(),
            duration_ms: 1,
            runner_ref: "local".to_string(),
            produced_at: 0,
        };

        let ac = FileAc::new(ac_path.clone());
        let err = ac
            .store(&leaky)
            .expect_err("a secret toolchain axis must be REFUSED at the write boundary");
        assert!(
            matches!(err, hugit_checks::client::ac::AcError::Transport(ref m) if m.contains("toolchain_digest")),
            "the refusal names the offending axis: {err:?}"
        );
        // The `.ac` file must NOT have been written with the secret (fail-closed).
        let ac_bytes = std::fs::read_to_string(&ac_path).unwrap_or_default();
        assert!(
            !ac_bytes.contains("16C7e42F292c6912E7710c838347Ae178B4a"),
            "the refused secret never reached the .ac:\n{ac_bytes}"
        );

        // A legit (hex) toolchain axis stores normally — the guard is shape-gated,
        // not a blanket block (the cache still works).
        let tc = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let key2 = hugit_refstore::compute_memo_key("tree", "def", tc);
        let clean = CheckResult {
            memo_key: key2.clone(),
            tree_hash: "tree".to_string(),
            def_digest: "def".to_string(),
            toolchain_digest: tc.to_string(),
            exit: 0,
            artifacts: vec![],
            stdout_ref: String::new(),
            stderr_ref: String::new(),
            duration_ms: 1,
            runner_ref: "local".to_string(),
            produced_at: 0,
        };
        ac.store(&clean)
            .expect("a legit hex toolchain axis stores normally");
        assert!(
            ac.lookup(&key2).unwrap().is_some(),
            "the legit entry is a HIT — the cache still works"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_axis_rejects_secret_shapes_but_passes_legit_digests() {
        // The door reuses the shared structural detector: credential PREFIXES are
        // rejected (exit-2 secret_in_identifier); hex digests / `rustc …(hash)` /
        // slugs pass (bare-hex + entropy EXEMPT).
        assert_eq!(
            validate_axis("ghp_16C7e42F292c6912E7710c838347Ae178B4a", "--toolchain")
                .unwrap_err()
                .kind(),
            "secret_in_identifier"
        );
        validate_axis(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "--toolchain",
        )
        .expect("a 64-hex toolchain digest passes the door");
        validate_axis("rustc-1.96.0-abc123def456", "--toolchain")
            .expect("a rustc-version-shaped toolchain marker passes the door");
        validate_axis("clippy", "--def").expect("a built-in def name passes the door");
    }

    #[test]
    fn env_manifest_captures_only_allowlisted_result_affecting_vars() {
        // Wave-L C3 hermetic env axis (ported from K-RUN's manifest test onto
        // `env_manifest_axis(&captured_hermetic_env(&[]))`). A change to an
        // ALLOWLISTED var must change the manifest (→ a different def_digest →
        // memo-key bust → MISS, no stale green). A change to an UNLISTED var
        // (e.g. PWD/FOO) must NOT, so the
        // hit-rate is preserved. This test mutates process env, so it asserts on
        // the snapshot the function returns rather than racing other tests; the
        // var names used here (HUGIT_KRUN_*) are private to this test.
        //
        // SAFETY: set_var/remove_var are unsafe in the 2024 edition (env mutation
        // is not thread-safe). This test runs single-threaded reasoning over its
        // own private var names; we save/restore to avoid leaking into siblings.
        let listed = "RUSTFLAGS";
        let unlisted = "HUGIT_KRUN_NOT_AN_AXIS";
        let saved_listed = std::env::var(listed).ok();
        let saved_unlisted = std::env::var(unlisted).ok();

        unsafe {
            std::env::set_var(listed, "-C target-cpu=native");
            std::env::remove_var(unlisted);
        }
        let m_a = env_manifest_axis(&captured_hermetic_env(&[]));
        assert!(
            m_a.contains("RUSTFLAGS=-C target-cpu=native"),
            "an allowlisted var is captured into the env axis: {m_a:?}"
        );
        assert!(
            m_a.lines().any(|l| l == "RUSTFLAGS=-C target-cpu=native"),
            "the allowlisted var is a discrete, well-formed manifest line: {m_a:?}"
        );

        // Changing the allowlisted var changes the manifest (memo-key bust).
        unsafe {
            std::env::set_var(listed, "-C opt-level=3");
        }
        let m_b = env_manifest_axis(&captured_hermetic_env(&[]));
        assert_ne!(
            m_a, m_b,
            "a change to an allowlisted var changes the manifest (busts the key)"
        );

        // An unlisted var never enters the manifest (hit-rate preserved).
        unsafe {
            std::env::set_var(unlisted, "anything");
        }
        let m_c = env_manifest_axis(&captured_hermetic_env(&[]));
        assert!(
            !m_c.contains(unlisted),
            "an unlisted var is DECLARED not-result-affecting — never in the axis: {m_c:?}"
        );
        assert!(
            m_c.contains("RUSTFLAGS=-C opt-level=3"),
            "the unlisted-var change did not perturb the allowlisted capture: {m_c:?}"
        );

        // Manifest is canonical/sorted: keys appear in lexicographic order.
        unsafe {
            std::env::set_var("CARGO_TERM_COLOR", "never");
        }
        let m_d = env_manifest_axis(&captured_hermetic_env(&[]));
        let keys: Vec<&str> = m_d.lines().filter_map(|l| l.split('=').next()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(
            keys, sorted,
            "the manifest is sorted by key (canonical): {m_d:?}"
        );

        // Restore the environment we mutated.
        unsafe {
            match saved_listed {
                Some(v) => std::env::set_var(listed, v),
                None => std::env::remove_var(listed),
            }
            match saved_unlisted {
                Some(v) => std::env::set_var(unlisted, v),
                None => std::env::remove_var(unlisted),
            }
            std::env::remove_var("CARGO_TERM_COLOR");
        }
    }

    // (K-RUN's `ac_busy_lock_exhaustion_maps_to_retryable_kind_not_terminal_ac_error`
    // was removed here: it asserted the OLD string-path contract
    // — `AcError::Transport("ac_busy: …")` → retryable — which the Round-8 C6 typed
    // taxonomy deliberately drops. Retryability is now ONLY the typed `AcError::Busy`
    // variant, covered by `ac_busy_lock_exhaustion_maps_to_retryable_kind` and
    // `terminal_ac_errors_map_to_ac_error` above.)

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

    // ── Wave P FIX B: exec-only mode fold (umask no longer leaks) ────────────────

    #[cfg(unix)]
    #[test]
    fn file_mode_folds_only_exec_bits_not_umask_rw_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hugit-p-mode-{}-{}", std::process::id(), "u"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("file.txt");
        std::fs::write(&f, b"same bytes").unwrap();

        // A non-exec file at 0644 and the SAME bytes at 0664 (only the group-write
        // bit flipped — a pure umask difference git does NOT track) must fold to
        // the SAME mode value, so the memo key is umask-invariant (FIX B).
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        let m644 = file_mode(&f);
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o664)).unwrap();
        let m664 = file_mode(&f);
        assert_eq!(
            m644, m664,
            "a 0644 vs 0664 (umask) difference must NOT change the folded mode"
        );

        // But the EXEC bit IS result-affecting: chmod +x must change the fold so
        // `chmod -x` still busts the key (the N-1 P0 stays closed).
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        let m755 = file_mode(&f);
        assert_ne!(
            m644, m755,
            "adding the exec bit must change the folded mode (N-1 stays closed)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Wave P FIX A: ancestor toolchain-config digest ──────────────────────────

    #[cfg(unix)]
    #[test]
    fn ancestor_config_digest_busts_on_a_parent_cargo_config_change() {
        // Layout: parent/.cargo/config.toml is an ANCESTOR of the proj root (above
        // it), so the `--root`-relative tree glob never sees it — but cargo reads
        // it. The ancestor digest must move when its content changes (FIX A).
        let base = std::env::temp_dir().join(format!("hugit-p-anc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let parent = base.join("parent");
        let proj = parent.join("proj");
        std::fs::create_dir_all(parent.join(".cargo")).unwrap();
        std::fs::create_dir_all(&proj).unwrap();
        let cfg = parent.join(".cargo/config.toml");
        let root = std::fs::canonicalize(&proj).unwrap();

        // No ancestor config yet → a baseline digest.
        let d_none = ancestor_config_digest("clippy", &root);

        // Add the ancestor cargo config (caps lints) → digest MUST change (MISS).
        std::fs::write(&cfg, b"[build]\nrustflags = [\"--cap-lints=allow\"]\n").unwrap();
        let d_capped = ancestor_config_digest("clippy", &root);
        assert_ne!(
            d_none, d_capped,
            "adding an ancestor .cargo/config.toml must change the digest (R11-1 close)"
        );

        // Mutate the ancestor config (remove the cap) → digest MUST change again.
        std::fs::write(&cfg, b"[build]\nrustflags = []\n").unwrap();
        let d_nocap = ancestor_config_digest("clippy", &root);
        assert_ne!(
            d_capped, d_nocap,
            "mutating the ancestor config must change the digest (the R11-1 stale-green)"
        );

        // `fmt` does NOT read the cargo config, so its ancestor digest must be
        // INVARIANT to a `.cargo/config.toml` change (per-def scoping; hit-rate).
        let f_a = ancestor_config_digest("fmt", &root);
        std::fs::write(&cfg, b"[build]\nrustflags = [\"-C\", \"opt-level=0\"]\n").unwrap();
        let f_b = ancestor_config_digest("fmt", &root);
        assert_eq!(
            f_a, f_b,
            "fmt does not read .cargo/config — its ancestor digest is invariant to it"
        );

        // An ancestor rustfmt.toml DOES affect fmt → its digest must move.
        std::fs::write(parent.join("rustfmt.toml"), b"max_width = 1\n").unwrap();
        let f_c = ancestor_config_digest("fmt", &root);
        assert_ne!(
            f_b, f_c,
            "an ancestor rustfmt.toml must change the fmt ancestor digest"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn ancestor_config_digest_is_absolute_prefix_invariant() {
        // Two checkouts at DIFFERENT absolute prefixes with the SAME ancestor
        // config must compute the SAME digest (depth+relname label, not abspath) —
        // the cross-machine hit-rate is preserved.
        let mk = |tag: &str| -> String {
            let base = std::env::temp_dir().join(format!(
                "hugit-p-anc-inv-{}-{}",
                std::process::id(),
                tag
            ));
            let _ = std::fs::remove_dir_all(&base);
            let parent = base.join("parent");
            let proj = parent.join("proj");
            std::fs::create_dir_all(parent.join(".cargo")).unwrap();
            std::fs::create_dir_all(&proj).unwrap();
            std::fs::write(
                parent.join(".cargo/config.toml"),
                b"[build]\nrustflags = [\"--cap-lints=allow\"]\n",
            )
            .unwrap();
            let root = std::fs::canonicalize(&proj).unwrap();
            let d = ancestor_config_digest("clippy", &root);
            let _ = std::fs::remove_dir_all(&base);
            d
        };
        assert_eq!(
            mk("a"),
            mk("b"),
            "the ancestor digest is invariant to the absolute prefix (hit-rate preserved)"
        );
    }

    #[test]
    fn read_snapshot_content_caps_oversized_files_soundly() {
        // PS-17: a file at/under the cap folds as its raw bytes (byte-identical to
        // the old bare `fs::read`, so the memo key is UNCHANGED for real inputs); a
        // file OVER the cap folds as a bounded streaming-hash sentinel that STILL
        // changes when the file changes (no stale green) while bounding peak memory.
        // Exercised with a TINY cap so no >64 MiB fixture is materialized.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("hugit-ps17-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob");
        std::fs::write(&path, b"hello-world").unwrap(); // 11 bytes

        // Under the cap → raw bytes, byte-identical to a plain read (key unchanged).
        assert_eq!(
            read_snapshot_content_capped(&path, 1024).as_deref(),
            Some(&b"hello-world"[..]),
            "a file at/under the cap folds as its raw bytes"
        );

        // Over the cap (4 < 11) → a deterministic OVERSIZE sentinel, NOT the raw bytes.
        let over = read_snapshot_content_capped(&path, 4).expect("oversized read");
        assert!(
            over.starts_with(b"OVERSIZE:11:"),
            "an oversized file folds as a streamed-hash sentinel carrying its length"
        );
        assert_ne!(
            over, b"hello-world",
            "the oversized fold is not the raw bytes"
        );
        assert_eq!(
            over,
            read_snapshot_content_capped(&path, 4).unwrap(),
            "the oversized sentinel is deterministic for identical content"
        );

        // Soundness: a content change MUST change the sentinel (no stale green).
        std::fs::write(&path, b"hello-worlds").unwrap(); // changed (12 bytes)
        assert_ne!(
            over,
            read_snapshot_content_capped(&path, 4).unwrap(),
            "changing an oversized file must change its sentinel (PS-17 soundness)"
        );

        // A missing file is None — fail-safe "absent", exactly as `fs::read(..).ok()`.
        std::fs::remove_file(&path).unwrap();
        assert!(read_snapshot_content_capped(&path, 1024).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_axis_declares_a_custom_var_into_the_captured_set() {
        // PS-11: a caller-declared --env-axis var is captured (so it is both keyed
        // into the memo manifest AND passed to the hermetic spawn), alongside the
        // allowlist + PATH. Verified on a SYNTHETIC env via the pure inner form, so
        // no process-global env mutation (the cross-test data race, sweep T-3).
        let vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("RUSTFLAGS".to_string(), "-Cdebuginfo=0".to_string()), // allowlisted
            ("MY_GATE_MODE".to_string(), "strict".to_string()),     // custom
            ("PWD".to_string(), "/somewhere".to_string()),          // ambient, not keyed
        ];

        // Undeclared: the custom var is NOT captured (hermetic spawn would clear it).
        let base = captured_hermetic_env_from(vars.clone(), &[]);
        assert!(base.iter().any(|(k, _)| k == "PATH"));
        assert!(base.iter().any(|(k, _)| k == "RUSTFLAGS"));
        assert!(
            !base.iter().any(|(k, _)| k == "MY_GATE_MODE"),
            "an undeclared custom var is cleared (not captured)"
        );
        assert!(
            !base.iter().any(|(k, _)| k == "PWD"),
            "an ambient non-allowlisted var is cleared"
        );

        // Declared: the custom var is now captured WITH its value; PWD still is not.
        let declared = captured_hermetic_env_from(vars, &["MY_GATE_MODE".to_string()]);
        assert_eq!(
            declared
                .iter()
                .find(|(k, _)| k == "MY_GATE_MODE")
                .map(|(_, v)| v.as_str()),
            Some("strict"),
            "a declared --env-axis var is captured with its value"
        );
        assert!(
            !declared.iter().any(|(k, _)| k == "PWD"),
            "declaring one var does not capture unrelated ambient vars"
        );
    }

    #[test]
    fn env_axis_value_change_busts_the_memo_manifest() {
        // A change to a DECLARED var's value changes the env manifest → a different
        // def_digest → a MISS (the soundness PS-11 buys). An UNDECLARED var leaves
        // the manifest invariant (the hit-rate the hermetic clear preserves).
        let manifest = |mode: &str, declared: &[String]| {
            let vars = vec![
                ("PATH".to_string(), "/usr/bin".to_string()),
                ("MY_GATE_MODE".to_string(), mode.to_string()),
            ];
            env_manifest_axis(&captured_hermetic_env_from(vars, declared))
        };
        let axis = vec!["MY_GATE_MODE".to_string()];
        assert_ne!(
            manifest("strict", &axis),
            manifest("lax", &axis),
            "a declared var's value change busts the manifest (MISS)"
        );
        assert_eq!(
            manifest("strict", &[]),
            manifest("lax", &[]),
            "an undeclared var does not affect the manifest (hit-rate preserved)"
        );
    }
}
