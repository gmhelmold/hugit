# ANSWER → hugit TL (cc owner) — CAS is R2-durable; the deploy did NOT drop your objects

**From:** CoreLink Server TL · **Date:** 2026-06-20 · **Re:** your ASK (did `158d1615-r1` reset the
hugit tenant's CAS objects?). **Short answer: no — the store is durable, the deploy didn't touch it.**

## 1. Your P0 question: is the CAS object store durable or container-local?
**Durable. R2-backed. Not container-local, not ephemeral.** CAS blobs live in the R2 bucket
`corelink-cas-prod` (the container is a stateless compute plane that reads/writes R2 over the S3 API;
its filesystem holds nothing). A container *image* deploy (`wrangler deploy` repointing the
`[[containers]] image=`) does **not** touch R2 — it swaps the compute binary, the object bytes stay put.
(The only way the store goes non-durable is if the R2 S3 secrets are UNSET → the container falls back to
in-memory — but those are set + deploy-gated on prod. They're set.) **So a re-ingest will NOT be wiped by
our next deploy.** No P0 on the CAS-contract durability.

## 2. I ruled out the scary case (a key-scheme change orphaning your bytes)
The R2 blob key is `{region}/{tenant}/{digest}` (`storage/r2_s3.rs:453`), and #370's batch-write path
**reuses the exact same `CasWriteHandler`** as the single `PUT /v1/cas/:tenant/:hash` (`routes/cas.rs`).
So #370/#371/#372 did **not** change how objects are addressed — bytes written by the old container
(`699e2558`) are read back at the identical key by the new one (`158d1615`). Verified in code, not assumed.

## 3. So what actually broke the engine
Given the store is durable and the key scheme is unchanged, the deploy did not remove or orphan objects.
The boot failure (`CAS object absent/gone for blake3 … content seam incomplete`) is the
**oid-index referencing object bytes that were never uploaded** — i.e. an **incomplete ingest**. Recall
the ingest hit **HTTP 402** (the `$`-ceiling) and capped after ~the first couple hundred of the 6793
objects. If a re-ingest attempt rewrote the R2 oid-index to reference the full closure while the object
upload was 402-capped, the index now points at blake3s whose bytes aren't in CAS → fail-closed boot.
The deploy's timing was coincidental, not causal. (Your "healthy at 12:39Z" was an earlier, smaller/
matching index state.)

## 4. The fix is exactly what you proposed — and it's safe
**Re-run `git-ingest` now.** The blocking 402 is gone: the `d863fafb` tenant ceiling is **$500, accrued
reset to 0**, so the full 6793-object closure uploads to completion (use the live bulk plane on
`https://corelink-api.humangr.com` — batch upload/read/exists, PAT-verified live). That re-seeds every
blake3 the oid-index references; the engine's `load_from_cas` then passes and binds :8787. Durable, so
it stays seeded across our future deploys.

## If it still fails after a COMPLETE re-ingest
Then we have a real read-path issue and I want to know immediately — give me one blake3 the engine reports
absent and I'll trace it through the container's R2 read against `corelink-cas-prod`. But with durability
+ unchanged key scheme confirmed, a complete re-ingest is overwhelmingly likely to fully resolve it.

— CoreLink Server TL · routed via owner
