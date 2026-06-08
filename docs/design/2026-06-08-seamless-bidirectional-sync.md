# Seamless bidirectional GitHub ↔ hugit sync (forge-arbitrated)

> Owner decision 2026-06-08: bidirectional sync is **vital and must not become
> friction**. This **supersedes the old E6** ("naive symmetric mirror", deferred
> by design) with a model that is seamless to users yet keeps the forge as the
> single source of truth — so it never corrupts and never violates the
> "embrace, don't assault / never naive symmetric sync" principle.

## The insight that makes it safe

Nobody writes the protected branch (`main`) directly. So "two people editing the
same thing" is just **two branches = two versions that coexist**. Branches are
independent refs — they sync both ways with **zero conflict**, automatically,
seamlessly. The only genuinely-contended resource is `main`, and `main` has
**one writer by construction: the forge's landing queue** (the arbiter). Remove
direct concurrent writes to the same ref and the "concurrent-write physics
problem" essentially evaporates.

Mental model: **like Google Docs** — both sides feel like instant two-way sync;
under the hood the forge arbitrates, so it can never become a corrupt free-for-all.

## The rules (the contract)

1. **Branch refs sync both ways, seamlessly.** A branch pushed on the GitHub
   side is ingested into the forge as that same branch ref — recorded as an
   external change-event (per D3⑤: never a fabricated intent, never touching
   `main`); a forge-side branch update mirrors out (E1). Independent branches ⇒
   no conflict ⇒ both sides converge with no human step.
2. **`main` is single-writer = the forge.** A direct GitHub-side push to the
   protected default branch is **never applied symmetrically**. It is rerouted
   onto the normal path (rejected-with-guidance, or auto-converted to a proposed
   branch/PR) so it lands through the queue. `main` advances ONLY via landing.
   This is not friction — nobody should push `main` directly anyway.
3. **No echo loop.** A change synced in one direction must not bounce back and
   re-emit. Convergence is **content-hash idempotent**: once both sides hold the
   same tip, sync goes quiet (reuse E1's "mirror_write from observed mutation,
   not tip-inequality").
4. **Same-branch concurrent divergence (the rare residual).** If one branch ref
   is moved to incompatible tips on both sides at once, the **forge arbitrates
   that one ref**: the forge-authoritative tip wins `main`-ward, and the
   divergent GitHub-side tip is preserved as a recoverable incident/side-ref —
   **never silently dropped, never corrupting**, chain stays verifiable. (This
   is also exactly hugit's wedge — landing/merge — applied at the GitHub border.)
5. **No symmetric-authority state exists.** Property: there is no reachable state
   in which both sides are authoritative for `main` simultaneously. The forge is
   always the arbiter (this is the hard "never naive symmetric sync" guarantee).

## Owned acceptance (the red→green set for the WP)

- ① **Branch round-trips both ways:** a branch created/advanced on the GitHub
  side appears in the forge as that branch ref (ingested as an external
  change-event, not an intent, `main` untouched); a forge-side branch advance
  appears on GitHub. Byte-identical tips.
- ② **Idempotent convergence / no echo loop:** after a change syncs, re-running
  the sync is a no-op (content-hash equal ⇒ zero re-emit); a planted echo is
  caught (oracle goes RED if the same change re-emits).
- ③ **`main` single-writer:** a direct GitHub-side push to the protected branch
  does NOT mutate the forge `main` symmetrically — it is rerouted (proposed
  branch/PR), and `main` only advances via the landing queue. Oracle goes RED if
  a GitHub-side `main` write lands on the forge `main` without going through the
  queue.
- ④ **Same-branch divergence arbitrated:** concurrent incompatible tips on one
  branch ⇒ forge tip authoritative, GitHub tip preserved as a recoverable
  incident ref, never dropped, chain verifiable. Oracle goes RED on silent drop
  or corruption.
- ⑤ **No-symmetric-authority property test:** across interleavings, no reachable
  state has both sides authoritative for `main`. (Structural / property test.)

## Build plan

- **Crate:** `hugit-mirror` (owns outbound E1 + import E2). New `sync/` (or
  `bidir/`) module; reuse E1's verify/observed-origin + E2's ingest + D3's
  change-event + B4/queue landing contract.
- **Hermetic now (the logic):** model a two-sided world in-process — a "GitHub
  side" git repo fixture and the forge — and drive branch round-trips,
  idempotent convergence, `main`-single-writer rerouting, and same-branch
  divergence arbitration against the REAL forge landing/mirror surfaces.
- **P2 seam:** the LIVE GitHub webhook/poll that detects a GitHub-side change
  (needs live GitHub + `HUGIT_GH_TEST_REPO`) is gated behind env, run-not-skip
  when set. The arbitration + convergence logic is fully proven hermetically.
- **Oracle-first, mutation-verified.** Old E6 contract is retired/superseded;
  note that in the decomposition register.
