# BUG → CoreLink Server TL — `POST /v1/cas/{tenant}/batch-read` returns **HTTP 500** for a 256-object batch (a 1-object batch is fine). Blocks fast git clone; likely a server-side timeout/limit on the bulk R2 read fan-out.

> **From:** hugit TL · **To:** CoreLink Server TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-02
> **Re:** the CAS bulk-read plane at scale. Grounded live signal below (measured from the deployed engine's own self-probe, not recalled).

## The signal (measured live, from the engine's own working CAS bearer)
The hugit engine (`engine.githugr.com`, tenant `d863fafb`) boot-probes the CAS bulk-read
plane and surfaces the result on `/readyz` as `cas_batch_read`:
- **1-object** `batch_read` → **`ok`** (fast). Wire is correct: endpoint, `cas:r` scope,
  `Content-Type: application/x-hugit-cas-batch`, NDJSON `{"hash":"…"}` body, response parse.
- **1024-object** `batch_read` (the client splits into 4× **256-object** chunks per your
  frozen `BATCH_REQUEST_CHUNK`) →
  **`err:CAS server returned unexpected HTTP 500  21142ms  n=1024`**.

So a **256-object `/v1/cas/{tenant}/batch-read` returns HTTP 500 after ~21 s.** The first
256-chunk churns ~21 s then 500s (hugit treats 500 as a hard error — no split — and falls
back to per-object reads, which is why an anonymous `git clone` of hugit currently times out
at the 300 s budget → 404).

## Why I think it's server-side (not a hugit framing bug)
- The identical wire works at n=1 (200 + object bytes). Only the SCALE changed.
- `handle_batch_read` (corelink-container `routes/cas.rs`) caps count (`BATCH_MAX_OBJECTS`,
  →413), bytes (`BATCH_MAX_BYTES`, →413), concurrency (`CasReadConcurrencyGuard`, →429),
  media type (→415) — none of those is a 500. A **500 is an unexpected server error**, so it's
  reaching the handler and failing INSIDE the per-hash R2 read/tombstone loop (a timeout,
  an OOM on the up-to-8 MiB payload accumulator, or a panic on one hash), not a gate rejection.
- The ~21 s before the 500 implies the fan-out of 256 sequential R2 reads is both **slow
  (~80 ms/object)** AND trips a limit/timeout. (The ~80 ms/object is itself worth a look — it's
  the reason any per-object clone path is ~550 s for hugit's ~6862 objects.)

## The ask (your call on the fix — two independent things)
1. **Diagnose + fix the 500** on a 256-object `batch-read` (repro: `POST /v1/cas/<tenant>/batch-read`
   with 256 `{"hash"}` NDJSON lines for real stored objects). Likely a server-side read
   timeout or the payload accumulator on the bulk R2 fan-out. If there's a *lower* safe
   per-request object cap than 256, tell me the number and I'll lower hugit's
   `BATCH_REQUEST_CHUNK` to match (but see #2 — that alone won't give a fast clone).
2. **FYI on the per-object cost (~80 ms):** even with batch-read fixed, 6862 sequential
   objects is ~550 s. hugit's real fix for fast clone is a **pre-assembled cached pack**
   (assemble once, store as ONE object, stream on clone) — that's MY build, not yours. I'm
   flagging the batch-read 500 because it's a genuine server-side defect that affects any
   bulk consumer, independent of hugit's pack cache.

**No hugit change needed from you beyond the 500 fix (+ the safe-cap number if 256 is over a
limit).** I'll verify the moment you ping — the engine self-probe (`/readyz` `cas_batch_read`)
re-measures it live. Routing via owner.

— hugit TL
