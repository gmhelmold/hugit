# CLASS 5 — STATE-MACHINE — Round 9 re-audit

Round 9 · fresh-context convergence re-audit · 2026-06-12
Branch `integ/wave-l` (HEAD `95353f9`). Scope: the four state machines —
**verdict · campaign · pr · intent** — re-running the Round-8 transition matrix
against Wave L (L-D), which claims to close C5-F1 (within-record laundering) and
C5-F2 (sealed campaign not terminal). READ-ONLY; all repros on temp logs OUTSIDE
the repo, binary `target/debug/hugit` (rustc 1.96.0, `cargo build -p hugit-cli
--locked` clean).

---

## 1. Scope & method

The invariant for this class is unchanged: **transitions enforce integrity
STRUCTURALLY** — a sealed campaign is terminal, a reject is sticky (cannot be
laundered within OR across records), no ghost-record, multi-write transitions
serialize. Method:

1. Re-verified the two SEVERE holes are CLOSED by live repro (the exact Round-8
   reproductions).
2. Attacked for NEW laundering / non-terminal vectors the fix might have missed
   (reversed/triple-result lens, case-variant lens, the ledger fold directly via
   a raw payload patch, every `pr` verb post-seal, campaign re-open, a non-verb
   refstore append, TOCTOU close-vs-intent, the intent two-phase reverse seam).
3. Confirmed the legit paths are NOT over-blocked.

Source read: `crates/hugit-cli/src/campaign/seal_guard.rs` (the shared
chokepoint), `crates/hugit-ledger/src/ledger/mod.rs` (`merge_lens_outcome` +
the per-record scratch fold), `crates/hugit-cli/src/verdict/mod.rs` (the
`duplicate_lens` door + the shared seal call), `crates/hugit-cli/src/pr/cli.rs`
(`guard_campaign_not_sealed` on open/land/settle/abandon),
`crates/hugit-cli/src/intent/{new.rs,canonical_log.rs}`,
`crates/hugit-cli/src/campaign/{close,open,abandon}.rs`.

Note on the brief's phrasing: C5-F2 is described as a guard "at the
`hugit-refstore` append chokepoint." The implementation is actually a **shared
guard module** (`campaign::seal_guard::guard_not_sealed`) called from each
campaign-scoped verb under the lock it already holds — not a hook inside
`hugit-refstore`. Functionally equivalent for the invariant (verified below by
enumerating that the campaign projection reads ONLY the four guarded kinds), but
it is a single shared call-site, not a construction-level chokepoint inside the
append primitive. Recorded as an honesty note, not a finding.

---

## 2. Transition matrix re-verification

### C5-F1 — within-record lens-substitution launder → **CLOSED** (both layers)

| Check | Result |
|---|---|
| `verdict --lens security --result reject --lens security --result approve` (one call) | `duplicate_lens` exit-2, nothing appended; `proven:0 rejected:0` |
| reversed `approve … reject` (one call) | `duplicate_lens` exit-2 |
| triple `reject approve reject` (one call) | `duplicate_lens` exit-2 |
| ledger fold itself, raw payload `claims_checked:["sec:reject","sec:approve"]` | the read path REFUSES the tampered record (`chain_broken` exit-2 — hash chain), so the launder bytes cannot even reach the fold via the log; the fold's own stickiness is proven by `acceptance_round8_fold.rs` (5/5 green): within-record reject→approve AND approve→reject both resolve `rejected:1 proven:0` |

Both layers verified: the recorder **door** refuses the conflicting input
(`duplicate_lens`), AND the ledger **fold** (the source of truth) is reject-sticky
within a record via `merge_lens_outcome` (proven by unit/acceptance test, since
the hash chain blocks injecting a self-contradictory record at runtime). This is
the genuine belt-and-suspenders close Round 8 recommended.

### C5-F2 — sealed campaign not terminal → **CLOSED** (all five verbs)

After `campaign close` on `camp-D`/`camp-E` (`done` frozen at 1):

| Post-seal verb | Result |
|---|---|
| `intent new` | `campaign_sealed` exit-2 |
| `pr open` (existing or new pr) | `campaign_sealed` exit-2 |
| `pr land` (re-queue) | `campaign_sealed` exit-2 |
| `pr land --settle` | `campaign_sealed` exit-2 |
| `pr abandon` | `campaign_sealed` exit-2 (when the pr resolves to the sealed campaign) |
| `verdict` | `campaign_sealed` exit-2 |
| `done` after all attacks | unchanged (1) |

All four campaign-scoped verbs route through the ONE shared
`campaign::seal_guard::guard_not_sealed`; the seal-detection (`campaign.closed`
record naming the key) matches `World::campaign_closed`, so projection and guard
cannot disagree.

### Legit paths NOT over-blocked

| Path | Result |
|---|---|
| honest single reject | `rejected:1 proven:0` |
| multi-DIFFERENT-lens one call (all approve) | exit 0, `proven:1` |
| case-variant lens `sec` + `SEC` reject/approve | both recorded, aggregate honest `reject`, `rejected:1` (NOT a launder — distinct lens identities, reject sticky) |
| cross-record same-lens clear (reject A → approve B) | clears → `proven:1 rejected:0` |
| cross-record DIFFERENT-lens approve over reject | stays `rejected:1` (sticky preserved) |
| clean close over a real reject | refused `campaign_has_rejected` exit-2 |
| `campaign close --allow-rejected` | seals, `sealed_with_rejected:true rejected_count:1` |

No over-blocking. The F1⊕F2 compound (Round-8 SEVERE) is closed end-to-end: the
launder is refused at the door, a real reject survives the fold, and `close`
reads that live reject and refuses a clean seal.

---

## 3. Break attempts (new shapes)

1. **Different lens-launder shape — case/suffix variant** (`sec` reject, `SEC`
   approve, one call): NOT laundered. They are distinct lens identities; both are
   recorded, the panel aggregate is honest `reject`, the fold keeps `sec:reject`
   sticky → `rejected:1`. No collapse.
2. **Three-record / reversed within-record sequences**: all refused at the door
   (`duplicate_lens`); the fold is order-independent (a reject anywhere wins).
3. **Direct ledger-fold attack via raw log patch**: blocked upstream by the
   read-path hash-chain verification (`chain_broken` exit-2) — a tampered
   self-contradictory `verdict.recorded` cannot reach the fold through the log.
   The fold's stickiness is independently proven by `acceptance_round8_fold.rs`.
4. **Seal-evasion via a non-guarded verb**: enumerated every top-level verb. The
   campaign projection (`campaign/world.rs`) reads ONLY `campaign.*`, `pr.*`,
   `intent.landed`, `verdict.recorded` — exactly the kinds the four guarded verbs
   emit. `check`/`tournament`/`queue` either do not append (tournament) or emit
   `check.recorded`, which is NOT a campaign-projection input and cannot mutate
   `done/proven/rejected/seal`. No un-guarded campaign-scoped append path exists.
5. **Campaign re-open after seal**: `campaign open` is idempotent on
   `campaign_opened` and does NOT remove the `campaign.closed` record; the seal
   guard keys off ANY `campaign.closed`, so a re-open cannot unseal. No reopen
   verb exists.
6. **TOCTOU — concurrent `campaign close` vs `intent new` into the same
   campaign**, 12 trials: zero post-seal slips. The advisory FileLock serializes
   load→mutate→persist, so the intent either lands before the seal or is refused
   `campaign_sealed` — never a closed campaign with `done:2`.
7. **Intent two-phase reverse atomicity** (F3): UNCHANGED in Wave L. `new.rs`
   is log-first (`land_intent`, 256–258) then `store.save_locked` (267–269). A
   store-save failure AFTER the log commit leaves the log ahead of the store and
   returns an error; the log is authoritative and idempotent retry reconciles.
   Not a laundering or seal defect — the same MEDIUM seam residual Round 8 named.

---

## 4. Residual findings (sev · repro · TYPE)

| ID | Sev | Repro | TYPE | Status |
|---|---|---|---|---|
| C5-F3 | MEDIUM | code-read only (store-save fault injection needed); `new.rs:256–269` log-first, no log rollback on store fail | seam | **Accepted residual** — log authoritative + idempotent retry; tracked as a P2 successor to K-ERRLAW2 (Round-8 §6). Not gameable: no laundering/seal bypass. |
| (honesty) | — | C5-F2 guard is a shared call-site, not a `hugit-refstore`-internal chokepoint as the brief phrasing implies | doc | Functionally complete (all append paths enumerated); a future verb that forgets the call is a one-line miss, not a re-implemented rule. Recommend the heavier "all appends through one chokepoint" option only if a 5th campaign-scoped verb is added. |

No P0/P1. No new gameable transition found.

---

## 5. CONVERGENCE VERDICT

**CONVERGED.**

Both Round-8 SEVERE holes are closed by construction and re-verified by live
repro:

- **C5-F1** — reject is now sticky WITHIN a record at the fold (the source of
  truth, `merge_lens_outcome`, acceptance-tested) AND the recorder refuses a
  conflicting duplicate-lens input at the door (`duplicate_lens` exit-2). Every
  new launder shape tried (reversed, triple, case-variant, raw-fold) failed.
- **C5-F2** — a sealed campaign is terminal for ALL four campaign-scoped verbs
  via the ONE shared `seal_guard::guard_not_sealed`; enumeration confirms no
  un-guarded campaign-projection append path, re-open cannot unseal, and a
  concurrent close/intent race never slips (12/12).

Every legit path is intact (cross-record clear, multi-lens, `--allow-rejected`).
The only residual is the intent two-phase **reverse-atomicity seam (C5-F3)**, a
MEDIUM seam already tracked as a P2 successor to K-ERRLAW2 — not a gameable
transition. The state-machine integrity spine holds against all attack shapes.
