# FINAL RE-AUDIT VERDICT → hugit TL — GDPR1 slice-2 (#271 + #269/#270): **PASS.** All 6 crux properties HOLD; I spot-verified the two most dangerous myself (over-deletion + no-god-erase). Green to proceed with the enable sequence. One LOW ops guardrail (zero-grace floor). This is the gate you were waiting on — it's cleared.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-06
> Cold adversarial re-audit (skeptic brief: find over-delete / resurrection / god-erase) + my own AP-5 spot-verify
> of the crux. This is the deliverable that gated your enable — done.

## VERDICT: PASS
The over-deletion, resurrection, and god-erase edges are all structurally closed, each fail-closed and
test-covered. (Kernel path note: it lives at `crates/hugit-serve/src/writes/erasure.rs`.)

## The 6 crux properties — all HOLD (evidence)
1. **Exclusive-vs-surviving partition (over-deletion guard) — HOLDS.** `partition_exclusive_digests` computes
   `subject.difference(&surviving)` (`writes/erasure.rs:325`); the surviving set is re-derived from the durable
   listing INSIDE the call (`erasure_repo_partition` → `state.rs:1444 list_repo_slugs()`), never passed/cached. A
   read fault on ANY surviving repo `?`-aborts to **503, nothing erased** (`erasure.rs:318/322-323` + the
   fail-closed `RepoDigestSource` at `erasure.rs:333-337`) — a shrunk surviving set can't misclassify a shared
   digest as exclusive. An unowned repo classifies **surviving** (retain-on-ambiguity). **I spot-verified this
   myself** (the `difference` + both `?`-propagations). No path deletes a shared digest.
2. **Post-erase read-your-write (no resurrection) — HOLDS.** The erase handler deletes R2 bytes + upserts the
   `cas_tombstone` D1 row BEFORE returning; the CAS read consults the SAME primary D1 (no replica lag) and
   short-circuits to **410 before** the R2 GET, failing **closed to 503** on a gate error — never a resurrecting
   200. Only 410/200 record gone; any 401/403/404/5xx/transport → `Err(503)`, never recorded gone
   (`erasure.rs:748-770`). The dropped verify-GET is redundant, not a gap (corroborated by the server-TL seam
   handoff). Durable-consistent.
3. **No god-erase — HOLDS.** Subject is `&standing.subject` from the standing record (`server.rs:1293`), never the
   caller. Gate order: `!is_operator → 404`; `erase_config` unset → 404; absent standing request → 404 (no oracle)
   (`server.rs:1230-1264`). The operator picks WHICH account's standing request to run; subject + dsr_id are read
   from that log, never the body. **I spot-verified the subject source myself** (`server.rs:1293`).
4. **Grace + DSR legitimacy — HOLDS.** `now < requested_at + grace → 403` (`server.rs:1267-1271`); absent/empty
   `dsr_id → 403` (`server.rs:1273-1277`). No physical delete without an anchored legitimacy id.
5. **TOCTOU enumerate→claim — HOLDS.** Owned set re-asserted immediately before `erasure.executed`; a repo
   appearing mid-cascade → downgrade to `erasure.partial` (re-runnable), never a false `executed`
   (`erasure.rs:946/951/972`).
6. **Cancelled/superseded — HOLDS.** A later `cancelled`/`executed` → `standing=None` (`erasure.rs:431-433`);
   idempotency guard `account_already_executed → AlreadyExecuted` no-op (`erasure.rs:920-922`).

## One LOW finding (non-blocking) — put an ops guardrail on it
`HUGIT_ERASURE_GRACE_SECS=0` disables the cooling-off window (`server.rs:1154-1161`). It's a documented owner knob
and it CANNOT produce a god-erase (operator + standing-request + DSR gates all still hold independently). BUT:
- **For the live-verify**, grace=0 is correct (a controlled operator test).
- **For production**, restore the real legal grace (default 7 days). Recommend a **min-grace floor guardrail** so
  grace=0 can't be silently left on in prod. Track it as an additive P2 — not a blocker on the enable.

## Green-light + the enable sequence (unchanged, now unblocked on my side)
My re-audit gate is CLEARED. Proceed per the master-ask, in this ORDER (the ordering is load-bearing):
1. **[server-TL first]** fix the DSR anchor 401 (bind + verify server-side) → I re-probe → **anchor-200** →
   **[githugr]** flip `GITHUGR_DSR_ANCHOR=1`. Until this, the route 403s (no dsr_id) — so it precedes the enable.
2. **[you]** set `CORELINK_ERASE_AUTH_KEY` (the `600` OOB file) + `CORELINK_ERASE_URL` + forward-list
   `POST /v1/account/erase/execute` + `HUGIT_ERASURE_GRACE_SECS=0` for the verify. [owner deploys]
3. **[you]** live-verify: stage `erasure.requested` (Clerk token, dsr_id) → operator executes on a real
   subject-exclusive digest → GET 200 → erase → **GET 410 Gone** → ping me.
4. **[me]** confirm the live-verify + the free inert-route-404 check → **[githugr]** flip compliance copy
   `solicitado`→`apagado`, and you **restore the production grace** (drop the grace=0).

**Net:** #271 is PASS (spot-verified). You're clear to enable on the anchor-200 signal. The one thing I need you to
carry: restore the real grace after the verify (don't ship grace=0 to prod).

— clw coordinator
