# Round 13 — The Wedge Convergence Decider

**Auditor:** fresh-context HOSTILE, READ-ONLY.
**Target:** hugit memoized-CI wedge, `main` HEAD `6122b8f` (Wave Q).
**Question:** Is there a 6th BOUNDED-local stale-green, or is the wedge finally DRY
(every remaining vector is the genuinely-UNBOUNDED P2 hermetic seam)?

**VERDICT: CONVERGED (local).** All 5 prior fixes hold under live repro; no 6th
bounded-local stale-green was found. Every remaining stale-green vector is the
documented, genuinely-unbounded P2 hermetic-execution seam (arbitrary in-tree
fixture / `build.rs` / `include_*!` reads that a narrow built-in glob cannot
enumerate, plus network/clock). The wedge **local-soundness class is CLOSED.**

Method: built `hugit-cli` (`1.96.0`, `--locked`); drove the real `hugit check`
binary against scratch trees OUTSIDE the repo (`/tmp/r13.*`); observed the
emitted `memo_key`/`cache_hit` (the key is computed before execution, so a MISS
is observable even where `cargo`/`rustc` is absent on the hermetic PATH → `exit 127`).
No edits, no commits, no `cargo test --workspace`.

---

## A. The 5 prior fixes — ALL HOLD (live)

Code single-source: `crates/hugit-cli/src/checks/run.rs` (axes/capture) +
`crates/hugit-checks/src/client/memo_key.rs` (the three-axis key formula).

| # | Fix (wave) | Live result |
|---|-----------|-------------|
| 1 | **Env axis** (Round-8 C3, hermetic env) | `RUSTFLAGS` set → MISS (key `f9b0…`→`18e1…`); unlisted `FOO` → HIT (unchanged). PATH change → MISS. HOLDS. |
| 2 | **File-mode (exec-only)** (Wave P FIX B) | `0755`→`0644` (chmod -x) → MISS (`f9b0…`→`0276…`); `0664` vs `0644` (both non-exec, pure umask) → HIT (same key); restore `0755` → original key. Umask-invariant, exec-sensitive. HOLDS. |
| 3 | **In-root toolchain config** (WO-GLOBMISS) | In-root `.cargo/config.toml` add + mutate under `--def clippy` → distinct key each time (MISS). HOLDS. |
| 4 | **Ancestor `.cargo/config` + rustfmt** (Wave P FIX A) | Ancestor `.cargo/config.toml` add+mutate → MISS (clippy); ancestor `rustfmt.toml` → MISS (fmt). HOLDS. |
| 5 | **Ancestor `Cargo.toml`/`Cargo.lock`** (Wave Q) | Ancestor `Cargo.toml [workspace.lints]` add+mutate → MISS; ancestor `Cargo.lock` add+mutate → MISS (clippy). HOLDS. |

No regression on any axis.

---

## B. Hunt for a 6th BOUNDED-local stale-green

| Attempt | Classification | Evidence |
|---|---|---|
| **B1 — ancestor `.cargo/config.toml [env]` table value change** | **CAPTURED** | The config FILE content is folded into the ancestor digest (Fix 4/5). Changing `[env]` `MY_BUILD_FLAG="v1"`→`"v2"` → MISS (`8233…`→`cb6c…`). The injected value reaches the key via the captured FILE-CONTENT axis (it need not also reach the `std::env::vars()` env axis). Not a hole. |
| **B2 — `[patch]`/`[replace]`/`[profile]` in workspace Cargo.toml** | **CAPTURED** | `Cargo.toml` is captured whole-file (in-root glob + ancestor names). `[workspace.lints]` add/mutate proven to bust; the same byte-level fold covers `[patch]`/`[profile]`/`[replace]`. Not a hole. |
| **B3a — ancestor `rust-toolchain.toml` channel/components** | **CAPTURED** | Ancestor `rust-toolchain.toml` add + `channel` change → MISS (clippy). In-root `rust-toolchain.toml` is also tree-globbed. |
| **B3b — rustup directory override (`~/.rustup/settings.toml [overrides]`, NOT a tree file)** | **CAPTURED BY EFFECT (live probe)** — strongest 6th attempt | The toolchain axis is `sha256(rustc --version --verbose)` computed by a LIVE probe (`default_toolchain_digest`, run.rs:461). A rustup override changes which toolchain the `rustc` proxy resolves → different verbose version string → different digest → MISS. The override is captured by its EFFECT on the live probe, not by hashing the settings file. Residual: if `rustc` is OFF PATH the probe returns the honest distinct `toolchain-unprobed` sentinel (visible, never a fabricated hex) — a probe-availability degradation, not a silent stale-green. Not a bounded-local content hole. |
| **B4 — `CARGO_TARGET_DIR` redirect** | **CAPTURED** | `CARGO_` prefix → env axis. `CARGO_TARGET_DIR=/tmp/x` vs `/tmp/y` → distinct keys (MISS). Not a hole. |
| **B5 — `build.rs`/`include_*!`/test reads a NON-globbed in-tree file** | **GENUINELY-UNBOUNDED P2** (the disclosed seam) | Built-in `clippy`/`test` glob is narrow (`*.rs`, `Cargo.toml`, `Cargo.lock`, config). Adding+mutating `src/fixture.json` under `--def clippy` → key UNCHANGED (`2450…` throughout) → a real run that reads it would be a stale green. **This is exactly the unbounded hermetic-fixture seam already documented in `builtin_glob_set` (run.rs:127-140)**: the set of files a `test`/`build.rs` reads is OPEN, not enumerable by globs — the same class as the campaign-#1 runner-side isolated-rootfs P2 seam. NOT a bounded-local close. The ad-hoc `--cmd` path (glob `**/*`) DOES capture the same `fixture.json` (MISS), so the unbounded gap is confined to the narrow-glob built-ins by design. |
| **Determinism — false MISS** | **SOUND** | Two byte+mode+config-identical trees at DIFFERENT absolute prefixes → SAME key (`f0a4…` both). Hit-rate preserved cross-machine (depth-labeled, prefix-invariant ancestor digest). |
| **Determinism — false HIT/collision** | **SOUND** | One-byte content difference → distinct key (`f0a4…` vs `949b…`). No collision. |
| **`--cmd` path glob `**/*` soundness** | **SOUND** | Captures top-level files, dotfiles, and dotdirs (`.cargo/config.toml`, `.hidden`, `sub/.dotnested` — each add busts). `**/` matches the empty prefix (matcher + tests). Only prunes `target`/`.git`/`.claude` (build-output / VCS / agent-config — never check inputs). No silent drop. |

---

## Why this is convergence, not "one more round"

The 5 prior closes each captured a DETERMINISTICALLY-LOCATABLE, FINITE input
class: a fixed env allowlist, the POSIX exec bit, and an ENUMERATED set of
toolchain-config filenames probed in-root and up the ancestor chain
(`{Cargo.toml, Cargo.lock, .cargo/config{,.toml}, rust-toolchain{,.toml},
clippy.toml/.clippy.toml, rustfmt.toml/.rustfmt.toml}` + `$CARGO_HOME/config.toml`).

Round 13 exercised the remaining KNOWN cargo/rustc config surface — the `[env]`
table, `[patch]`/`[profile]`/`[replace]`, the rustup override-file,
`CARGO_TARGET_DIR`, registry/source config — and every one is already captured
(by file-content, by env-prefix, or by the live toolchain probe). The ONLY
surviving stale-green vector (B5) is the arbitrary-in-tree-read of a narrow-glob
built-in: an OPEN input set that cannot be closed by enumerating more globs (more
globs would only widen toward `**/*`, trading the wedge's hit-rate for a
correctness gap that the ad-hoc `**/*` path already closes for callers who want
it). That is the SAME unbounded class as the disclosed P2 hermetic-execution seam
— the runner-side isolated rootfs of campaign #1, where the action physically
cannot read outside the seeded tree axis. It is not a 6th bounded-local hole.

## VERDICT

**CONVERGED (local).** The wedge's local-soundness class is CLOSED: 5 fixes hold,
no 6th bounded-local stale-green exists, and the residual stale-green vector is
the genuinely-unbounded, already-disclosed P2 hermetic seam (tracked for the
runner-side rootfs, not closable by local enumeration). Determinism is sound in
both directions. Recommend retiring the bounded-stale-green hunt and tracking only
the unbounded hermetic-execution seam under pending-seams (P2).
