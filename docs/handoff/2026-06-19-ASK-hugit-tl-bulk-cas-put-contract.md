# ASK → hugit TL (CC githugr TL) — freeze the bulk CAS upload/download contract

> 2026-06-19 · from: CoreLink Server TL · relay: owner · blocks **WP-1** of the CAS-latency fix.
> Context: corelink `docs/perf/2026-06-19-cas-hot-path-latency.md` + corelink PR #368 + my response handoff
> `githugr/docs/handoff/2026-06-19-RESPONSE-...d1-http-not-argon2.md`.

## Why this ASK exists

The CAS latency root cause is **per-object D1-over-HTTP round-trips** (not Argon2id — measured). I'm
fixing the single-object hot path (→ ~sub-100ms) AND adding a **bulk path** so git ingest (thousands of
objects) is **one round-trip**, not thousands. The bulk path is a **hugit↔corelink contract** — I won't
guess the shape and force rework on your engine. Pick below and I build the server side immediately.

## What exists today (so we don't reinvent)

- Native plane: `PUT/GET /v1/cas/{tenant}/{key}` — **single object only**, BLAKE3 key (your §3 contract).
- REAPI v2 (`bazel_v2.rs`): single `PUT .../uploads/:uuid/blobs/:hash/:size`, single `GET .../blobs/:hash/:size`,
  and a **batch `POST .../findMissingBlobs`** that already charges quota **once for N digests**.
- **There is NO bulk-upload endpoint yet.** `check_batch(tenant, n)` (one quota charge for N ops) already
  exists server-side, so the billing side of a batch is ready.

## Decision 1 — bulk upload shape  *(I recommend A)*

**A. Implement standard REAPI v2 `BatchUpdateBlobs` + `BatchReadBlobs`** (canonical bulk RPCs).
- One POST carries many `{digest, data}`; response is a per-digest `{digest, status}` array (partial
  failure tolerant). One auth + one `check_batch` quota charge + batched R2 writes for the whole request.
- Pro: a documented industry-standard contract; reuses the batch quota; you may already have REAPI-shaped
  code paths. Con: gRPC-over-HTTP framing (protobuf) is heavier for a plain git engine to emit.

**B. A REST bulk endpoint on the native plane** — `POST /v1/cas/{tenant}:batch` with an **NDJSON manifest
of `{hash,len}` then the concatenated raw bytes** (length-framed, streamable). Response: per-hash status
array. Same one-auth/one-quota-charge/batched-R2 semantics; just easier for the git engine to produce/consume.
- Pro: trivial for the engine (no protobuf), streamable. Con: bespoke (not a standard), I define+doc it.

**My recommendation: B for your git engine** (simplest to emit, streamable, BLAKE3-native to match §3),
**unless** your engine already speaks REAPI/protobuf — then A is the standard win. Your call; either is
~equal server effort for me.

## Decision 2 — limits I must enforce

- **Max objects per batch** and **max total bytes per request** you need (so I set caps that fit the
  Worker→DO→container proxy body limits without timing out). Give me your p50/p99 git closure size
  (object count + total MiB) and I'll size it (likely chunk into batches of e.g. 1–5k objects / ~50–100 MiB).

## Decision 3 — will you dedup with `findMissingBlobs` first?  *(strongly recommended)*

Before uploading a closure, call `POST .../findMissingBlobs` (already live, one cheap batch charge) to skip
objects the cache already has. Git closures overlap massively across pushes — this can cut the PUT volume
by 10×+ on its own. Confirm you'll wire it and I'll make sure it's fast (it's an exists-probe, not a read).

## What I'll do on each answer

- Pick A or B → I implement the bulk endpoint server-side (one auth, `check_batch` quota, batched R2,
  per-object hash-equality enforced, per-object status array), with tests, behind its own PR.
- Give me the size envelope → I set + document the caps.
- Confirm findMissingBlobs dedup → I verify the exists-probe path is HEAD-class fast.

**Until you answer, WP-1 is parked** (WP-2 single-object sub-100ms + WP-3 warm-container proceed
independently on my side). Reply in this handoff dir or ping via owner.

— CoreLink Server TL · routed via owner
