# hugit → corelink-runners: §13 P2 envelope-transport decisions (3 items)

**From:** hugit techlead · **To:** corelink-runners techlead · **Via:** owner ·
**Date:** 2026-06-14 ·
**In response to:** `corelink-runners/docs/handoff/2026-06-14-hugit-p2-transport-and-hook-locality.md`
**Owner-ratified:** 2026-06-14 (all four calls below). **No frozen-type change** is
implied (Item 1 = Option A, not C); the §13.4 `IntentMetrics` conformance vector
(`conformance/IntentMetrics.json`, sha256 `2d8d2215…`) and the §13.5 wrapper markers
(`close_reason`, `capture_incomplete`) stay UNTOUCHED — this is transport, not shape.

---

## Item 1 — Subscriber identity → **Option A** (same tenant PAT)

The hugit envelope-consumer (the recorder, PS-1) runs **inside the same tenant
context** that acquired the lease — there is no split role/service on hugit's side.
So the consumer presents the **same Bearer PAT of the acquiring tenant** (Option A,
zero contract change). One-line change at your acquire composition root.

- Option **B** (CoreLink-issued per-tenant envelope credential, out-of-band) is the
  documented **upgrade path** if hugit ever splits envelope-ingest into a dedicated
  service with its own credential — it needs no frozen-type change, so we can adopt
  it later without churn. Not needed now.
- Option **C** (per-lease credential in `AcquireResponse`) is **rejected** — it is a
  frozen-type amendment for per-lease isolation we don't need (the tenant boundary is
  the isolation unit, ADR-0002).

## Item 2 — P2 transport contract (Q2a–Q2d)

> Scope note: a **NORMAL** close is already delivered inline to the live orchestrator
> with the exactly-once client ack — it needs no transport WP. Everything below is the
> **abnormal/forensic** path (crash/expiry, no live client).

- **Q2a — delivery mode → PULL.** hugit **polls** the fabric; it does **not** expose a
  push sink/webhook/queue. Rationale: hugit is a headless engine (the `/v1` surface is
  read-only; a write-sink is new infra hugit does not run). This matches your M1 pull
  shape — no new push machinery on either side.
- **Q2b — completion signal → (i) poll `meta`.** The consumer polls
  `GET /v1/leases/{id}/envelope/meta`, observes the **terminal state**, then drains
  `…/events`. **No fabric-side close emitter needed.**
- **Q2c — retention / drain window → durable until drained, bounded TTL fallback.**
  Because Item 3 makes the hook **durable** (below), the closed lease's envelope is
  retained until the consumer drains + acks it, OR a bounded TTL elapses
  (**recommend 24h** as the forensic drain SLA — covers consumer-side downtime). A
  promptly-polling consumer gets it in the common case; the TTL bounds storage.
- **Q2d — backpressure / ack → at-least-once + consumer dedup by `lease_id`.** The
  fabric may re-deliver; hugit **dedups by `lease_id`** — the hugit event-log is an
  **append-only idempotent** store, so recording the same envelope twice is a no-op
  (the lease_id IS the idempotency key). This keeps the fabric simple — **no durable
  exactly-once state machine required**. (Durable hooks from Item 3 *guarantee the
  at-least-once actually fires*; exactly-once becomes feasible but is unnecessary.)

## Item 3 — hook-locality forensic SLA → **YES: durable hook (the P2 fabric WP)**

**Owner-directed SOTA call:** the abnormal partial forensic envelope **must NOT be
silently dropped at N>1**. Today the `CaptureHook` registry is in-memory per fabric
instance, so when the reaper instance that wins the terminal-transition CAS is not the
instance holding the hook, the partial is lost. That silent loss is a **loose end** —
which hugit's zero-debt/impeccable doctrine forbids, even for forensic-only data.

**The distinction that makes this coherent with §13.5 Option B:** §13.5 stays
**best-effort about WHAT it captures** (a partial flush on abnormal close — owner
ratified). Item 3 is about **durably DELIVERING whatever WAS captured** — not letting
an infra accident (instance mismatch) drop a record we already produced. "Best-effort
partial content" + "durable delivery of that content" are compatible, and together they
are the SOTA path.

**Fabric WP (your build, your scoping):** persist the `CaptureHook` alongside the lease
(so any instance can flush on reap), **or** route the flush to the owning instance. We
defer the mechanism to you; the **requirement** is: at N>1, the abnormal partial
envelope is reliably produced + made drainable, never silently dropped.

- Billing impact: still **NONE** (flat pricing; forensic provenance, never a billing
  input) — the SLA is about audit completeness, not money.
- This also unblocks Q2c's durable-retention answer (the hook outliving its instance is
  what makes "retain until drained" possible).

---

## Summary for your fabric build

| Item | Decision | Fabric impact |
|---|---|---|
| 1 — subscriber identity | **A** (same tenant PAT) | one-line at acquire root; no frozen change |
| Q2a — delivery | **PULL** (hugit polls) | no push emitter needed |
| Q2b — completion | **poll `meta` → terminal** | no close-event emitter needed |
| Q2c — retention | **durable until drained / 24h TTL** | rides Item 3's durable hook |
| Q2d — ack | **at-least-once + hugit dedup by `lease_id`** | no durable exactly-once machine |
| 3 — hook-locality SLA | **YES — durable hook (P2 fabric WP)** | persist hook w/ lease OR route-to-owner |

**hugit side (our build):** the recorder (PS-1) polls `meta`, drains on terminal,
records into the append-log, dedups by `lease_id`. The ingest/`LogSource` is hugit's
lane (no dependency on your schedule beyond the endpoints already mounted).

— routed via owner; no `path`/`git` dependency between repos. The frozen contract is
unchanged; this is the production wiring of the already-frozen §13 mechanism.
