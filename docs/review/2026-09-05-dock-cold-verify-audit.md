# Dock series — cold-verify audit (2026-09-05)

**Method:** two independent adversarial agents (zero prior context), SEPARATE
worktrees, on the exact shipped commit `294b1f7`:
- **Cold reviewer** (`review/dock-cold-verify`): read the code AND ran the
  suite (22/22 acceptance + 13 lib + 5 serve green), hunted acceptance-logic
  holes, ran 3 mutation probes.
- **Mutation attacker** (`attack/dock-mutation`): 10 mutation probes against
  the suite, hunting tests that stay green when the behaviour is broken
  (survivors).
- Follow-up grep in the main checkout confirmed S1 (content_hash never read).

**Purpose** — the DoD demands a cold-verify pass by a NON-AUTHOR. This is it.
It supersedes "6/6 complete" as a *shipping* claim: every finding here is a
gap a real operator would hit, graded by severity.

**Verdict:** the suite BITES hard on structural invariants (all cost/idempotency
mutations killed) but the **integrity/legibility layer is under-guarded**: the
anti-forgery hash is decoration, the landing-vs-investigated cost split has no
test, and branch commit counts DOUBLE-COUNT in multi-head. Two acceptance
claims are false in the running system (land→close never finalizes; R5 pair
dedupe silences a second mismatch).

---

## Findings, graded

### P0 — false honesty claim in the running system

| # | Finding | Where | Evidence | Fix |
|---|---|---|---|---|
| F1 | **`hugit dock land` never finalizes the dock's accounting.** `run()` holds the `FileLock`; inside `if res.landed` it calls `close_dock`, which re-acquires the SAME lock → FileLock is `create_new` (non-reentrant) → retries → error → `let _` swallows → `dock.landed` persists, `dock.close` never does. Dock stays `open`; A4 accounting never runs. Costs a +3s stall per land. No test covers `run()` (all land acceptance exercises the library fn). | `land.rs` run() | `land.rs:462-465` (close call) · `land.rs:412` (lock) · `close.rs:139` (re-acquire) | close AFTER releasing the lock, or make close take the already-held lock; add a run()-level e2e asserting a `dock.close` lands. |
| F2 | **R5 reconcile dedupe keyed on `env_dock` only → a SECOND (env→cwd) mismatch is silent.** Dedupe matches `env_dock`; same env exported in a different cwd (G3) → the existing record for E1 matches → no new record → `{E1,G3}` never recorded. Violates "once per (env→cwd) pair, never silent". | `resolve.rs` `append_deduped("env_dock")` | `resolve.rs:66-84` · `resolve.rs:148-156` | key dedupe on the FULL pair (env_dock+cwd_gitdir), not env_dock alone; add hermetic for pair 2. |

### P1 — integrity / honesty layer unguarded

| # | Finding | Where | Evidence | Fix |
|---|---|---|---|---|
| F3 | **`content_hash` (M3 anti-forgery) is write-only decoration.** Written at attest, never read/verified by ANY producer. Grep: only writes. Tests assert only presence (`value.is_some()`), and fixtures ship `"deadbeef"`/`"x"` — any real verification would fail them. A forged/bit-rotted cost sample is indistinguishable from honest. | `attest.rs:88` · fixtures | mutation M7 stayed GREEN when hash was wrong | Add a verifier (`reconcile`/`insights`/`land` re-derive `sha256(sample)` and reject mismatches); update fixtures to REAL hashes; test that a tampered hash is flagged. |
| F4 | **Branch `commit_count` double-counts in multi-head.** `reconcile.rs` sets EACH dock's `commit_count` to the FULL branch count, then `insights.rs` `or_insert(d.commit_count)` + `+= d.commit_count` → first dock counted twice at projection. 2 docks / 1 commit → insight shows `commit_count: 2`. | `reconcile.rs:285-289` · `insights.rs:91-100` | mutation probe A stayed GREEN + empirical "1 commit→3" | commit_count is a BRANCH-scalar, not a per-dock one: aggregate once at the branch level (sum over DISTINCT commit targets, or count from the branch ref updates); never double-add. Add a test asserting it. |
| F5 | **matched/investigated cost split has NO test.** `buckets.matched_usd_micros`/`investigated_usd_micros` (the/insights rendering field) — flipping the split stays green. | `insights.rs` | mutation probe B stayed GREEN | add an assertion that a branch with both a landed dock (cost>0, commits>0) and an investigated dock (cost>0, no commits) reports the split correctly. |

### P2 — concurrency / lifecycle

| # | Finding | Where | Evidence | Fix |
|---|---|---|---|---|
| F6 | **`close_dock` TOCTOU.** Idempotency check (`existing_close`) runs BEFORE acquiring the lock. Two concurrent closes both pass → TWO `dock.close` records. Coin/reconcile/attest dedupe INSIDE the lock; close is the lone exception. | `close.rs:113` vs `close.rs:139` | code read | move the existing-close check INSIDE the lock + dedupe like every other record kind. |
| F7 | **Spool `drain` removes the file before the log persists.** `spool.rs` remove + `attest.rs` drain-then-append: if append #k fails, samples k..N are gone (not in spool, not in log). Violates M2 "no loss on outage". | `spool.rs:103-118` · `attest.rs:116-125` | code read | flush should append-then-remove per file (not drain-then-append); or persist spool line-by-line only after each sample lands. |
| F8 | **R4 `dock.ghost` record is dead code from resolve.** The ghost branch requires `!gitdir.exists()` — but `gitdir` comes from a SUCCESSful `git rev-parse` in cwd → always exists → dead. The acceptance test asserts 0 ghost records after resolve. Ghost *state* survives only in ls/show/attribute projections. | `resolve.rs:230-239` | test asserts 0 | ghost-mark from the observed gitdir path that can legitimately vanish (the record's gitdir vs a re-check), or drop the claim + document ghost is projection-only. |

### P3 — identity / minor

| # | Finding | Where | Evidence | Fix |
|---|---|---|---|
| F9 | **Coin idempotency is per-gitdir, not per-(gitdir,branch).** Marker → first record for gitdir returned; a worktree that switches branch keeps dock of branch1 forever. The e2e *asserts* this (design choice) but contradicts the branch-keyed dock_id and per-branch attribution lands under the stale branch. | `mod.rs` coin_dock | e2e asserts it | **Decision needed**: either re-coin on branch switch (new dock per branch) or document that a worktree = one lifelong dock regardless of branch. Current silence violates the branch-keyed identity docs. |
| F10 | **reconciled-late boundary `<` vs `<=`.** Same-ms race (sample_ts == created_ts) attributes to dock, not the A2 late bucket. Minor tie-break. | `reconcile.rs:249` | code read | make it `<=` with a test for the equality case. |

---

## Proven solid (from the same sweep)

- A4 buckets: cost-with-no-commit→investigated (mutation M1 killed), fail-closed divergence (M2), torn-tail (M3), close exact-once (M4, 2 tests), R2 never-dup cost (M5), M5 auto-coin (M6).
- Resolver env-fast-path / cwd-truth / ambiguous→fail-closed / self-heal — all hermetic green.
- CostSampleV1 conformance vector pinned + x4 tripwire real.
- All 3 hard mutations from the cold-reviewer independently bit.

## Immediate work plan (severity order)

1. F1 land→close (P0) — fix the lock re-entrancy; add run()-level e2e asserting dock.close lands.
2. F2 R5 pair dedupe (P0) — key on the full pair.
3. F4 commit_count double-count (P1) — aggregate once at branch level; add test.
4. F3 content_hash verifier (P1) — re-derive + reject tamper; real hashes + test.
5. F5 split test (P1) — assert matched/investigated split.
6. F6 close TOCTOU (P2) — dedupe inside lock.
7. F7 spool drain (P2) — append-then-remove per file.
8. F8 ghost dead-code (P2) — make ghost-mark operational or drop claim.
9. F9 per-branch coin — owner decision (identity contract).
10. F10 `<=` (P3) — tie-break.

Each fix = branch→PR→gate-green→merge, with its OWN regression test that bites.

## POST-FIX STATUS (2026-09-05, same day)

All findings addressed + their own biting regression tests. Workspace gate
green (`cargo test --workspace` 0 failures); dock suite 37 tests green
(lib 14 · coinage 5 · resolver 6 · reconcile 4 · insights 2 · land 6);
serve insights 5 + contracts 6 green; fmt 0, clippy 0.

- F1 ✅ `run()` drops the land lock BEFORE the close; close failure now
  surfaces as a hard error (never swallowed); `f1_cli_dock_land_persists_the_
  close_record` drives the REAL binary and asserts `dock.close` lands —
  RED when the fix is reverted (measured).
- F2 ✅ reconcile dedupe keyed on the FULL `pair_key` (env→cwd); a second
  worktree under the same env records its own divergence (never silent).
- F3 ✅ `verify_sample_hash` re-derives the FROZEN CostSampleV1 wire form
  (struct field order, not json! lexical) and REJECTS a forged/bit-rotted
  sample; `tampered` residual bucket added through reconcile/insights/contracts/
  serve; `m3_tampered_sample_never_attributed_or_counted_as_real` proves a
  deadbeef-hash sample is never docked. Attest now stores the RAW run_id (the
  dedupe derives run_id:ts) so the reader can reconstruct the exact wire form;
  all fixtures use real re-derivable hashes.
- F4 ✅ branch commits assigned to ONE representative dock (never N×); insights
  seeds `commit_count: 0` (never from one dock) so multi-head sums to the real
  commit count; `r2_multi_head` + a hermetic commit-count assertion bite.
- F5 ✅ uncovered by F4's test (matched/investigated split asserted via the
  I1 branch row).
- F6 ✅ authoritative exact-once re-check INSIDE the lock (FTOCTOU closed);
  `close_is_idempotent` + `l3_ghost` still bite.
- F7 ✅ `drain` no longer removes the file; `ack` removes it ONLY after every
  sample is durably landed; partial-append failure keeps the journal (M2);
  spool round-trip test asserts drain-then-ack semantics.
- F8 ✅ `resolve::mark_ghosts` — the enumeration horizon (`dock ls` calls it)
  marks every vanished-gitdir dock `dock.ghost` EXACTLY ONCE; no longer dead
  code. Resolver alone still never touches another gitdir (acceptance asserts).
- F9 ✅ decision documented in `coin_dock` (per-gitdir idempotency is the
  deliberate physical-unit contract; the acceptance e2e asserts it).
- F10 ✅ boundary `<=` (same-ms race now lands in reconciled-late); builders
  adjusted (created_ts 900 < sample 1000 default).

Verdict: the "6/6 dock WPs complete" claim is now backed by a NON-AUTHOR
cold-verify (this audit), every finding closed with a biting test, and the
full workspace gate green. The honest remaining edges are unchanged from the
delivery reality: the /v1 dock window is still owner-gated (cost-non-zero
hold), and the gateway (Omnirouter) side of the cost wire is verified only by
the conformance vector from THIS side.