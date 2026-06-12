# CLASS 2 — READ-PATH INTEGRITY — Round 9 re-audit

**Round 9 — 2026-06-12**
**Auditor:** Fresh-context adversarial agent (no carry-forward from Round 8)
**Scope:** Every code path that reads/projects the hash-chained event-log
(`[EventRecord, …]` canonical log OR the `IntentStore`-embedded event spine)
and whether it calls `verify_chain` before projecting any authoritative state.
Wave L (L-B) closed the Round-8 hole (`intent::list::resolve_landed`).

---

## 1. Scope & method

**Invariant:** Every read path that deserialises an event log and projects
authoritative state MUST call `hugit_refstore::verify_chain` before projecting.
A tampered or reordered log must yield a structured `chain_broken`/exit-2
error, never a silent authoritative projection.

**Method:**
1. Enumerated every `EventLog::new() + push_record` site and every
   `load_event_log` / `load_canonical_log` / `load_log` call in
   `crates/hugit-cli/src/` (production source only, excluding `#[cfg(test)]`).
2. Traced each to confirm `verify_chain` is called before any projection.
3. Identified `intent/store.rs::IntentStore::load` as a NEW read path
   (present in Wave L HEAD but not enumerated in Round-8 matrix) — the
   `IntentStore` has its own embedded `[EventRecord, …]` spine, hash-chained
   and verified via `verify_chain` inside `IntentStore::load`.
4. Performed live tamper repros for `intent list` (the R8 hole now fixed) and
   `intent show` + `intent list` (store path, the NEW path).
5. Ran `cargo test --test acceptance_round8_readpath --locked` to confirm the
   regression tests added by Wave L are green.
6. Inspected `docs/plan/2026-06-11-pending-seams.md` PS-8 AC-4 for honesty
   against the current code.

**Files examined:**
- `crates/hugit-cli/src/main.rs` — `run_why`, `run_export`, `run_tournament`
- `crates/hugit-cli/src/checks/mod.rs` — `load_event_log`, `show`
- `crates/hugit-cli/src/checks/run.rs` — `record_on_log`
- `crates/hugit-cli/src/pr/cli.rs` — `load_log`, `load_log_or_empty`; all PR verbs
- `crates/hugit-cli/src/campaign/world.rs` — `load_canonical_log`, `World::load`
- `crates/hugit-cli/src/intent/canonical_log.rs` — `load`
- `crates/hugit-cli/src/intent/list.rs` — `resolve_landed` (the R8 hole, now fixed)
- `crates/hugit-cli/src/intent/show.rs` — `run` via `IntentStore::load_existing`
- `crates/hugit-cli/src/intent/store.rs` — `IntentStore::load` (NEW in this matrix)
- `crates/hugit-cli/src/queue/mod.rs` — `show`
- `crates/hugit-cli/src/verdict/mod.rs` — `record`
- `crates/hugit-cli/src/export/cut.rs` — `Cut::take_to` (verify before cut)
- `crates/hugit-cli/src/export/mod.rs` — `export` (via cut) + `restore_from_bytes`

---

## 2. Matrix (every read path × verify_chain ✓/✗ × projection)

| Read path | Entry point | Calls verify_chain? | What it projects |
|---|---|---|---|
| `hugit why --log` | `run_why` → inline `verify_chain` | ✓ K-CHAIN | intent provenance from log |
| `hugit export --log` | `run_export` → `checks::load_event_log` | ✓ K-CHAIN (via loader) | full export corpus |
| `hugit tournament --log` | `run_tournament` → `checks::load_event_log` | ✓ (via loader) | intent existence check |
| `hugit checks show --log` | `show` → `checks::load_event_log` | ✓ (loader) | check rows, KPIs |
| `hugit check --store --log` | `record_on_log` → `checks::load_event_log` | ✓ (via loader) | dedup scan before append |
| `hugit queue show --log` | `queue::show` → `checks::load_event_log` | ✓ (via loader) | queue entries, batches |
| `hugit verdict --store --log` | `verdict::record` → `checks::load_event_log` | ✓ (via loader) | intent existence, dedup |
| `hugit pr show --log` | `run_show` → `load_log` | ✓ (`pr::cli::load_log`) | PR intents, queue state |
| `hugit pr list --log` | `run_list` → `load_log` | ✓ (`pr::cli::load_log`) | all PR rows |
| `hugit pr land --log` | `run_land` → `load_log` | ✓ (`pr::cli::load_log`) | queue projection |
| `hugit pr open --log` | `run_open` → `load_log_or_empty` → `load_log` | ✓ (via `load_log`) | intent existence, campaign check |
| `hugit pr abandon --log` | `run_abandon` → `load_log` | ✓ (`pr::cli::load_log`) | PR open record |
| `hugit campaign show --log` | `World::load_existing` → `load_canonical_log` | ✓ (`load_canonical_log`) | PR phases, ledger, rollup |
| `hugit campaign list --log` | `World::load_existing` → `load_canonical_log` | ✓ (`load_canonical_log`) | campaign keys, progress |
| `hugit campaign open --log` | `World::lock_and_load` → `load_canonical_log` | ✓ (`load_canonical_log`) | bootstrap / idempotency |
| `hugit campaign close/abandon --log` | `World::lock_and_load(bootstrap=false)` → `load_canonical_log` | ✓ (`load_canonical_log`) | seal condition |
| `hugit intent new --log` (canonical_log) | `land_intent` → `canonical_log::load` | ✓ (`canonical_log::load`) | idempotency before append |
| **`hugit intent list --log`** | `list::resolve_landed` | **✓ L-B** (was ✗ R8 hole — now FIXED) | landed-state projection |
| `hugit intent list --store` (no --log) | `IntentStore::load_existing` → `IntentStore::load` | ✓ (`IntentStore::load`) | store-resident intent list |
| `hugit intent show --store` | `IntentStore::load_existing` → `IntentStore::load` | ✓ (`IntentStore::load`) | full intent projection from store |
| `hugit export --log` (cut sub-path) | `Cut::take_to` | ✓ (`cut.rs:83`) | prefix chain for export corpus |
| `export::restore_from_bytes` | library-only (no CLI subcommand) | ✗ (does NOT call verify_chain) | `Restored` struct for tests/API callers |

**Notes on `export::restore_from_bytes`:** This function is NOT wired to any
CLI subcommand (confirmed by grep of `main.rs` and `src/`). It is only called
from acceptance tests. It does NOT project state to a user-facing CLI output
and is not a live read-path under the invariant. Schema validation
(`envelope.validate()`) and monotonic-seq checking (`push_record`) are applied
on the way back in; the missing `verify_chain` call is a hardening gap on an
internal-only utility, not a live hole. Tracked as a honesty note below.

**Matrix verdict: 20 ✓ / 0 ✗ among live CLI read paths**
(1 internal-only library utility skips verify_chain — not CLI-exposed, noted below)

---

## 3. `intent list` closed? (tamper repro before/after)

**Before L-B (R8 hole, reproduced for baseline):** Round 8 performed the live
repro. The acceptance test `tampered_log_is_chain_broken_exit_2_on_intent_list`
(file `acceptance_round8_readpath.rs`) now codifies the before/after as a
regression guard.

**After L-B (Round 9 verification):**

Environment: `hugit` binary built from `integ/wave-l` HEAD; temp dir
`/tmp/r9-class2-PWfDh2`.

1. `hugit campaign open --log $LOG --campaign camp-r9 --charter … --owner user:owner`
   → `{"opened":true}` exit 0.
2. `hugit intent new --charter … --campaign camp-r9 --id i-r9-001 --store $STORE --log $LOG`
   → `{"intent_id":"i-r9-001","already_exists":false}` exit 0.
3. `hugit intent list --store $STORE --log $LOG`
   → `{"intents":[{"id":"i-r9-001","landed":true}]}` exit 0. ← baseline confirmed.
4. Hand-tampered record[1] (`intent.landed`) in `$LOG`: mutated `charter` field
   in the payload JSON while leaving `this_hash` stale.
5. `hugit intent list --store $STORE --log $LOG`
   → `{"error":{"kind":"chain_broken","fix":"the --log file's hash chain is tampered or corrupt","message":"log … failed integrity verification: tamper: this_hash mismatch at seq 1 …"}}` exit 2. ← **CORRECT: hole is closed.**

Regression suite (`cargo test --test acceptance_round8_readpath --locked`):
`baseline_well_formed_log_projects_landed_true_exit_0` … ok
`tampered_log_is_chain_broken_exit_2_on_intent_list` … ok

---

## 4. Hunt for NEW uncovered read paths

### 4.1 `IntentStore::load` — NEW path (not in R8 matrix)

Wave L introduced `intent/store.rs` with `IntentStore::load` which deserialises
an embedded `[EventRecord, …]` event spine and calls `verify_chain`. This path
is exercised by both `hugit intent show` and `hugit intent list` (the
`--store`-only variant with no `--log`).

**Live tamper repro (store path):**

1. Wrote a tampered `intents.json` — mutated `charter` in the payload of
   the one `intent.landed` record while leaving `this_hash` stale.
2. `hugit intent show --intent i-r9-001 --store $STORE`
   → `{"error":{"kind":"store_error","message":"store chain failed verification: tamper: this_hash mismatch at seq 0 …"}}` exit 2. ← **CORRECT.**
3. `hugit intent list --store $STORE` (no --log)
   → `{"error":{"kind":"store_error","message":"store chain failed verification: tamper: this_hash mismatch at seq 0 …"}}` exit 2. ← **CORRECT.**

`IntentStore::load` is CLOSED. The R8 report's note that `intent show`
"reads from `IntentStore` … out of scope for the verify_chain invariant" is
NOW SUPERSEDED: `IntentStore` was added in Wave L with verify_chain baked in;
both `intent show` and `intent list` (store path) are covered.

### 4.2 `export::restore_from_bytes` — library utility, not CLI-exposed

`export::restore_from_bytes` rebuilds an `EventLog` from an `ExportEnvelope`
without calling `verify_chain`. It is NOT wired to any CLI subcommand and is
only called from acceptance tests (`acceptance_e5.rs`). The monotonic-seq
invariant (`push_record`) is enforced; the hash-chain integrity is not
re-verified. This is a hardening gap in a library utility, not a live
user-facing hole. The `Restored` struct is consumed only within test assertions.

**Assessment:** NOT a DO-NOT-SHIP finding. The invariant (every CLI read path
verifies) is satisfied. Noted for the PS-13 chokepoint refactor: if a future
CLI verb calls `restore_from_bytes` and projects its `event_log` as
authoritative, that verb must add `verify_chain` — the per-site pattern is
the persistent structural risk.

### 4.3 `export/mod.rs` `cut_log` (line 206)

Built from a `Cut` object whose constructor (`Cut::take_to`) calls
`verify_chain` at line 83 before returning. The `cut_log` is populated from
already-verified records; no second verify needed. Not a gap.

### 4.4 All other `EventLog::new()` sites in `pr/mod.rs`, `verdict/mod.rs`, `campaign/seal_guard.rs`

All confirmed `#[cfg(test)]` blocks — test harnesses building fresh logs, not
reading from disk. Not in scope.

### 4.5 Conclusion of hunt

No SIXTH uncovered live CLI read path was found. All 20 live read paths
(canonical log + store spine) call `verify_chain` before projecting state.

---

## 5. PS-8 AC-4 honesty

`docs/plan/2026-06-11-pending-seams.md` §PS-8 §4 (line 273–293) now
correctly reflects the Round-8 correction: it records that the "every-read"
claim was false at R8 (`intent list` skipped `verify_chain`), names Wave L /
L-B as the fix, and tracks the structural residual (five ad-hoc loaders, not
one chokepoint) as PS-13. The honesty claim is TRUE as of Wave L HEAD for all
five loaders.

The `export::restore_from_bytes` gap (§4.2 above) is an omission in the
PS-8 AC-4 note — it correctly claims "five loaders all verify" but does not
mention the test-only utility. This is a minor imprecision in documentation,
not a false claim about live behaviour.

---

## 6. CONVERGENCE VERDICT

**CONVERGED.**

Every live CLI read path that deserialises the event log and projects
authoritative state calls `verify_chain` before projecting. The R8 hole
(`intent list --log` / `resolve_landed`) is closed by Wave L / L-B and
confirmed by live tamper repro and the `acceptance_round8_readpath` suite.
The newly enumerated `IntentStore` read path (Wave L addition) also verifies
correctly — confirmed by live store-tamper repro.

The only open item is PS-13: `verify_chain` is still called ad-hoc per loader
(five loaders + the `resolve_landed` fix = six call sites) rather than through
a single mandatory `load_verified_log` chokepoint. This is defence-in-depth
bookkeeping, not a live hole — all six sites verify correctly today. A future
read verb that forgets to call `verify_chain` would re-open the class, which
is why PS-13 remains open.

| Item | Status |
|---|---|
| R8 hole: `intent list --log` skips verify | CLOSED (L-B; live repro + regression test) |
| `IntentStore` store spine verify | PRESENT (live repro confirms exit-2 on tamper) |
| PS-8 AC-4 honesty claim | TRUE for all five named loaders (minor doc omission for test-only `restore_from_bytes`) |
| PS-13 structural residual (chokepoint refactor) | OPEN — defence-in-depth, not a live hole |
| Any NEW uncovered live CLI read path | NONE FOUND |
