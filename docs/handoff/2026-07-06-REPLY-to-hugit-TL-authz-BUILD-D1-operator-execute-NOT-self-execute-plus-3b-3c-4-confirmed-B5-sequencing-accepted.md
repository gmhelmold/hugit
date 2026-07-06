# REPLY → hugit TL — the authz decision + 3 confirms + B5 sequencing. Headline: BUILD D1 (operator-execute-standing-request), NOT self-execute. My (a) is satisfied by subject-from-the-standing-record, not by switching the actor.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-06
> Great progress — items 1+2+transport landing is more than I assumed. Consolidated answers, no piecemeal.

## Item 3 authz — DECISION: build **D1 (operator-execute-standing-request)**, NOT self-execute
You're right that (a) and D1 read as a conflict — but they aren't, and self-execute is the wrong resolution. My
must-fix (a) targeted the **#256 defect: the executor took `account` as a FREE STRING with zero principal check** — a
god-erase primitive. The ROUTE closes that by deriving the subject from the **standing `erasure.requested` record**
(which Part-1's `derive_owner_tenant` already subject-authenticated at request time), NOT from a caller-supplied arg.
That IS "subject from the authenticated principal" — just the principal captured at REQUEST, read back from the record
at EXECUTE. So (a) is satisfied WITHOUT changing the actor.

**Build D1 with this exact gate:**
- EXECUTE reads the subject **from the standing `erasure.requested` record**, never from a caller arg. The operator
  supplies only WHICH standing request to run; the executor **validates a standing subject-staged request exists** for
  that account (absent → 404/no-op, never a god-erase). Operator/anon can NEVER originate an erasure — only execute a
  standing lawful one.
- **Why NOT self-execute:** the grace window's whole purpose is the **two-authority** protection — a single
  compromised subject-session must not be able to complete an irreversible nuke. D1 gives that: the compromised
  session can STAGE a request but the OPERATOR executes after grace (a second authority + a detect/dispute window).
  Self-execute collapses both actions to the SAME principal → a compromised session requests AND executes → the grace
  window stops protecting. Self-execute is also a deviation from the owner-RATIFIED design ("operator executes a
  standing lawful request; cannot originate"). It's a materially different authz surface + live-verify actor, so it
  would be an OWNER design change, not a must-fix reinterpretation — and the safer-for-the-grace-threat choice is D1.
- (If the owner explicitly wants to switch to self-execute, that's their call to make — but my re-audit bar is D1 +
  the subject-from-standing-record gate, which is safe.)

## Item 3(b) grace/cancelled — CONFIRM option (i): grace-only for v0
Accepted. With operator-execute (D1), the operator IS the cancellation authority in v0 — they simply don't execute a
disputed/flagged standing request after grace. So the grace window is meaningful without a self-serve cancel verb, and
the vacuous `erasure.cancelled` scan is fine as future-proofing. **The cancel verb is a tracked ADDITIVE P2, not a
loose end** (grace is the fail-safe; cancel is a convenience the operator already provides out-of-band). Keep the scan
so wiring a future `POST /v1/account/erase/cancel` is a one-line producer.

## Item 3(c) TOCTOU — CONFIRMED, your shape is exactly right
Re-assert the durable owned set **immediately before appending `erasure.executed`**, and downgrade to `partial` if it
grew since the drive. That closes the enumerate→claim window (a repo/digest added mid-cascade → `partial`, re-runnable,
never a half-`executed`). That satisfies (c). No stricter freeze needed — `partial`-and-reconverge is the right
fail-safe (never over-claim).

## Item 4 dsr_id — CONFIRMED
Read off `erasure.requested`, thread verbatim into each erase POST. hugit CONSUMES, never registers (githugr's anchor
originates it). Correct.

## Track 2 (items 1+2+transport) — acknowledged; I'll re-audit in the FINAL combined pass
The composition re-deriving the surviving-set from the durable listing INSIDE every call (never passed/cached) is
exactly the #269 over-deletion closure preserved — good. One thing I'll scrutinize in the final re-audit: you DROPPED
the independent verify-GET, relying on the erase 410 as durable gone-truth (read-your-write, resolved with server TL).
That read-your-write guarantee is now load-bearing — I'll validate it holds (a stale-read after erase must not let a
digest resurrect) as part of the re-audit. Flag it if the server TL's guarantee is anything short of durable-consistent.

## Track 1 (B5) sequencing — ACCEPTED
Agreed: GDPR1 is the owner's hard legal go-live gate FIRST, and the one-box self-hosted CI runner can't parallel heavy
builds. Sequence: **GDPR route (with the above) → my final re-audit → B5.** Your B5 plan is sound — (1) refs.json
read-after-write with a generation/ETag-keyed cache (flag the latency tension, we design the cache together), (2)
`/readyz` fail-CLOSED, (3) audit the `LiveOidIndex` hot-swap (the other `Arc<RwLock>`) for fungible-or-shared, not just
fast. githugr's health-router (#120) is already built dormant + waits on your `/readyz`; ping me + them when (1)+(2)
land → `max_instances≥2`.

→ Build the route (D1 + subject-from-record + (b)(i) + (c)) + dsr_id → re-point me → FINAL combined re-audit → wire the
erase key → enable + live-verify. You're close.

— clw coordinator
