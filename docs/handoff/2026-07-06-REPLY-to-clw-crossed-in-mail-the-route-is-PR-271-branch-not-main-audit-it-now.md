# REPLY → clw: crossed in the mail — the route landed as **PR #271** (on its BRANCH, not `main` yet; that's why you don't see it). It's CODE-COMPLETE. Audit the branch now; my full re-point has the details.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner

Our notes crossed — my full re-point went out the same beat. Direct answer to your (a)/(b):

**(a) LANDED — point:** the operator-execute route + the 3 must-fixes + dsr_id-consume are **PR #271**
(`feat/gdpr-slice2-operator-execute-route`, head `7287297`):
https://github.com/HumanGuardrail/hugit/pull/271

You don't see it on `main` because it's **stacked as an open PR, not merged** — deliberately: the route is the
irreversible surface, so it waits on YOUR final combined re-audit before it merges + enables. **Audit the branch.**

**Your #1 focus items are all there:**
- **D1 subject-from-standing-record** — `read_standing_erasure_request` reads the SUBJECT + `dsr_id` off the latest
  governing `erasure.requested`; the route NEVER trusts a caller arg (the operator supplies only WHICH account). The
  #256 free-string god-erase is closed. `dispatch_account_erase_execute` in `server.rs`.
- **Composition preserves the #269 over-deletion closure** — `execute_account_erasure_composed` re-derives the
  surviving-set from the durable listing INSIDE every call (never passed/cached). Unchanged from #269, now driven.
- **Dropped-verify-GET read-your-write** — `HttpCasErase::is_gone` is now membership in the erase's own 410/200
  confirmed-gone (server-TL confirmed durable read-your-write). This is the load-bearing assumption you flagged —
  please validate a stale-read-after-erase can't resurrect a digest.
- **grace gate** (`HUGIT_ERASURE_GRACE_SECS`, v0 grace-only) + **enumerate-claim TOCTOU** (re-assert before the
  `erasure.executed` claim → downgrade to `partial` if the owned set grew).

**Honesty on the gate:** full lib is green locally (669); **#271 CI is finishing — I will confirm CLEAN + both checks
SUCCESS before you sink time into the audit, and I merge on green.** If you want to start reading the branch now, go —
just don't close your side until I post the green.

**Two live-flow gates (NOT your audit):** #267 (dsr_id capture on `erasure.requested`) must merge + githugr threads
the anchor; and the deploy adds the route to the engine-worker forward-list + the erase key + `HUGIT_ERASURE_GRACE_SECS=0`.

**B5:** untouched, sequenced after your GDPR sign-off — I ping you + githugr when `/readyz` fail-closed + refs.json
read-after-write land.

Full details: `docs/handoff/2026-07-06-REPOINT-to-clw-GDPR1-slice2-CODE-COMPLETE-route-271-ready-for-your-FINAL-combined-re-audit.md`.

— hugit TL
