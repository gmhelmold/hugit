# CLASS 2 — READ-PATH INTEGRITY — SOTA audit

**Round 8 — 2026-06-12**  
**Auditor:** Fresh-context adversarial agent (no carry-forward from Round 7)  
**Scope:** Every code path that reads/loads/projects the hash-chained event-log and whether it calls `verify_chain` before projecting as authoritative.

---

## 1. Scope & method

**Invariant under audit:** Every code path that reads or projects the event-log hash chain MUST call `hugit_refstore::verify_chain` before projecting log content as authoritative state. A tampered or reordered log must yield `chain_broken`/exit-2, never an authoritative projection.

**Method:**
1. Located every function that deserialises/loads/reads `EventLog` from disk across the `crates/hugit-cli` tree — the only layer that reads on-disk logs from the `--log` seam.
2. Traced each call site to determine whether `verify_chain` is invoked on the loaded records before any projection is emitted.
3. Performed a live tamper-repro for the discovered hole (`intent list`): built the binary, created a valid log, hand-tampered an `intent.landed` payload (breaking the chain hash), ran `hugit intent list --store ... --log ...`, confirmed exit-0 and "landed":true returned from a tampered log.
4. Cross-checked the PS-8 honesty claim in `docs/plan/2026-06-11-pending-seams.md` §AC-4 (line 273) against findings.

**Files examined:**
- `crates/hugit-cli/src/main.rs` — `run_why`, `run_export`, `run_tournament`
- `crates/hugit-cli/src/checks/mod.rs` — `load_event_log` (the canonical shared loader), `show`
- `crates/hugit-cli/src/checks/run.rs` — `record_on_log`
- `crates/hugit-cli/src/pr/cli.rs` — `load_log`, `load_log_or_empty`, `run_show`, `run_list`, `run_land`, `run_abandon`
- `crates/hugit-cli/src/campaign/world.rs` — `load_canonical_log`, `World::load`, `World::load_existing`
- `crates/hugit-cli/src/campaign/show.rs` — `run` (calls `World::load_existing`)
- `crates/hugit-cli/src/campaign/list.rs` — `run` (calls `World::load_existing`)
- `crates/hugit-cli/src/intent/canonical_log.rs` — `load` (used by `land_intent`)
- `crates/hugit-cli/src/intent/list.rs` — `resolve_landed` ← **THE HOLE**
- `crates/hugit-cli/src/intent/show.rs` — `run` (reads from `IntentStore`, not the canonical log)
- `crates/hugit-cli/src/queue/mod.rs` — `show` (calls `load_event_log`)
- `crates/hugit-cli/src/verdict/mod.rs` — `record` (calls `load_event_log`)

---

## 2. Complete inventory — THE matrix

| Read path | Entry point | Calls verify_chain? | What it projects as authoritative |
|---|---|---|---|
| `hugit why --log` | `run_why` → `read_log` + inline verify | ✓ K-CHAIN | intent provenance from log |
| `hugit export --log` | `run_export` → `checks::load_event_log` | ✓ K-CHAIN (via loader) | full export corpus from log |
| `hugit tournament --log` | `run_tournament` → `checks::load_event_log` | ✓ (via loader) | intent existence check |
| `hugit checks show --log` | `show` → `checks::load_event_log` | ✓ (loader §WF-3) | check rows, KPIs |
| `hugit check --store --log` | `record_on_log` → `checks::load_event_log` | ✓ (via loader) | dedup scan before append |
| `hugit queue show --log` | `queue::show` → `checks::load_event_log` | ✓ (via loader) | queue entries, batches |
| `hugit verdict --store --log` | `verdict::record` → `checks::load_event_log` | ✓ (via loader) | intent existence, verdict dedup |
| `hugit pr show --log` | `run_show` → `load_log` | ✓ (pr::cli `load_log`) | PR intents, queue state, rollup |
| `hugit pr list --log` | `run_list` → `load_log` | ✓ (pr::cli `load_log`) | all PR rows |
| `hugit pr land --log` | `run_land` → `load_log` | ✓ (pr::cli `load_log`) | queue projection |
| `hugit pr open --log` | `run_open` → `load_log_or_empty` → `load_log` | ✓ (via `load_log`) | intent existence, campaign check |
| `hugit pr abandon --log` | `run_abandon` → `load_log` | ✓ (pr::cli `load_log`) | PR open record |
| `hugit campaign show --log` | `World::load_existing` → `load_canonical_log` | ✓ (`load_canonical_log`) | PR phases, ledger, rollup |
| `hugit campaign list --log` | `World::load_existing` → `load_canonical_log` | ✓ (`load_canonical_log`) | campaign keys, progress |
| `hugit campaign open --log` | `World::lock_and_load` → `load_canonical_log` | ✓ (`load_canonical_log`) | bootstrap / idempotency |
| `hugit campaign close/abandon --log` | `World::lock_and_load(bootstrap=false)` → `load_canonical_log` | ✓ (`load_canonical_log`) | seal condition |
| `hugit intent new --log` (canonical_log) | `land_intent` → `canonical_log::load` | ✓ (`canonical_log::load`) | idempotency before append |
| **`hugit intent list --log`** | `list::resolve_landed` | **✗ HOLE** | **landed-state projection for all intents — projects tampered log as authoritative** |

**Matrix verdict: 17 ✓ / 1 ✗ (one hole confirmed by live tamper-repro)**

---

## 3. Findings

### F-1 — `hugit intent list --log` skips verify_chain (CONFIRMED CODE DEFECT, HIGH)

**Severity:** HIGH — a tampered log is projected as authoritative landed-state for every intent in the store.

**Location:** `crates/hugit-cli/src/intent/list.rs`, function `resolve_landed` (lines ~145–186).

**Root cause (code):** `resolve_landed` deserialises `EventRecord`s from disk, pushes them into an `EventLog` via `push_record`, then calls `intents_from_log` and returns the landed id set — but NEVER calls `hugit_refstore::verify_chain` on the records before projecting. The PR/campaign/checks/queue loaders all call `verify_chain`; this one does not.

**What it projects as authoritative:** The `landed` field on each `IntentListItem` in the `{"intents":[…]}` output. An agent that uses `intent list --log` to confirm an intent is landed (or not) before acting receives an unchecked assertion.

**TYPE:** CODE (missing call — structurally identical to the `why`/`export` gap K-CHAIN fixed in Wave K).

**Live tamper repro (2026-06-12):**
1. Created a valid campaign + intent on a shared canonical log.
2. `hugit intent list --store S --log L` → `{"intents":[{"landed":true,…}]}` exit 0. ✓
3. Hand-tampered the `intent.landed` payload's `charter` field (breaking `this_hash`).
4. `hugit intent list --store S --log L` → `{"intents":[{"landed":true,…}]}` exit 0. ← **WRONG — tampered log projected as authoritative, no `chain_broken` error.**

---

### F-2 — PS-8 AC-4 honesty claim is FALSE (HONESTY GAP, MEDIUM)

**Severity:** MEDIUM — a documented claim is incorrect.

**Location:** `docs/plan/2026-06-11-pending-seams.md` lines 273–280 (PS-8 AC-4).

**Claim (verbatim, line 273):** "verify_chain continues to run on every read for partial-corruption detection … Both now verify the chain and return `chain_broken` exit-2 on corruption; the every-read claim is true again."

**Reality:** `hugit intent list --log` does NOT verify the chain. The "every-read" claim is therefore FALSE as of Wave K HEAD (`def8a18`). The K-CHAIN fix correctly remediated `why` and `export` but did not audit `intent list`, leaving one read path uncovered and the PS-8 AC-4 claim over-stated.

**TYPE:** HONESTY (documented claim diverges from code reality — the same class as the Round-5 PS-8 original finding).

---

## 4. Root-cause analysis — the ONE structural reason

**Root cause:** `verify_chain` is called ad-hoc at every individual load site, not enforced by the type system or a single mandatory entry point. There is no single `load_verified_log()` function that all read paths MUST go through — several sites call `checks::load_event_log` (which verifies), others call their own local loaders (`campaign::world::load_canonical_log`, `pr::cli::load_log`, `intent::canonical_log::load`, `run_why`'s inline block, and now the defective `intent::list::resolve_landed`). Each loader independently re-implements the verify step.

The result is that every new or changed read path is a latent gap: if its author forgets to call `verify_chain` — as happened with `why`/`export` (fixed by K-CHAIN) and now `intent list` — there is no compiler or API-level guarantee to catch the omission before it ships. The K-CHAIN wave fixed the two then-known gaps but could not prevent future ones because the root structure (ad-hoc per-site verify) was not changed.

---

## 5. Recommended structural remediation — the single-choke-point refactor

**The fix that makes skipping structurally impossible:**

### 5.1 Introduce `load_verified_log(path)` as the ONE public entry point

In `crates/hugit-cli/src/checks/mod.rs`, `load_event_log` already does the right thing: read → push_record → verify_chain → return EventLog. Rename it `load_verified_log` (or keep the name and make it the single canonical entry point) and:

1. **Make the raw deserialise-without-verify helper `pub(crate)` or private** — `resolve_landed` in `intent::list` currently re-implements the load pattern from scratch rather than calling `load_event_log`. Once it and all other sites are migrated to the one function, the raw pattern has no legitimate callers outside the function itself.

2. **Migrate every read site to `load_verified_log`:**
   - `intent::list::resolve_landed` — **the confirmed hole** — replace the custom load with `checks::load_event_log` (already public, already correct). One-line fix.
   - `campaign::world::load_canonical_log`, `pr::cli::load_log`, `intent::canonical_log::load`, and `run_why`'s inline block — already correct but should be unified to the single function to remove the duplicated pattern (reduces future risk).

3. **Remove the raw `EventLog::new() + push_record + …` pattern from every read path** — make it only reachable via `load_verified_log`. The only callers of `EventLog::new()` should be write paths (initial bootstrap for `campaign open`, `pr open`, `intent new`) and tests.

### 5.2 Exact function names

```
// The ONE public read entry point (already exists in checks::mod.rs):
pub fn load_event_log(path: &Path) -> Result<EventLog, PorcelainError>  // already verifies

// Fix for the hole (intent/list.rs resolve_landed — replace current ~20-line body):
fn resolve_landed(log_path: &Path) -> Result<HashSet<String>, PorcelainError> {
    if !log_path.exists() {
        return Ok(HashSet::new());  // missing log → no intents landed
    }
    let log = crate::checks::load_event_log(log_path)?;  // ONE LINE — verifies chain
    Ok(intents_from_log(&log)
        .map(|il| il.intents().iter().map(|i| i.intent_id.clone()).collect())
        .unwrap_or_default())
}
```

**Estimated change:** 3 lines net in `intent/list.rs` (delete ~18 lines of raw load, replace with `load_event_log` call + missing-file guard). The structural refactor to unify all sites is a larger cleanup but is not required to close the confirmed hole.

---

## 6. Residual / accepted

| Item | Disposition |
|---|---|
| PS-8 AC-4 honesty claim | Must be corrected in `docs/plan/2026-06-11-pending-seams.md` after the fix lands — add `intent list` to the correction note (currently only names `why`/`export`). |
| Structural unification of all load sites | Recommended but not required for the hole fix. The single confirmed code defect is `resolve_landed`. The other load sites (campaign/pr/canonical_log) are independently correct. |
| Tests | The existing `tampered_chain_is_chain_broken_exit_two_on_{why,export}` pattern should be extended to `intent list` to close the regression surface. |
| `hugit intent show` | Reads from `IntentStore` (a separate `.hugit/intents.json` side-document), NOT from the canonical `[EventRecord,…]` log. No hash chain governs the store file; this is a distinct seam and is out of scope for the verify_chain invariant. Not a gap. |
