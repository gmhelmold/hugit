# Round 13 — All-class confirmation + Wave P/Q regression check

Fresh-context hostile auditor. `main` HEAD `6122b8f` (Wave Q). READ-ONLY.
Scope: the FIVE non-wedge classes + a Wave P/Q (`checks/run.rs`, `memo_key.rs`)
regression check. The wedge decider is a sibling agent's scope, excluded here.

Build: `cargo build -p hugit-cli --locked` (1.96.0 toolchain) — clean.
All findings are live-reproduced against `target/debug/hugit` (scratch in
`/tmp/r13scratch`, outside the repo) or precisely code-cited.

## VERDICT

**No P0/P1 in any of the five classes. No Wave P/Q regression.** The five
classes HOLD and Waves P/Q are CLEAN. One P2/honesty residual noted (already
in the disclosed P2 hermetic-execution seam class), no new finding.

---

## A. The five non-wedge classes — all HOLD

### 1. REDACTION — HOLD (live)
`campaign open --charter "ghp_AAAA…(40) secret PAT" --owner main` →
log at rest carries `"charter":"[REDACTED]"`; `grep` for the raw 40-char PAT
in `log.json` = 0 hits. Secret in a free-text field of a real verb is redacted
at rest via the unified `secret_shape`. No leak.

### 2. READ-PATH — HOLD (live)
Tampered a benign string field in record 0 of a real intent log (chain not
re-stitched). Read verbs:
- `pr show --log tampered.json` → `kind:"chain_broken"` ("this_hash mismatch at seq 0"), exit-2.
- `campaign show --log tampered.json` → `chain_broken`, exit-2.
- `export --log tampered.json --out …` → `chain_broken`, exit-2 (no export off a tampered snapshot).
- `why` rejected my array-shaped tamper on its OWN wrapper-shape grounds (not a
  bypass — it reads a distinct `[{record,attestation?,sidecar?}]` shape;
  `verify_chain` is wired at `replay/undo/compaction/recovery` + the read verbs
  per the chokepoint, cited `crates/hugit-refstore/src/{tamper,replay,recovery,compaction,undo}/mod.rs`).
Chain re-verification is the read chokepoint; tamper → exit-2 on every read verb tested.

### 3. AUTHZ / D14 — HOLD (code)
`crates/hugit-refstore/src/log/mod.rs:386` `pub(crate) fn append` (raw append
is crate-private); `:432` `pub fn append_authorized` is the only public
mutation. `grep` over `crates/hugit-cli/src/` for raw `.append(` (excluding
`append_authorized`) = 0. Every verb-crate mutation routes through the guarded
`append_authorized` (D14 matrix). No forge-able mutation from a verb.

### 4. STATE-MACHINE — HOLD (live)
`campaign close smc` → sealed. Re-`close` → idempotent `already_closed:true`
(no double-seal). `intent new --campaign smc` (sealed) → `kind:"campaign_sealed"`
exit-2 ("a sealed campaign … cannot be mutated"). Sealed-terminal + reject-sticky
enforced; reconcile is fail-closed (cited `verdict/mod.rs:210`,
`export/cut.rs:82`, `intent/canonical_log.rs:217`).

### 5. ERROR-LAW — HOLD (live)
- Malformed JSON (`{not valid json`) → `kind:"parse"` exit-2, no panic.
- 100 000-deep nested array → `kind:"parse"` exit-2, no stack overflow (serde
  recursion limit fires structurally).
- 10 MB top-level array → `kind:"parse"` exit-2.
All adversarial input yields a structured envelope, never a panic. Atomic
multi-write via `pr/filelock.rs:187 atomic_write` (temp + rename, unique
pid+nanos name).

These five chokepoints are in `hugit-refstore`/`hugit-ledger`/`pr` — **untouched
by Wave P/Q** (whose diff is confined to `checks/run.rs` + the `memo_key.rs`
mode-fold doc/selection), so the Round-11 pass is preserved by construction and
confirmed by live repro.

---

## B. Wave P/Q regression check — CLEAN

Diff reviewed: `ancestor_config_digest` upward walk (`run.rs:257–316`), the
per-def `ancestor_config_names` sets (`:192–226`, Wave Q adds `Cargo.toml`/
`Cargo.lock` to clippy/test), and the exec-only `& 0o111` mode fold
(`file_mode` `run.rs:661`; `frame_file_with_mode` made mode-agnostic in
`memo_key.rs`).

**The ancestor WALK introduces no problem:**
- **Perf / unbounded walk:** the walk climbs `canonical_root.parent()` to the
  filesystem root, doing ~8 enumerated `fs::read` probes per level. A 120-deep
  root → full run in **0.15 s**. Bounded by real FS depth; no concern.
- **Symlink loop in ancestors:** IMPOSSIBLE — `--root` is canonicalized once
  (`run.rs:1353`), and `.parent()` on a canonical path strictly shortens to
  root. Live: a `a/self → a` dir-symlink root path collapses on canonicalize and
  terminates (no hang, structured JSON). No loop.
- **Permission-denied ancestor:** an unreadable candidate FILE (`chmod 000
  Cargo.toml` in a searchable ancestor) → `fs::read` Err is swallowed
  (`run.rs:273`), file silently skipped, run completes structured, no panic.
  (Minor honesty edge: an unreadable config reads identically to an absent one,
  so a perm-flip alone wouldn't bust the key — pathological, P2 class, not a
  regression.)
- **Huge ancestor file (DoS):** a 200 MB ancestor `Cargo.toml` is read whole
  into memory (peak footprint ~400 MB) — confirmed real, but bounded to the
  ENUMERATED config names and is the SAME disclosed **P2 hermetic-execution /
  unbounded-read seam** the code already discloses (`run.rs:127–140`,
  `:255–256`). No NEW finding; not closable by enumeration.
- **`$CARGO_HOME` unset/empty/weird:** `cargo_home_dir()` filters empty
  CARGO_HOME/HOME → `None`; weird/nonexistent path → `fs::read` Err swallowed.
  Live: `CARGO_HOME=` `HOME=` and `CARGO_HOME=/nonexistent` both run structured,
  no panic. A real `$CARGO_HOME/config.toml` content change BUSTS the key (live).
- **Non-UTF8 paths:** labels are `anc{depth}/{rel}` from the static ASCII name
  list (not the absolute path), so no UTF8 dependence on the FS prefix.

**Original mode-bit P0 still detected:** ad-hoc def, `chmod 755 → 644` on
`gate.sh` → **memo key MOVES** (MISS) = N-1 P0 stays closed. Pure-umask `644 →
664` (no exec change) → **SAME key** = umask-invariant (the Wave-P FIX B goal).
`frame_file_with_mode` is mode-agnostic; only `file_mode`'s `& 0o111` selection
changed — correct.

**No determinism break in `ancestor_config_digest`:** two checkouts at
DIFFERENT absolute prefixes (`/tmp/.../loc1/sub/proj` vs
`/tmp/.../loc2/elsewhere/deeper/proj`) with an identical parent
`.cargo/config.toml` → **IDENTICAL memo key** (`f0c904ec…`). The `anc{depth}/`
labeling (depth, not absolute path) makes the digest absolute-prefix invariant —
no cross-machine false MISS. Wave-P ancestor `.cargo/config.toml` change and
Wave-Q parent `Cargo.toml`/`Cargo.lock` change both BUST the key (live, the
4th/5th stale-green closes verified).

**Riskiest aspect checked:** absolute-path leakage into the digest (cross-machine
false MISS) — proven absent by the prefix-invariance repro above.

---

## Residuals (no new finding)
- The unbounded outside-`--root` config-file read (huge-file DoS, unreadable=absent)
  remains the disclosed **P2 hermetic-execution seam** — already documented, not
  closable by enumeration locally.
