# CLASS 2 — READ-PATH INTEGRITY — Round 10 confirmation re-audit

**Round 10 — 2026-06-12**
**Auditor:** Fresh-context adversarial agent (no carry-forward from Round 9)
**Scope:** Confirmation re-audit after WP **M-1** introduced a single
verified-loader chokepoint (PS-13) and WP **M-3** routed the intent-`new`
reconcile path through it. Branch `integ/wave-m`, HEAD `e85bcf5`
(`fix(readpath): route M-3 reconcile loader through the M-1 chokepoint`).

---

## 1. Scope & method

**Invariant:** Every read path that deserialises a hash-chained event log and
projects authoritative state MUST run `hugit_refstore::verify_chain` before
projecting. A tampered/reordered log must yield a structured
`chain_broken`/exit-2 (or the store's `store_error`/exit-2) error, never a
silent authoritative projection.

**What M-1/M-3 changed (verified against source):**
- The sole production site of the `EventLog::new() + push_record + verify_chain`
  rehydrate-and-verify loop for a canonical `[EventRecord, …]` disk log is now
  `checks::rehydrate_and_verify` (`checks/mod.rs:411`). `load_event_log`
  (`checks/mod.rs:439`) delegates to it.
- The four canonical disk loaders route through it:
  `intent/list.rs:185`, `intent/canonical_log.rs:255`, `pr/cli.rs:397`,
  `campaign/world.rs:610`.
- **M-3:** `intent/new.rs::reconcile_store_from_log` (`new.rs:351`) loads its
  `--log` via the file-local `load_log_verified` (`new.rs:434`) which now
  routes through `checks::rehydrate_and_verify` (`new.rs:458`) — confirmed,
  no inline `verify_chain`/`push_record` remains in `new.rs`.
- A source-invariant test
  (`acceptance_wave_m_readpath.rs::canonical_log_loaders_route_through_the_chokepoint`)
  fails the build if `list.rs`/`canonical_log.rs`/`pr/cli.rs`/`world.rs`
  hand-roll `verify_chain(`/`.push_record(`.
- `export/cut.rs:91` re-verifies an already-chokepoint-loaded in-memory prefix
  (tagged `readpath-verify-exempt`).
- `intent/store.rs` reads a DIFFERENT on-disk shape (`IntentStoreFile`, its own
  embedded spine) and runs its OWN `verify_chain` (`store.rs:205`) — a
  documented safe sibling, out of the canonical-loader set.

**Method:**
1. Re-enumerated EVERY production deserialise-then-project site (grep all
   `from_slice`/`from_str` into `Vec<EventRecord>`/`IntentStoreFile`/
   `ExportEnvelope`, all `push_record`/`verify_chain`/`rehydrate_and_verify`/
   `load_event_log`/`load_log_verified` callers, excluding `#[cfg(test)]`).
2. Built the real `hugit` binary (`target/debug/hugit`, toolchain 1.96.0).
3. Live tamper repro per read verb in temp dirs OUTSIDE the repo
   (`/tmp/r10-*`, since removed): seed a full log
   (`campaign open`→`intent new`→`pr open`), then mutate ONE payload byte
   leaving `this_hash` stale (JSON stays valid → reaches verify, not parse).
4. Specifically attacked the M-3 reconcile: tampered `--log` + store missing
   the log's intent → must fail closed, never replay the tampered
   `intent.landed` into the store.
5. Ran the Wave-M source-invariant + tamper suite
   (`cargo test -p hugit-cli --test acceptance_wave_m_readpath --locked`).
6. Enumerated the full CLI verb surface (`main.rs:54` `enum Command`) to
   confirm no NEW read verb / restore / import bypasses the chokepoint.

---

## 2. Matrix (every read path × chokepoint/verify × projection)

| Read path | Loader → verify site | Routes chokepoint? | Verifies? | Projects |
|---|---|---|---|---|
| `checks show --log` | `load_event_log` → `rehydrate_and_verify` | ✓ | ✓ | check rows, KPIs |
| `queue show --log` | `queue` → `load_event_log` | ✓ | ✓ | queue/batches |
| `check --store --log` | `run::record_on_log` → `load_event_log` | ✓ | ✓ | dedup scan |
| `verdict --store --log` | `verdict::record` → `load_event_log` | ✓ | ✓ | intent existence |
| `tournament --log` | `run_tournament` → `load_event_log` | ✓ | ✓ | intent existence |
| `export --log` | `run_export` → `load_event_log` | ✓ | ✓ | full export corpus |
| `export --log` (cut prefix) | `Cut::take_to` re-verify (exempt) | n/a (in-mem) | ✓ | export prefix |
| `pr show/list/land/open/abandon --log` | `pr::cli::load_log` → `rehydrate_and_verify` | ✓ | ✓ | PR rows, queue |
| `campaign show/list/open/close/abandon --log` | `world::load_canonical_log` → `rehydrate_and_verify` | ✓ | ✓ | phases, ledger, rollup |
| `intent new --log` (idempotency) | `canonical_log::load` → `rehydrate_and_verify` | ✓ | ✓ | idempotency pre-append |
| `intent new --log` (**M-3 reconcile**) | `reconcile_store_from_log` → `load_log_verified` → `rehydrate_and_verify` | ✓ | ✓ | store heal from log |
| `intent list --log` | `list::resolve_landed` → `rehydrate_and_verify` | ✓ | ✓ | landed-state |
| `intent list --store` (no `--log`) | `IntentStore::load` (own spine) | safe sibling | ✓ | store intent list |
| `intent show --store` | `IntentStore::load` (own spine) | safe sibling | ✓ | full intent projection |
| `why --log` | `read_log` (own WHY shape) + inline `verify_chain` (main.rs:159) | inline sibling | ✓ | provenance |
| `export::restore_from_bytes` | library-only, NOT CLI-wired | n/a | ✗ | `Restored` (tests/API) |
| `impact --graph` | `GraphInput` (NOT an event log) | n/a | n/a | blast radius |

**Counts among live read paths that project a hash-chained log:**
**18 ✓ verify / 0 ✗** (4 canonical loaders + `load_event_log` family of 6 verbs
+ M-3 reconcile + `intent list --log` + the 2 `IntentStore` store-spine reads +
`why`'s inline-verify sibling). One library-only utility
(`restore_from_bytes`) skips verify but is NOT CLI-reachable (no `Restore`
verb in `enum Command`); `impact` reads a non-log shape.

---

## 3. Tamper repro per verb (incl. reconcile)

Binary `target/debug/hugit`; temp dirs OUTSIDE the repo (removed after).
Tamper = mutate one payload byte, `this_hash` left stale, JSON still valid.

**Canonical-log verbs (one shared tampered `--log`):**

| Verb | Baseline (clean) | Tampered |
|---|---|---|
| `intent list --store --log` | `landed:true` exit 0 | `chain_broken` exit 2 |
| `pr show --log` | `pr_id:"1"` exit 0 | `chain_broken` exit 2 |
| `pr list --log` | rows exit 0 | `chain_broken` exit 2 |
| `campaign show --log` | progress exit 0 | `chain_broken` exit 2 |
| `campaign list --log` | exit 0 | `chain_broken` exit 2 |
| `queue show --log` | exit 0 | `chain_broken` exit 2 |
| `checks show --log` | exit 0 | `chain_broken` exit 2 |
| `export --log` | `exported.git_dir` exit 0 | `chain_broken` exit 2 |
| `tournament --log` | exit 0 | `chain_broken` exit 2 |
| `why --log` | n/a | `chain_broken`/`parse_log` exit 2 (own shape) |

Every tampered read returned the structured `chain_broken` envelope (`error.kind
== "chain_broken"`, `error.fix` present, no flat projection key), exit 2 — never
a projected tampered state. Sample:
`{"error":{"kind":"chain_broken","fix":"the --log file's hash chain is tampered
or corrupt","message":"… this_hash mismatch at seq 1 …"}}`.

**Store-spine (`intent/store.rs`, tampered `intents.json`):**
- `intent list --store` (no `--log`) → `store_error` exit 2.
- `intent show --store --intent i-S` → `store_error` exit 2.
Message: `store chain failed verification: tamper: this_hash mismatch at seq 0`.

**M-3 reconcile attack (the headline target):**
1. Seeded `$LOG` with `i-A.landed` against a throwaway seed-store; `$STORE`
   absent (fresh).
2. Tampered `$LOG` (broke `i-A`'s payload byte, `this_hash` stale).
3. `intent new --log $LOG --store $STORE --id i-B` (fresh store missing `i-A`
   → triggers `reconcile_store_from_log`):
   → `{"error":{"kind":"chain_broken","message":"--log … failed integrity
   verification: tamper: this_hash mismatch at seq 1 …"}}` **exit 2.**
   `$STORE` was **NOT written** — no tampered `i-A` replayed, no `i-B` appended.
   **Fail-closed CONFIRMED.**
4. Liveness control: same scenario with a CLEAN `$LOG` → `intent new i-B`
   succeeds, `intent list --store` shows BOTH `i-A` (healed in from log) AND
   `i-B`. The reconcile path is genuinely exercised, not dead code — so the
   fail-closed result above is a real guard, not an inert branch.

The reconcile's load goes through the chokepoint: `load_log_verified`
(`new.rs:434`) → `checks::rehydrate_and_verify` (`new.rs:458`); verified by
source read and by the fact the tamper surfaces the SAME chokepoint
`chain_broken` message before any store write.

**Source-invariant + tamper suite:**
`cargo test -p hugit-cli --test acceptance_wave_m_readpath --locked` →
6 passed / 0 failed (`canonical_log_loaders_route_through_the_chokepoint`,
`the_chokepoint_function_exists_in_checks`, and the four valid-then-tamper
verb tests).

---

## 4. Findings

No DO-NOT-SHIP finding. Every live CLI read path that projects a hash-chained
log fails closed on tamper. The bypass hunt surfaced three structural notes
(all SAFE today, all PS-13 defence-in-depth residuals, none a live hole):

**F-1 (residual, low):** Two inline-verify siblings exist OUTSIDE the chokepoint
and OUTSIDE the source-invariant test's `converged` set:
- `why` (`main.rs:159`) reads its own wrapper shape (`WhyLogEntryInput`) and
  calls `verify_chain` inline. It verifies today (live repro: tamper → exit 2),
  but `main.rs` is not policed by the source-invariant test, so a future edit
  here that dropped the verify would NOT break the build.
- `intent/new.rs::load_log_verified` (the M-3 reconcile loader) DOES route
  through the chokepoint correctly (no inline verify), but `new.rs` is also NOT
  in the test's `converged` array (which lists only `list.rs`,
  `canonical_log.rs`, `pr/cli.rs`, `world.rs`). If a future edit re-introduced
  an inline `verify_chain`/`push_record` in `new.rs`, the source invariant
  would not catch it. The runtime tamper guard would still hold, but the
  structural backstop is incomplete for this file.
  **Recommendation (non-blocking):** add `intent/new.rs` to the source-invariant
  `converged` set (and document `main.rs::why` as an explicit inline-verify
  sibling) so the M-3 path is structurally policed, not just convention-policed.

**F-2 (residual, carried from R9 §4.2, unchanged):**
`export::restore_from_bytes` (`export/mod.rs:630`) rebuilds an `EventLog` via
`push_record` (monotonic-seq enforced) + schema `validate()` but does NOT call
`verify_chain`. Confirmed still NOT wired to any CLI subcommand — there is no
`Restore`/`Import` verb in `enum Command` (`main.rs:54`); the only caller is the
library `restore()` and acceptance tests. Not a live hole; a hardening gap on an
internal utility. If a future verb ever projects `restore_from_bytes`' output as
authoritative, it must add `verify_chain`.

**F-3 (informational):** `intent/store.rs` is the documented safe sibling — a
different on-disk shape (`IntentStoreFile`) with its own `verify_chain`
(`store.rs:205`); deliberately and correctly NOT in the canonical-loader set.
Store-spine tamper fails closed (`store_error` exit 2, repro §3).

The chokepoint is genuinely single FOR THE CANONICAL `[EventRecord, …]` SHAPE:
all four canonical disk loaders + the M-3 reconcile + the `load_event_log`
family route through `checks::rehydrate_and_verify`. The remaining verifiers
(`why` inline, `store.rs` own-shape, `cut.rs` exempt in-mem re-verify) read
DIFFERENT shapes or already-loaded in-memory logs and each verify correctly.

---

## 5. CONVERGENCE VERDICT

**CONVERGED.**

Every read path that deserialises a hash-chained log and projects authoritative
state verifies before projecting — confirmed by source read AND by live tamper
repro of every read verb (`intent list`, `pr show/list`, `campaign show/list`,
`queue show`, `checks show`, `export`, `tournament`, `why`, plus the
`--store`-only `intent list`/`intent show`). All return `chain_broken`/
`store_error` exit-2 on a tampered chain, never a tampered projection.

The M-3 reconcile (`reconcile_store_from_log`) — the new attack surface this
round — **fails closed** on a tampered `--log` (live repro: `chain_broken`
exit 2, no store written, no tampered `intent.landed` replayed) and routes its
load through the M-1 chokepoint; a clean-log control proves the path is live,
not inert. The canonical chokepoint is genuinely single for the
`[EventRecord, …]` shape; the source-invariant test is load-bearing (build fails
on a hand-rolled loop in the four converged files).

| Item | Status |
|---|---|
| Canonical chokepoint single for `[EventRecord, …]` | YES (4 loaders + `load_event_log` family + M-3 all route through it) |
| M-3 reconcile fail-closed on tampered log | CONFIRMED (live repro: chain_broken exit 2, no store write; clean-log control heals) |
| Every read verb tamper → chain_broken/exit-2 | YES (live repro, all verbs incl. store-spine + why) |
| Source-invariant load-bearing | YES (6/6 suite green; build fails on hand-rolled loop) |
| Any NEW read path bypassing the chokepoint | NONE live (no Restore/Import verb; `impact` reads non-log) |
| Residual PS-13 (defence-in-depth, NOT a live hole) | `new.rs` + `main.rs::why` outside source-invariant set (F-1); `restore_from_bytes` no-verify but not CLI-wired (F-2) |
