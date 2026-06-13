# Performance review — hugit engine (sweep-2026-06-12)

**Reviewer role:** performance. **Baseline:** `main` HEAD `5457730` (Wave M).
**Scope:** read-only on code; measurements via a throwaway probe (deleted) +
the release `hugit` binary over synthetic hash-chained logs.

## Scope & method

- Read the hot-path code: the read-path chokepoint (`checks::rehydrate_and_verify`
  → `verify_chain`), the reconcile (`reconcile_store_from_log`), the memo wedge
  (`snapshot_tree`/`collect_files`), and the scrub/redaction primitives
  (`secret_shape.rs`, `redact.rs`, `porcelain.rs`).
- **Measured** `verify_chain` and `intents_from_log` scaling with a throwaway
  release-mode test (`hugit-refstore`, `test-support`, synthetic logs of
  100–10 000 records, ~1/3 `intent.landed`, warmed, 50 reps avg). Probe deleted
  after measuring.
- **Measured** real end-to-end CLI read latency with the release binary
  (`hugit checks show --log <log>`), 10–20 runs/size, vs. an empty-log baseline
  to subtract process startup.
- Reasoned about the scrub/snapshot paths where a clean micro-harness wasn't
  worth the build cost; flagged allocation patterns by inspection.

## The hot-path map

| Path | Trigger | Code | Per-call cost |
|---|---|---|---|
| **Read chain-verify** | EVERY `--log` read verb (`checks`/`queue`/`campaign show`/`pr show`/`why`/`export`/`verdict`/`intent list`) | `checks::rehydrate_and_verify` → `hugit_refstore::verify_chain` (`tamper/mod.rs:98`) | O(n) SHA-256 over the WHOLE chain |
| **Store load chain-verify** | every `intent new`/`intent show`/`intent list --store` | `IntentStore::load` → `verify_chain` (`intent/store.rs:205`) | O(n) |
| **Reconcile** | START of every `intent new --log` | `reconcile_store_from_log` (`intent/new.rs:351`) | **O(I·n)** — see F2 |
| **Intent projection** | inside every `intent_for` / list | `intents_from_log` (`intent/model/mod.rs:151`) | O(n) JSON re-parse |
| **Memo snapshot** | every `hugit check` | `snapshot_tree`/`collect_files` (`checks/run.rs:348`) | O(files) read+hash (correct, see "fine") |
| **Scrub** | per field per WRITE event | `structural_secret_scrub`/`is_safe_identifier_shape` (`secret_shape.rs`) | O(L) per field; allocs (F4) |

---

## Findings

### F1 — `verify_chain` re-hashes the entire append-only log on EVERY read (P1, scales-badly)

- **id:** PERF-F1
- **severity:** **P1** (user-visible-slow at campaign scale; today's logs are small)
- **where:** `crates/hugit-refstore/src/tamper/mod.rs:98` (`verify_chain`),
  driven by `crates/hugit-cli/src/checks/mod.rs:411` (`rehydrate_and_verify`) and
  every read verb routed through `load_event_log` / `rehydrate_and_verify`
  (`checks/mod.rs:439`, `intent/list.rs:185`, `pr/cli.rs:397`,
  `campaign/world.rs:610`, `intent/canonical_log.rs:255`, `queue/mod.rs:98`,
  `verdict/mod.rs:258`, `export/cut.rs`).
- **cost — MEASURED:** `verify_chain` is **O(n)** but with a real per-record
  SHA-256 + allocation constant that grows under cache pressure:

  | n records | verify_chain ms/call | µs/record |
  |---|---|---|
  | 100 | 0.40 | 3.95 |
  | 1 000 | 4.41 | 4.41 |
  | 5 000 | 29.6 | 5.93 |
  | 10 000 | 103.1 | 10.3 |

  End-to-end `hugit checks show --log` (release binary, process startup ~12.5 ms
  subtracted): n=1000 → ~8 ms chain-verify, n=5000 → ~36 ms, **n=10 000 → ~73 ms
  of pure re-verification per single read.** A campaign log is the shared
  forever-append `--log`; an orchestrated fleet doing thousands of events makes
  every `checks show`/`pr show`/`campaign show`/`why` re-hash the full history.
  The chain only GROWS, yet the prefix `[0..k]` was already verified on the last
  read and is byte-identical — the work is almost entirely redundant.
- **why it's a per-call constant, not super-linear:** the µs/record creep
  (4→10 µs as n grows) is allocation/cache pressure, not algorithmic — note
  `verify_chain` does **two `String` clones per record** (`prev_hash.clone()` at
  `tamper/mod.rs:138` for the rolling `prev_this`, plus the error-path clones)
  and `compute_this_hash` builds a fresh `Vec<u8>` pre-image + `hex::encode`
  allocation per record (`log/mod.rs:143,159`). At 10k records that's 10k
  short-lived `Vec`s + 10k 64-byte hex strings per read.
- **the fix (ranked):**
  1. **Verify the SUFFIX, trust the verified prefix.** The chain is append-only;
     a read does not need to re-verify history it verified last time. Cache the
     last-verified `(len, head_hash)` (e.g. an mtime+len-keyed memo, or a
     `.hugit/<log>.verified` sidecar holding `len`+`head_hash`) and only
     re-hash `records[verified_len..]`. Turns the per-read cost from O(n) to
     O(Δ). This is the single highest-leverage fix.
  2. **Avoid the per-record `prev_this` String clone:** carry `&str` of the
     predecessor's `this_hash` instead of cloning into a fresh `String` each
     iteration (`tamper/mod.rs:138`). Pure constant-factor win, no semantic change.
  3. `compute_this_hash` could feed SHA-256 incrementally (`Sha256::new()` +
     `update` per field) instead of building one `Vec<u8>` pre-image, dropping
     the per-record `Vec` alloc (`log/mod.rs:143`).

### F2 — `reconcile_store_from_log` is O(I·n): seconds at campaign scale (P0/P1)

- **id:** PERF-F2 — **the worst scaling problem in the engine.**
- **severity:** **P0** at a few thousand intents (multi-second `intent new`),
  **P1** today (small logs).
- **where:** `crates/hugit-cli/src/intent/new.rs:351` (`reconcile_store_from_log`),
  the inner `store.intent_for(&intent.intent_id)` at line 376.
- **cost — MEASURED + reasoned:** the loop iterates every intent on the `--log`
  (`I` intents). For each one it calls `store.intent_for()`
  (`intent/store.rs:239`), which calls `intents_from_log` over the **whole store
  log** (`intent/model/mod.rs:151`) — an O(n) JSON re-parse of every
  `intent.landed` payload — then a linear `by_id` scan (`model/mod.rs:120`).
  So the reconcile is **O(I) × O(n) = O(I·n)**, and since `I ≈ n/k` it is
  **O(n²)**. Worse, this runs at the START of **every** `intent new --log`, so
  over a campaign's lifetime the authoring cost is **O(n³)** cumulative.

  Measured (release, the inner `I × intents_from_log + by_id` loop):

  | store n (intents I) | reconcile inner-loop |
  |---|---|
  | 100 (34) | 3.7 ms |
  | 500 (167) | 157 ms |
  | 1 000 (334) | 866 ms |
  | 2 000 (667) | **3.64 s** |
  | 5 000 (1667) | **18.2 s** |

  At 5 000 events a single `intent new --log` pays ~18 s of pure reconcile
  before it does any work — clean quadratic (5× the log ⇒ ~50× the time). On a
  log already in sync (the common case) this is **entirely redundant**: nothing
  is missing, yet the full O(n²) scan runs every time.
- **the fix (ranked):**
  1. **Project the store log ONCE before the loop**, not per-iteration. Hoist
     `let store_intents = intents_from_log(&store.log)?;` out and build a
     `HashSet<&str>`/`HashMap` of store intent-ids; the per-log-intent check
     becomes O(1). This alone collapses O(I·n) → O(n) (the dominant fix).
  2. **Fast-path the in-sync case:** compare `store.log.len()` /
     `head_hash` against the `--log` and skip the whole reconcile when the
     store is already ≥ the log (no divergence possible). Makes the common case
     O(1).
  3. `by_id` linear scan (`model/mod.rs:120`) → back the `IntentLog` with a
     `HashMap<String,usize>` index built once at projection time (also helps F1's
     `intent_for`).

### F3 — `intents_from_log` re-parses every payload on every projection (P1)

- **id:** PERF-F3
- **severity:** **P1** (the multiplier behind F2; also a standalone read cost)
- **where:** `crates/hugit-refstore/src/intent/model/mod.rs:151,157` +
  `parse_intent` (`:168`).
- **cost — MEASURED:** O(n) but heavy: `serde_json::from_str` of each
  `intent.landed` payload + 4 `field_str` lookups + `.to_string()` on each of
  intent_id/ref/target/charter + `principal_chain.clone()` per record
  (`model/mod.rs:170–182`). Measured 36 ms at n=10 000 (~⅓ intents). It is
  re-run from scratch on **every** `intent_for` call and every list — and
  `intent_for` itself is called repeatedly inside F2.
- **the fix:** memoize the projection on the loaded `IntentStore` (project once
  after load, cache the `IntentLog`), and index `by_id` (see F2.3). The
  `.to_string()`/`.clone()` per field are unavoidable for an owned `Intent` but
  only need to happen once per load, not per query.

### F4 — scrub allocates redundantly per field on the write path (P2)

- **id:** PERF-F4
- **severity:** **P2** (write path, ~handful of fields per event — minor today,
  but pure waste)
- **where:** `crates/hugit-ledger/src/secret_shape.rs:177`
  (`has_keyword_context_secret`), `:298` (`shannon_entropy`); `redact.rs`
  `is_secret`; `porcelain.rs:421` `structural_secret_scrub`.
- **cost — by inspection:**
  - `has_keyword_context_secret` calls `s.to_ascii_lowercase()` — a **full-string
    heap allocation** — once per field, then does up to `KEYWORD_PREFIXES.len()`
    (6) substring `find` scans over it (`secret_shape.rs:179–215`). For a long
    charter/reason field that's a 6× O(L) substring sweep plus an O(L) alloc,
    run for **every** field on every write event (`is_structural_secret` calls it
    unconditionally at `:95`).
  - `is_structural_secret` (`:89`) does `KNOWN_PREFIXES.iter().any(|p| s.contains(p))`
    — 12 independent `contains` substring scans — *before* the keyword scan, so a
    benign long field pays ~18 substring passes + a lowercase alloc per call.
  - `structural_secret_scrub` returns `s.to_string()` for the (common) safe case
    (`porcelain.rs:425`) — an allocation even when nothing is redacted; combined
    with the JSON round-trip in `scrub_to_canonical` (`porcelain.rs:532`:
    `value.to_string()` then re-parse via `canonical_json`) the write path
    serializes the payload **twice**.
- **the fix:** (a) lowercase once and thread it into both
  `has_keyword_context_secret` and any case-insensitive prefix check, or use a
  case-insensitive byte scan to avoid the alloc; (b) early-out
  `is_structural_secret` on a cheap length/charset pre-filter; (c) have
  `scrub_to_canonical` canonicalize the already-built `Value` directly
  (`canonicalize_value` is already exposed in spirit) instead of
  `to_string`→re-parse. None are hot enough to be urgent, but they're free wins
  on the append path.

### F5 — per-record `String` clones in the chain primitives (P2/P3 micro)

- **id:** PERF-F5
- **severity:** **P3** (micro; folded into F1's constant factor)
- **where:** `tamper/mod.rs:138` (`prev_this = record.this_hash.clone()` each
  iteration), `log/mod.rs:159` (`hex::encode` alloc per hash),
  `log/mod.rs:345` (`head_hash` clones the last hash).
- **cost:** O(1) per record but n times per read; ~10k allocations/read at
  n=10k. Already captured in F1's measured creep.
- **the fix:** borrow instead of clone in the verify loop (F1.2).

---

## What I measured (numbers)

- **`verify_chain`:** linear, 3.95 → 10.3 µs/record (100 → 10 000 records);
  **103 ms/call at 10 000 records.**
- **End-to-end `hugit checks show --log`** (release, ~12.5 ms process baseline
  subtracted): ~8 ms (1k) / ~36 ms (5k) / **~73 ms (10k)** of chain re-verify per
  single read.
- **`intents_from_log`:** ~0.11 ms (100) → ~36 ms (10 000).
- **Reconcile inner loop (O(I·n)):** 3.7 ms (100) → 157 ms (500) → 866 ms (1k)
  → **3.64 s (2k)** → **18.2 s (5k)** — textbook quadratic.

## What's fine (don't touch)

- **`snapshot_tree`/`collect_files`** (`checks/run.rs:348`): O(matched files),
  reads+hashes each matched file once per `hugit check`. That's inherent to a
  content-addressed memo key — you must hash the inputs to key the cache. The
  walk is cycle-safe (visited-set + depth cap) and prunes `target/`/`.git/`.
  Not redundant, not a pathology. (It is a per-*check* cost, not per-read.)
- **`shannon_entropy`** (`secret_shape.rs:298`): fixed `[usize;256]` count array,
  single O(L) pass — already optimal; no allocation. Fine.
- **`compute_memo_key`/`compute_this_hash`** formula: correct single-source byte
  format; the only nit is the per-call `Vec` pre-image (F1.3), not the logic.
- **Locking windows** (`FileLock` + atomic temp-then-rename): the lock is held
  across load→mutate→save by design (correctness — WF-CLI2); the long pole inside
  that window is F1/F2, so fixing those also shortens the lock-held time. No
  separate lock pathology.
- **`scrub_payload` recursion**: walks the payload tree once; structurally fine.
  Only the per-field allocs (F4) are wasteful.

## Bottom line

The two real scaling pathologies are **F2 (reconcile, O(I·n)≈O(n²), measured
18 s at 5k events)** and **F1 (verify-the-whole-chain on every read, 73 ms at
10k events)**. Both stem from the same shape: an append-only log re-processed
from genesis on each operation. F2's fix is a one-line hoist + a `HashSet`
(O(n²)→O(n)); F1's fix is a verified-prefix memo (O(n)→O(Δ)). Everything else is
P2/P3 allocation hygiene on paths that aren't hot enough to hurt yet.
