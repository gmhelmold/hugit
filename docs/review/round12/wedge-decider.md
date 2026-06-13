# Round 12 — the deciding round for the memoized-CI wedge stale-green class

**Auditor:** fresh-context hostile auditor (read-only on code).
**Target:** hugit engine, `main` HEAD `3a63158` (Wave P merge).
**Verdict:** **NOT CONVERGED** — a 5th BOUNDED-LOCAL stale-green found and
live-reproduced end-to-end (cold GREEN → mutate a bounded ancestor axis → warm
serves the stale GREEN while the real gate is RED).

---

## Scope & method

- Code is READ-ONLY. The only write is this report. No edits, no commits.
- Build: `cargo build -p hugit-cli --locked` with the pinned 1.96.0 toolchain
  (succeeded). Repros use the built binary `target/debug/hugit`.
- The memo key is `H(tree_root ‖ def_digest ‖ toolchain_digest)`. The key is
  computed BEFORE execution and printed in the verb's JSON on every run, so a
  MISS vs HIT is decided by whether the key changes. A *stale-green* requires
  more than a key collision: a cold GREEN result stored under key K, a bounded
  axis mutated so the REAL gate flips RED, and a warm run still serving K's
  `exit=0 ok=true`. Every "hole" claim below is backed by that full cycle, not
  by a key comparison alone.
- Key sources read: `crates/hugit-cli/src/checks/run.rs` (the whole wedge —
  `builtin_glob_set`, `ancestor_config_names`, `ancestor_config_digest`,
  `captured_hermetic_env`, `file_mode`, `resolve_toolchain_digest`,
  `snapshot_tree`/`collect_files`) and
  `crates/hugit-checks/src/client/memo_key.rs` (the three-axis derivation +
  `frame_file_with_mode`).
- Scratch lived in `/tmp/r12wedge` (outside the repo) and was cleaned on exit.

---

## A. Re-confirmation of the 4 prior fixes — ALL HOLD

| # | Fix | Live result |
|---|-----|-------------|
| 1 | **env axis (K-RUN)** — `RUSTFLAGS="" ` vs `-C opt-level=2` (ad-hoc) | key changes → **MISS** (PASS) |
| 2a | **exec-bit (N-1 / Wave-P `0o111`)** — `chmod -x gate.sh` same content | key changes → **MISS** (PASS) |
| 2b | **umask-invariance (Wave P FIX B)** — `chmod 0644` vs `0664` non-exec | key UNCHANGED → **HIT** (PASS — umask doesn't bust) |
| 3a | **in-root `rustfmt.toml` add** (`--def fmt`) | **MISS** (PASS) |
| 3b | **in-root `rustfmt.toml` change** | **MISS** (PASS) |
| 3c | **in-root `.cargo/config.toml` add** (`--def clippy`) | **MISS** (PASS) |
| 4a | **ancestor `rustfmt.toml`** (parent dir above `--root`, `fmt`) | **MISS** (PASS) |
| 4b | **ancestor `rustfmt.toml` change** | **MISS** (PASS) |
| 4c | **ancestor `.cargo/config.toml`** (parent dir above `--root`, `clippy`) | **MISS** (PASS) |

No Wave-P regression. All four prior closes are intact.

---

## B. The 5th-hole hunt

Each candidate is classified **captured** (key busts) / **bounded-hole** (a
known finite axis that does NOT bust, with a real-gate flip) / **unbounded-P2**
(arbitrary read / network / clock — the accepted hermetic seam).

### B.1 — in-root `Cargo.toml` `[lints]`/`[profile]`/`[patch]` — CAPTURED
`builtin_glob_set` includes `**/Cargo.toml` in its base set, so any in-`--root`
`Cargo.toml` edit busts the key. Live: in-root `[lints.clippy]` add → MISS.
**Captured.**

### B.2 — WORKSPACE-PARENT `Cargo.toml` (ABOVE `--root`) — **BOUNDED HOLE (the 5th)**
This is the decider. In a cargo **workspace**, the member crate's effective
build configuration is inherited from the **workspace-root `Cargo.toml`**, which
cargo discovers by walking UP from the invocation dir — the SAME deterministic
upward walk that already makes `.cargo/config.toml` and `rust-toolchain.toml`
result-affecting ancestors (and which Wave P's `ancestor_config_digest` already
captures for those names). But `ancestor_config_names` does **not** include
`Cargo.toml`, and the tree glob is `--root`-relative, so a workspace-root
`Cargo.toml` sitting *above* `--root` is captured by **neither axis**.

**Confirmed result-affecting + bounded:** `cargo metadata` resolves the
workspace from the member dir; `[workspace.lints]`, `[profile]`, and `[patch]`
in the parent manifest all change the build outcome. The filename is a single
known token (`Cargo.toml`) on the same finite upward walk as the configs Wave P
already captures — it is **bounded**, not the unbounded P2 fixture class.

**Live end-to-end stale-green (the full cycle):**

```
workspace layout:  $WS/Cargo.toml  ([workspace] + [workspace.lints.clippy])
                   $WS/Cargo.lock
                   $WS/proj/         <-- --root
                   $WS/proj/Cargo.toml  ([lints] workspace=true)
                   $WS/proj/src/main.rs  (fn f() -> i32 { return 1; })

cold (parent lints: needless_return="allow"):
  real clippy   -> GREEN (exit 0)
  hugit check   -> cache_hit=false exit=0 ok=true  key=6656593f…   (stored)

mutate ONLY the parent $WS/Cargo.toml:  needless_return  "allow" -> "deny"
  real clippy   -> RED  (exit 101: "unneeded `return` statement", -D warnings)

warm (same --root, parent ws now denies the lint):
  hugit check   -> cache_hit=TRUE  exit=0 ok=true  key=6656593f…   <-- STALE GREEN
```

The warm key is byte-identical to the cold key (`6656593f…`) even though the
real gate has flipped RED — hugit serves a memoized `exit=0 ok=true`. This is a
genuine BOUNDED-LOCAL stale green: a fleet that lands a member PR after a
workspace-lint tightening would see a fabricated pass.

**Breadth of the same hole (all HIT / not-captured, key unchanged):**
- parent `Cargo.toml` `[workspace.lints]` allow→deny — full stale-green (above).
- parent `Cargo.toml` `[profile.dev]` add — key UNCHANGED (not captured).
- parent `Cargo.toml` `[patch.crates-io]` add — key UNCHANGED (not captured).

The fix locus is narrow: `ancestor_config_names` (in
`crates/hugit-cli/src/checks/run.rs`) must additionally probe `Cargo.toml` (and
the same upward `Cargo.lock`) for the `clippy`/`test` defs, exactly as it
already probes `.cargo/config.toml`/`rust-toolchain.toml`. The in-root glob is
correct as-is (it is `--root`-relative by design); the gap is strictly the
ABOVE-`--root` workspace manifest.

### B.3 — ancestor `Cargo.lock` (workspace root, above `--root`) — bounded-hole (same class)
The built-in `clippy`/`test` commands pass `--locked`; the authoritative
`Cargo.lock` of a workspace member lives at the workspace root, i.e. ABOVE
`--root`. A change to it is not captured (key unchanged). In a `--locked` world
a lock mismatch makes cargo *error* (RED both ways), so the stale-GREEN harm is
weaker than B.2, but it is the same bounded ancestor-manifest class and should
be closed alongside `Cargo.toml`.

### B.4 — `RUSTC_WRAPPER`/`sccache` — captured
`RUSTC_WRAPPER` is in `RESULT_AFFECTING_ENV_EXACT`; the `CARGO_`/`RUST_*`
families are prefix-captured. A wrapper change busts the env axis. **Captured.**

### B.5 — `[env]` table in `.cargo/config.toml` injecting vars — captured (via content), bounded
A `.cargo/config.toml` `[env]` table can inject result-affecting vars into the
build. The env axis snapshots the *ambient* process env (not cargo-injected
vars), so it does NOT see those injected values directly — BUT the
`.cargo/config.toml` file CONTENT is itself captured (in-root by the glob, and
in ancestors by `ancestor_config_digest`), so changing the `[env]` table changes
that file's bytes → MISS. The injected-value path is therefore covered by the
file-content capture. **Captured** (transitively, by config-file content).

### B.6 — toolchain digest: two toolchains, same version string — accepted bounded edge
`default_toolchain_digest` hashes `rustc --version --verbose` (release + commit
hash + commit date + host + LLVM), not the binary bytes. Two distinct binaries
emitting an identical verbose string would collide. This is the previously-noted
accepted edge (the verbose string is the deterministic toolchain identity); not
a new finding.

### B.7 — ancestor walk depth / `$CARGO_HOME` / case — sound
`ancestor_config_digest` walks parent→root with no early stop, labels by
*depth* (absolute-prefix-invariant, hit-rate-preserving cross-machine), and
additionally folds `$CARGO_HOME/config.toml` + extensionless `config` for
`clippy`/`test`. No early-termination or `$CARGO_HOME`-skip hole observed. A
config ABOVE `$CARGO_HOME` is captured by the depth walk; `$CARGO_HOME` itself
is env-captured. Sound for the names it probes — the gap is purely the MISSING
NAME `Cargo.toml`, not the walk.

### B.8 — non-built-in `--cmd` path (`**/*`) — sound
The ad-hoc path globs `**/*` relative to `--root`, so any in-root edit busts the
key (verified throughout Section A's ad-hoc repros). The ancestor-config class is
built-in-only by construction. No shared hole. **Sound.**

### B.9 — determinism (collision / false-MISS) — none observed
Two byte+mode+config-identical trees at different absolute prefixes compute the
SAME key (depth/relative labels, path-relative tree axis) — correct, hit-rate-
preserving, no false MISS. No false HIT (collision) surfaced across the probed
axes.

### Unbounded-P2 (accepted, NOT a failure)
A built-in `test` reading an ARBITRARY fixture (`tests/data/<anything>`,
`include_str!`/`include_bytes!`, a `build.rs`-emitted path), network, or clock
remains the disclosed P2 hermetic seam — not closable by enumeration. Correctly
documented in `builtin_glob_set`'s honest residual. **Out of local scope.**

---

## VERDICT

**NOT CONVERGED.** The 4 prior fixes all hold, but a 5th BOUNDED-LOCAL
stale-green exists and was live-reproduced end-to-end: the **workspace-parent
`Cargo.toml` above `--root`** (its `[workspace.lints]`/`[profile]`/`[patch]`) is
read by cargo on the same deterministic upward walk Wave P already captures for
`.cargo/config.toml`/`rust-toolchain.toml`, yet `ancestor_config_names` omits
`Cargo.toml`. A `[workspace.lints.clippy]` `allow`→`deny` flips the real
`clippy` gate RED while `hugit check` serves a memoized `exit=0 ok=true`.

**Bounded-vs-unbounded reasoning:** this is BOUNDED — it is a single known
filename (`Cargo.toml`, plus the workspace `Cargo.lock`) on a finite, ordered,
deterministic parent walk, identical in shape to the configs Wave P already
folds. It is NOT the unbounded arbitrary-read/network/clock P2 seam. Therefore
it counts as a NOT-CONVERGED finding.

**Fix locus (single, narrow):** add `Cargo.toml` (and the upward `Cargo.lock`)
to `ancestor_config_names` for the `clippy`/`test` defs in
`crates/hugit-cli/src/checks/run.rs`, so `ancestor_config_digest` folds an
above-`--root` workspace manifest into the key — exactly as it does today for
`.cargo/config.toml`. The in-root glob and the ancestor walk machinery need no
change. After this close, re-run the hunt for a 6th bounded axis before claiming
convergence.
