# Round 11 — Did the fix waves (N + O) regress / introduce a new bug IN THE FIXES?

- **Scope:** single focus — regressions or new bugs introduced BY Wave N + Wave O
  themselves (the fixes touched security-critical + hot-path code under time
  pressure). `main` HEAD `5b73a8b`. READ-ONLY audit.
- **Method:** `git show` each fix commit; reasoned about the stated failure modes;
  built the engine once and ran TARGETED live repros + the new acceptance/property
  suites (no `cargo test --workspace` — disk tight). Scratch dirs OUTSIDE the repo,
  cleaned.
- **Commits scrutinized:** N-1 `7ab5e9d` · N-2 `177cf0a` · N-4 `7a2bd11` ·
  N-6 `4125fb6` · WO-GLOBMISS `551dac3` · O-1 `66767bd` · O-2 `e50603d`.

## Verdict: ONE new P2 finding (correctness-SAFE, hit-rate). No P0/P1 regression.

The integrity direction is sound everywhere: every fix that bears on memoized-CI
correctness errs toward **false MISS / re-execute**, never toward a false HIT /
stale green. The single new defect is an *over-scoping* in N-1 that costs
cross-machine cache hit-rate; it can never serve a stale green.

---

## Per-fix verdict

### N-1 (`7ab5e9d`) — fold POSIX mode into the tree axis — **ISSUE (P2, over-scope)**

- **Framing is correct & deterministic.** `frame_file_with_mode` =
  `MODE_TAG ‖ LP(u32_le(mode)) ‖ LP(content)`; `push_lp` length-prefixes
  big-endian (the doc comment's "u32_be(len)" matches the code), so no
  field-boundary collision. `#[cfg(unix)]`/`#[cfg(not(unix))]` sentinels are sane.
- **Stale-green close verified live** (engine `hugit check`, `crates/hugit-cli/src/checks/run.rs:473-486`):
  - cold `./gate.sh` (chmod +x) → key `463b…`, miss, exit 0
  - warm (same tree) → key `463b…`, **HIT** (determinism / hit-rate preserved)
  - `chmod -x` (same content) → key `afe9…`, **MISS, exit 126** — the real failure now surfaces.
  Acceptance `acceptance_n1_modebit` 4/4 pass.
- **NEW FINDING (P2):** the fold uses the FULL `meta.mode() & 0o7777`, not just
  the executable bit. Git only tracks the exec bit; the *other* permission bits
  (group-write etc.) on a checkout depend on the runner's **umask**. So two
  runners with different umasks compute DIFFERENT memo keys for byte-identical,
  same-exec-bit sources. On the disclosed **fleet-shared / cross-runner AC**
  (CLAUDE.md) this is a cross-machine cache MISS for identical work.
  - **Live repro** (`crates/hugit-cli/src/checks/run.rs:file_mode` reads `st_mode & 0o7777`):
    a non-executable file at `0644` → key `a72d…`; the SAME bytes at `0664`
    (only the group-write bit flipped — exactly a umask difference, NOT tracked
    by git) → key `346e…`. Different key ⇒ a cross-umask MISS.
  - **Severity = P2, correctness-SAFE.** A divergent key only ever causes
    re-execution (false MISS); it can NEVER produce a false HIT / stale green
    (distinct key ⇒ distinct cache slot ⇒ no poisoning). It costs hit-rate on a
    cross-umask fleet, not correctness. The stale-green repro that motivated the
    fix needed only the **exec** bit (`chmod -x`); folding all `0o7777`
    over-captures. Recommended (non-blocking): fold only the exec bit
    (`mode & 0o111`, or normalize to a 2-state exec/no-exec) so the key is umask-
    invariant while still busting on `chmod -x`. The tree is walked from the
    FILESYSTEM, not the git index (`run.rs` `collect_files`), so the umask
    leakage is real, not hypothetical.

### N-2 (`177cf0a`) — O(n) reconcile — **CLEAN**

- Hoists the store projection out of the per-id loop and builds an owned
  `HashSet<String>` id index; the in-sync fast-path returns early iff EVERY
  `--log` intent is already in the store. Each healed id is folded back into the
  set, so an in-batch DUPLICATE intent_id is skipped on the 2nd occurrence —
  preserving exactly the idempotency the old per-id re-projection gave
  implicitly. Behavior-identical; the fail-closed chain-broken path is
  unchanged (`map_err → ChainBroken`).
- `acceptance_n2_reconcile_perf` 2/2 pass (incl. the at-scale heal + idempotent
  re-run + tampered-log `chain_broken` correctness cases). No idempotency or
  backfill regression.

### N-4 (`7a2bd11`) — policy secret gate → shared `secret_shape` — **CLEAN**

- `hugit-policy` now depends on `hugit-ledger`; **no cycle** (`hugit-ledger`
  has no dep on `hugit-policy` — verified by `Cargo.toml` and a clean
  `cargo build -p hugit-policy`). The hand-maintained `SECRET_PATTERNS` list is
  replaced by `is_structural_secret`.
- Semantics BROADEN (more detectors fire: `clp_`, `github_pat_`,
  `xoxo/xoxa/xoxs-`, connection-string passwords, keyword-context) — the SAFE
  (fail-closed) direction; two boundary tests that documented the OLD narrow
  behavior were intentionally removed. `acceptance_n4_secret_parity` 16/16 +
  `acceptance_d6` 6/6 pass. No correctness regression.

### N-6 (`4125fb6`) — no-args → exit-2 + kind unify — **CLEAN**

- Live-verified on the built binary: no-args → structured
  `{"error":{"kind":"invalid_argument",…}}` on stdout, **exit 2**;
  `--help`/`-h` → exit 0; `--version` → exit 0; bad verb → exit 2;
  missing-required-arg (`why`) → exit 2; bare subcommand groups (`intent`,
  `pr`) → exit 2. No real success path now wrongly errors. `acceptance_n6_ergonomics` 6/6 pass.

### WO-GLOBMISS (`551dac3`) — per-def toolchain-config globs — **CLEAN**

- Per-def split is correct: `fmt` adds only `**/{,.}rustfmt.toml`;
  `clippy`/`test` add clippy + `.cargo/config{,.toml}` + `rust-toolchain{,.toml}`.
  `fmt` deliberately does NOT capture clippy/cargo configs (avoids busting a fmt
  HIT on an unrelated change — hit-rate preserved).
- The subtle path patterns work: I probed the real matcher —
  `**/.cargo/config.toml` matches BOTH top-level `.cargo/config.toml` and
  `sub/.cargo/config.toml` (the `**`-matches-empty-incl-slash case is handled);
  `**/*.rs`, `**/rust-toolchain.toml` etc. all match. Globs are NOT over-broad
  (not `**/*`). `acceptance_globmiss` 3/3 pass (rustfmt.toml change busts to a
  real exit 1; no-change + doc edit stay HITs).

### O-1 / O-2 (`66767bd` / `e50603d`) — proptests + test-quality — **CLEAN**

- `compute_memo_key` is LP-framed, so the proptest invariant "distinct tuples ⇒
  distinct keys" cannot fail on a field-boundary collision (verified by reading
  `hugit-refstore/src/log/mod.rs:169`). Proptests use `failure_persistence: None`
  + bounded strategies; deterministic per-run but fresh seeds across runs.
  **Flakiness stress:** 3× runs at `PROPTEST_CASES=2000` (≈4× the configured 512)
  → 0 failures, all 18 properties green every run. No seed-dependent flake.
- O-2 test-quality edits (env-guard, scratch-ctr, field-level scrub assert, src
  walker) are test-only hardening; no production code touched.

---

## VERDICT

- **P0:** none.
- **P1:** none.
- **P2:** **1** — N-1 folds the full `0o7777` mode, leaking the runner's umask
  into the memo key ⇒ cross-machine cache MISS on a fleet-shared AC for
  byte-identical sources. Correctness-SAFE (only ever a false MISS / re-execute,
  never a stale green). Fix: fold only the exec bit. Track as a hit-rate residual
  (sibling of the disclosed P2 AC seam), not a ship-blocker.

The fix waves did not introduce a correctness regression. The one new defect is a
hit-rate over-scope in the safe direction.
