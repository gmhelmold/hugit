# CLASS 5 — STATE-MACHINE INTEGRITY — SOTA audit

Round 8 · fresh-context adversarial root-cause audit · 2026-06-12
Scope: the four state machines — **verdict · campaign · pr · intent** — and
every multi-write transition across them.
Binary under test: `cargo build -p hugit-cli --locked` (rustc 1.96.0), HEAD
`integ/web-spine` (Wave K, `def8a18`-derived). All repros driven on a temp log
outside the repo.

---

## 1. Scope & method

The invariant for this class: **state transitions enforce their integrity rules
STRUCTURALLY** — a sealed campaign is terminal, a rejection is sticky (cannot be
laundered), a verdict cannot be ghost-recorded, multi-write transitions are
atomic. The recurring root the prior rounds named: *resolution logic is gameable
/ checked ad-hoc per verb, so a new verb (or a new payload shape) forgets the
rule*.

Method:
1. Read every transition across the four machines:
   - `verdict/mod.rs` — `record()`: validate → tree-hash door → intent-existence
     guard → **post-seal guard (K-VERDICT)** → idempotency → append.
   - `hugit-ledger/ledger/mod.rs` — `Ledger::from_records`: the reject-sticky
     per-lens fold that is *supposed* to be the single source of truth for
     `proven`/`rejected`.
   - `campaign/{open,close,abandon}.rs` + `world.rs` — open → close(seal) /
     abandon, the `sealed_with_rejected` durable seal fact, the ghost-record /
     not-opened guards.
   - `pr/mod.rs` — open → land(queue) → settle(landed) / abandon, terminal
     idempotency.
   - `intent/new.rs` + `canonical_log.rs` — the `--store` + `--log` two-phase
     commit (log-first ordering, K-ERRLAW2).
2. Built THE matrix (§2): every transition × {structural ✓ / ad-hoc / ✗
   gameable} × attack.
3. Reproduced every ✗ live (§3) and read the broken invariant out of the
   projection (`campaign show` / `campaign close`).
4. Asked the root question (§4): enforce-by-construction, or re-checked ad-hoc?

Uncertain transitions were treated as gameable until a live repro proved
otherwise.

---

## 2. Complete inventory (THE matrix)

Legend: ✓ structural · ~ ad-hoc (checked in one verb, not by construction) · ✗
gameable (live repro in §3).

### Verdict machine

| Transition | Rule it must enforce | Verdict | Attack |
|---|---|---|---|
| record (dry, no `--store`) | never touch log | ✓ | — |
| record → append `verdict.recorded` | one per intent+lens-set, idempotent | ~ | dedup is latest-only (correct), but see laundering |
| reject stays sticky across records | approve under a NOVEL lens cannot clear a reject | ✓ | cross-record lens-substitution **held** (F-old, K-VERDICT) — re-confirmed clean |
| reject sticky **within one record** | a same-call duplicate-lens `reject … approve` cannot launder | **✗** | **F1** — `--lens sec --result reject --lens sec --result approve` → `proven:1, rejected:0` |
| same-lens re-approval clears reject | only the SAME lens re-approving clears | ✓ | confirmed legit clear works |
| ghost-record (intent does not exist) | refuse verdict for non-existent intent | ✓ | `intent_not_found` guard fires (B2) |
| post-seal verdict append | refuse after `campaign.closed` | ✓ | `campaign_sealed`/exit-2 fires (K-VERDICT) — confirmed |
| TOCTOU (concurrent record) | advisory lock across load→append→persist | ✓ | `FileLock` held across critical section |

### Campaign machine

| Transition | Rule | Verdict | Attack |
|---|---|---|---|
| open (idempotent) | one `campaign.opened`, no dup | ✓ | — |
| close = SEAL | refuse if any PR in-flight | ✓ | `in_flight_prs` guard |
| close = SEAL | refuse if any reject, unless `--allow-rejected` | ~ | reads **live ledger** `rejected()`; defeated by F1 (laundered reject → seals clean) |
| close persists `sealed_with_rejected` durably | seal fact is immutable | ✓ | `campaign_closed_seal_condition` reads payload, not live ledger (WJ-CLOSE) |
| close (idempotent re-close) | no dup, reads persisted seal fact | ✓ | — |
| close ghost-record | refuse to seal a never-opened campaign | ✓ | `not_opened` guard (WF-CLI2) |
| **post-seal append of NEW records** | sealed campaign is terminal/immutable | **✗** | **F2** — `intent new --log` and `pr open` append into a CLOSED campaign; `done` 1→2 |
| abandon | refuse on a sealed campaign | ✓ | `already_closed` guard |

### PR machine

| Transition | Rule | Verdict | Attack |
|---|---|---|---|
| open (idempotent) | same campaign no-op; diff campaign = mismatch | ✓ | — |
| open author-kind | subagent rejected (D14) | ✓ | type-level (`AuthorKind` has no Subagent) |
| open in a SEALED campaign | should be refused (terminal) | **✗** | **F2** (same root — no sealed guard) |
| land → queue | unknown/empty PR refused; idempotent | ✓ | terminal-landed idempotency (WI-PR) |
| settle → landed | must be queued first | ✓ | `SettleNotQueued` guard |
| abandon | refuse on a landed PR | ✓ | `AbandonLanded` guard |

### Intent machine

| Transition | Rule | Verdict | Attack |
|---|---|---|---|
| new → store + log (two-phase) | log-first; store not committed if log fails | ✓ | K-ERRLAW2 test covers store-orphan-on-log-fail |
| new → store + log | **store-save fails AFTER log committed** → log ahead of store | ~ | **F3** — divergence in the reverse direction; not atomic, retry-reconciled only |
| new in a SEALED campaign | sealed campaign is terminal | **✗** | **F2** (same root) |
| new (idempotent) | dup id → existing, exit 0 | ✓ | — |

---

## 3. Findings

### F1 — DO-NOT-SHIP — within-record lens-substitution launders a sticky reject
**Severity: SEVERE (code).** TYPE: **code**.

The K-VERDICT reject-sticky fix lives in the ledger fold
(`Ledger::from_records`, the per-lens `BTreeMap` accumulator). It closes the
*cross-record* lens-substitution attack Round 7 found. It does **not** close the
*within-record* variant: a single `verdict` call accepts repeated `--lens/--result`
pairs, the recorder writes them verbatim into one record's `claims_checked`
(`["sec:reject","sec:approve"]`), and the fold processes that vector with a plain
`per_lens.insert(lens, outcome)` (`ledger/mod.rs:193`) — **last entry wins**, so
the trailing `sec:approve` silently overwrites `sec:reject` in the per-lens map.

Repro (live, temp log):
```
hugit campaign open --campaign camp-B --owner bob --charter t --log L
hugit intent new --campaign camp-B --charter c --id i-y1 --log L --store S
hugit verdict --intent i-y1 --log L --store \
      --lens security --result reject --lens security --result approve   # exit 0
hugit campaign show --campaign camp-B --log L   →  proven:1  rejected:0
```
The aggregate field of the SAME record honestly says `"aggregate":"reject"` (a
non-approve lens fails the panel) — so the record self-contradicts its own
projection. The reject is gone from `proven`/`rejected` with no trace.

ROOT: the projection fold treats within-record claims as latest-wins, breaking
the very reject-sticky invariant the fold was added to guarantee. The recorder
also does not reject a duplicate-lens-with-conflicting-result input.

### F2 — DO-NOT-SHIP — a SEALED campaign is not terminal; every verb except `verdict` can append into it
**Severity: SEVERE (code).** TYPE: **code**.

K-VERDICT added a post-seal guard, but **only to the `verdict` verb**
(`campaign_for_intent` + `campaign_is_sealed` in `verdict/mod.rs`). No other
mutation path consults the seal. After `campaign close`:

Repro (live):
```
hugit campaign open  --campaign camp-D --owner bob --charter t --log L
hugit intent new     --campaign camp-D --charter c --id i-d1 --log L --store S
hugit campaign close --campaign camp-D --log L                 # sealed
hugit intent new     --campaign camp-D --charter c2 --id i-d2 --log L --store S   # exit 0 — appended!
hugit pr open --pr pr-1 --campaign camp-D --author-kind orchestrator --run-id r1 \
      --intent i-d1 --log L                                    # exit 0 — appended!
hugit campaign show  --campaign camp-D --log L  →  closed:true, done:2  (was 1)
```
The post-seal `intent.landed` joins a sealed campaign's projection — `done`
mutates **after** the immutable seal — while the `sealed_with_rejected` payload
written at close time now diverges from the live ledger. (For contrast, the same
post-seal `verdict` is correctly refused with `campaign_sealed`/exit-2.)

ROOT: the seal check is point-local to one verb instead of being a shared
precondition every campaign-scoped mutation evaluates. Exactly the predicted
"a new verb forgets" structural root.

### F1 ⊕ F2 compound — clean seal over a rejected intent
**Severity: SEVERE.** F1 lets a real reject vanish from `rejected()`; `campaign
close` reads that live count, so the WI-PROVEN2 acknowledgment gate is bypassed:
```
hugit verdict --intent i-z1 --log L --store --lens sec --result reject --lens sec --result approve
hugit campaign close --campaign camp-C --log L
   → closed:true, sealed_with_rejected:false, rejected:0   # NO --allow-rejected needed
```
A rejected intent is sealed as cleanly-proven with zero audit trace.

### F3 — FIX-RECOMMENDED — intent two-phase commit is not atomic in the reverse direction
**Severity: MEDIUM (honesty/seam).** TYPE: **seam**.

`intent new` is log-first (K-ERRLAW2): if the `--log` append fails, the `--store`
is not committed (good; tested). The reverse is unguarded — if `store.save_locked`
fails AFTER the log append committed (`new.rs:256–269`), the **log is ahead of the
store** and the verb returns an error, so the caller believes "not created" while
the shared log already shows `intent.landed`. It is not a true atomic commit
(no rollback of the log append). Mitigated, not closed: the log is the
authoritative shared seam and a retry reconciles via idempotency, so this is a
seam/honesty residual, not a laundering defect. No live repro attempted (requires
injecting a store-write fault); flagged from code reading.

---

## 4. Root-cause analysis

Two structural roots, both the predicted class root ("checked ad-hoc per verb /
per payload shape, so a new path forgets"):

1. **The reject-sticky invariant is enforced only at the cross-record granularity,
   not over the unit it actually folds (a claims_checked entry).** The fold's
   `insert` is last-wins; the stickiness was asserted for separate records but
   never for entries *inside* one record. The recorder compounds it by accepting
   duplicate-lens input without normalization. (F1)

2. **The seal/terminal precondition is a verb-local guard, not a shared
   transition gate.** `verdict` checks `campaign_is_sealed`; `intent new`,
   `pr open/land/settle/abandon` do not. There is no single chokepoint where
   "is this campaign-scoped append legal given the campaign's lifecycle state?"
   is answered. (F2)

Both confirm Class 5's recurring failure mode: integrity is *re-checked ad-hoc*
rather than *enforced by construction*. The projection fold is the right place
for the source-of-truth (good design), but it does not normalize its own input,
and the lifecycle gate is scattered.

---

## 5. Recommended structural remediation

### Kill F1 — make the fold reject-sticky over *entries*, and normalize at the recorder
One class-killing change, two layers:

- **Ledger fold (the source of truth):** in `Ledger::from_records`
  (`crates/hugit-ledger/src/ledger/mod.rs`, the `per_lens.insert` at ~line 193),
  replace the plain insert with a **sticky merge** helper, e.g.
  `fn merge_lens_outcome(slot: &mut Verdict, incoming: Verdict)` that within the
  fold of one record's `claims_checked` never lets an `Approve` overwrite a
  `Reject`/`FixFirst` already set for that lens *by the same record*. The
  cross-record clear (same-lens later RECORD approves) stays — sticky merge only
  governs the within-record collapse. This makes the fold normalize its own
  input regardless of what the recorder writes.

- **Recorder (defense at the door):** in `verdict::record`
  (`crates/hugit-cli/src/verdict/mod.rs`), reject a duplicate `--lens` with
  conflicting `--result` in one call with a structured `duplicate_lens`/exit-2,
  or canonicalize to the sticky outcome before building `claims_checked`. The
  matrix-test must assert both layers (fold + recorder) independently.

### Kill F2 — one shared seal precondition for every campaign-scoped mutation
Lift the verdict-local guard into a single reusable check and call it from EVERY
mutation that targets a campaign:

- Promote `campaign_is_sealed` (currently private in `verdict/mod.rs`) to a
  shared helper, e.g. `crate::campaign::world::World::campaign_is_sealed(&self,
  key)` (the projection already exists: `campaign_closed`), and a thin
  `guard_not_sealed(log, campaign_key) -> Result<(), PorcelainError>` returning
  `campaign_sealed`/exit-2.
- Call it from: `intent::new::run` (after resolving the `--campaign`),
  `pr::open` (after resolving `--campaign`), and `pr::land`/`settle`/`abandon`
  (resolve the PR's campaign from its `pr.opened`). The verdict verb already has
  it.
- Best-by-construction option (heavier): route ALL campaign-scoped appends
  through one `append_campaign_scoped(world, …)` chokepoint that evaluates the
  lifecycle precondition once, so a future verb cannot append without passing the
  gate. This is the true enforce-by-construction fix versus three more ad-hoc
  call sites.

### F3 — make the two-phase a real commit or document the seam
Either (a) treat the log as the commit point and make the store a pure cache
rebuilt from the log on read (removing the divergence window entirely), or
(b) keep log-first but on a store-save failure, attempt a compensating no-op and
surface `partial_commit` honestly. Lowest-effort acceptable close: an explicit
test + doc that the log is authoritative and the store reconciles on retry.

---

## 6. Residual / accepted

- **F3** is accepted as a seam residual if remediation (a) is deferred — the log
  is authoritative and idempotent retry reconciles; recommend tracking as a P2
  successor to K-ERRLAW2.
- **Live-infra seams** (real tree-hash union-test arbitration, the queue
  disjointness verdict) remain hermetic placeholders by disclosure (P2) and are
  out of this class's scope.
- The TOCTOU / lock discipline (WF-CLI2) and ghost-record guards
  (`not_opened`) held under inspection — no new finding there.

**Verdict: 3 findings (2 SEVERE code + 1 compound + 1 medium seam). The class is
NOT closed.** The matrix shows the spine holds for ghost-record, TOCTOU,
cross-record lens-substitution, and seal-idempotency — but the reject-sticky
invariant leaks at the within-record granularity (F1) and the terminal-seal
invariant is enforced in exactly one of five mutation verbs (F2).
