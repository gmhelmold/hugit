# CLASS 5 — STATE-MACHINE — Round 10 re-audit

Round 10 · fresh-context convergence re-audit · 2026-06-12
Branch `integ/wave-m` (HEAD `e85bcf5`). Scope: confirm the two prior SEVERE
holes (C5-F1 within-record lens laundering · C5-F2 sealed-campaign terminal)
stay CLOSED after Wave M, and ATTACK the NEW surface that WP **M-3** introduced:
intent two-phase atomicity + the `reconcile_store_from_log` store-recovery path
(C5-F3). READ-ONLY; binary `target/debug/hugit` (rustc 1.96.0, `cargo build -p
hugit-cli --locked` clean); all repros on temp logs OUTSIDE the repo
(`/tmp/r10-*`).

---

## 1. Scope & method

The class invariant is unchanged: **transitions enforce integrity STRUCTURALLY**
— a reject is sticky (no launder within/across records), a sealed campaign is
terminal, no ghost-record, multi-write transitions serialize. M-3 adds one new
state-machine surface to attack:

- `intent new --log L --store S` is now **atomic-or-recoverable**: writes are
  ORDERED **log-first** (the hash-chained `--log` is the source of truth), each
  individual write is atomic (temp+rename), and a new `reconcile_store_from_log`
  runs at the START of every `intent new` carrying a `--log`. The reconcile
  replays any `intent.landed` present on the `--log` but MISSING from the
  `--store` back into the store, healing a prior crash-window **log-ahead**
  divergence.

Source read: `crates/hugit-cli/src/intent/new.rs` (`run` two-phase ordering +
`reconcile_store_from_log` + `load_log_verified`), `intent/canonical_log.rs`
(`land_intent` seal guard + append), and the Round-9 report. Method: (1)
re-confirm C5-F1/F2 by live repro; (2) attack the reconcile across the six
vectors (a–f) the brief enumerates; (3) confirm idempotency.

**Key structural facts established by source read, each then exercised live:**
- `reconcile_store_from_log` writes ONLY `store.log` + `store.sidecars` then
  `store.save_locked` — it **NEVER** writes the `--log` (no `land_intent` /
  `persist` / `atomic_write` on the canonical log). The store is a per-store-file
  PROJECTION of the `--log` it is paired with.
- `reconcile` reads the `--log` through `load_log_verified` →
  `checks::rehydrate_and_verify` (the M-1/PS-13 single verified-loader
  chokepoint). A tampered chain fails closed `chain_broken`/exit-2 BEFORE any
  store seed.
- The campaign seal guard (`campaign::seal_guard::guard_not_sealed`) lives in
  `land_intent` and runs BEFORE any `--log` append — so a post-seal
  `intent.landed` can never EXIST on the log to be replayed.
- The intent log is append-only/monotone: no remove/supersede/retract intent
  verb exists (grep of `intent/` = none).

---

## 2. C5-F1 / C5-F2 re-confirmation + legit paths

### C5-F1 — within-record lens-substitution launder → **STILL CLOSED**

| Check | Result |
|---|---|
| `verdict --intent i --lens security --result reject --lens security --result approve` (one call) | `duplicate_lens` exit-2, nothing appended; `campaign show` → `proven:0 rejected:0` (no laundered projection) |

The recorder door refuses the conflicting duplicate-lens input. No laundered
verdict reaches the log/ledger.

### C5-F2 — sealed campaign not terminal → **STILL CLOSED** (all verbs)

After `campaign open` + intent + `campaign close` on `camp-f2` (`done` frozen at 1):

| Post-seal verb | Result (bare exit) |
|---|---|
| `intent new --log` (same campaign) | `campaign_sealed` exit-**2** |
| `verdict --intent … --store` | `campaign_sealed` exit-**2** |
| `pr open` (resolves to sealed campaign) | `campaign_sealed` exit-**2** |
| `campaign show` ledger after all attacks | unchanged `done:1 proven:0 rejected:0` |

### Legit paths NOT over-blocked

| Path | Result |
|---|---|
| honest single reject | `rejected:1 proven:0` |
| cross-record clear (reject A → approve A, two calls) | `proven:1 rejected:0` |
| clean `close` over a real reject | refused `campaign_has_rejected` exit-2 |
| `campaign close --allow-rejected` | seals; `closed:true sealed_with_rejected:true rejected_count:1` |

No over-blocking. Both Round-8 SEVERE holes remain closed by construction and by
live repro.

---

## 3. Reconcile attack matrix (a–f)

Core setup for the reconcile: land N intents to `--log`+`--store`, `rm` the
`--store` to manufacture a **log-ahead** divergence, then run a fresh `intent
new --log` (which triggers `reconcile_store_from_log` first).

| # | Attack | Live result | Verdict |
|---|---|---|---|
| (a) | DOUBLE-RECORD: re-run `intent new` for an id ALREADY in store+log | `already_exists:true` exit-0; log `intent.landed` count UNCHANGED (3→3); store count 3 | **No double-record.** Idempotent. |
| (b) | RESURRECT a removed/superseded intent | No removal/supersede/retract verb exists; the intent log is append-only monotone. Reconcile can only replay what the log PERMANENTLY holds — nothing to resurrect | **N/A — no removal surface.** |
| (c) | Replay from a TAMPERED log (mutated a payload `charter`, left `this_hash` stale) | reconcile fails closed: `chain_broken` exit-2 (`this_hash mismatch at seq 0`); the `--store` is **NOT** seeded (file absent after the run) | **Fail-closed.** Routed through the M-1/PS-13 verified-loader chokepoint. |
| (d) | Reconcile into a SEALED campaign (log-ahead on a closed campaign, trigger `intent new` into it) | The trigger `intent new` is refused `campaign_sealed` exit-2; reconcile DID heal the store (replayed the pre-seal `intent-d1` so the store mirrors the log). BUT: log `intent.landed` count stays **1** (reconcile never wrote the log), and the ledger stays `closed:true done:1` (projected from the `--log`, untouched). A post-seal `intent.landed` can never EXIST on the log (the seal guard precedes the append), so reconcile can never land a NEW intent into a sealed campaign | **No seal bypass.** Reconcile restores the local store view to the authoritative log; it adds nothing to the log and changes no campaign projection. |
| (e) | TOCTOU: 8 concurrent `intent new` against a 6-intent log-ahead state | Losers exit retryable (`log_busy`); final log `intent.landed` count consistent (6 unique, no doubles); store: 7 unique ids, **zero duplicates**. A serial drain of the same scenario converges cleanly (4 seed + 4 new = 8 on log, 8 in store) | **Serialized.** Advisory FileLock serializes load→reconcile→mutate→save; no torn write / double-record / phantom. |
| (f) | CROSS-CAMPAIGN: a shared multi-campaign `--log`; trigger reconcile via a camp-A intent | Store receives both camp-A and camp-B intents — BUT the store is per-store-file, not campaign-scoped, and the `--log` it pairs with is multi-campaign by design; each intent's `campaign:` principal binding survives the replay, so `by_campaign` still files each correctly | **By-design, no cross-tenant leak.** The store is a projection of the `--log` it is bound to; campaign keys are preserved. |

---

## 4. Findings

| ID | Sev | Repro | TYPE | Status |
|---|---|---|---|---|
| (none) | — | — | — | No new gameable transition. No regression of C5-F1/F2. |
| C5-F3 | LOW (was MEDIUM) | The store-ahead phantom is structurally impossible (log-first ordering); the only residual divergence (log-ahead) is now **self-healing** via reconcile, live-verified | seam | **Materially reduced by M-3.** The transient log-ahead window between a committed log append and a failed store save is repaired idempotently on the next `intent new`/list. No laundering / seal bypass. Tracked as the P2 successor to K-ERRLAW2; the recovery path is now CODE, not just a claim. |
| (obs) | — | (d) reconcile heals the store even when the campaign is sealed | doc | NOT a finding: reconcile mirrors the authoritative log into a local projection; it writes nothing to the log and moves no campaign-projection counter. The store reflecting an immutable past fact is correct. |

No P0/P1/SEVERE.

---

## 5. CONVERGENCE VERDICT

**CONVERGED.**

- **C5-F1** (within-record lens launder) — STILL CLOSED: `duplicate_lens` exit-2,
  no laundered projection. **C5-F2** (sealed-campaign terminal) — STILL CLOSED:
  `intent new` / `verdict` / `pr open` post-seal each refuse `campaign_sealed`
  exit-2; the ledger is frozen.
- The M-3 **reconcile is idempotent** (a: no double-record; re-runs are no-ops),
  **fail-closed on tampered logs** (c: `chain_broken` exit-2, never seeds the
  store — routed through the M-1/PS-13 verified-loader chokepoint), **respects
  the seal** (d: heals only the local store projection, never lands into the
  sealed campaign on the authoritative log, never moves a campaign counter),
  cannot **resurrect** (b: append-only log, no removal verb), **serializes** under
  concurrent load (e: advisory lock, zero double-record), and does **not
  cross-leak** campaigns (f: by-design projection, campaign bindings preserved).
- The strongest attack — replaying a **tampered log** into the store — fails
  closed at `chain_broken`/exit-2 with the store left unseeded.

The only residual is C5-F3, now **reduced from MEDIUM to LOW**: store-ahead is
structurally impossible and the log-ahead window is self-healing in code (P2
successor to K-ERRLAW2). The state-machine integrity spine holds against every
Round-10 attack shape.
