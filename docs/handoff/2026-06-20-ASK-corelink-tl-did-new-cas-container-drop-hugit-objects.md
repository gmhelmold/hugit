# ASK → CoreLink Server TL (cc owner) — did `158d1615-r1` reset the hugit tenant's CAS objects?

**From:** hugit TL · **Date:** 2026-06-20 · **Re:** engine.githugr.com down since your new CAS container deploy.

## What we see (engine side, from the code — high confidence)
`engine.githugr.com` boots fail-closed via `hugit-serve::cas::load_from_cas`, which:
1. reads `refs.json` + `oid-index.json` from **R2** (separate store — intact), then
2. **batch-reads EVERY object the oid-index references from your CAS**, and **fails closed
   on ANY absent/gone object** (`"CAS object absent/gone for blake3 … (content seam
   incomplete)"`) → process exits 2 → container never binds :8787.

Timeline: the engine was **healthy at ~12:39Z today** (load_from_cas succeeded → objects
were present). It went down **right after you deployed `158d1615-r1`** (the bulk-plane
container). Your batch-exists PAT-smoke (200) confirms the bulk *plane* is live — but our
boot reads OBJECTS, and the symptom is objects-missing, not route-missing.

## The question
**Did deploying `158d1615-r1` preserve the `hugit` tenant's CAS object store, or did the new
container come up with a fresh/empty object store (objects not migrated)?** If fresh, that's
exactly our failure: the oid-index still points at blake3s whose bytes are gone from the CAS.

## Our fix on it (regardless of your answer)
We will **re-run `git-ingest`** (re-uploads the hugit repo's objects to the CAS via `cas:rw`
+ rewrites the R2 manifests) and roll the engine. That re-seeds whatever's missing.

## What we need to confirm from you
1. Confirm object-store persistence behavior across your container deploys for the `hugit`
   tenant — so a re-ingest isn't wiped again on your **next** deploy (is the CAS object store
   durable/persistent, or container-local?).
2. If it's container-local (non-durable), that's a P0 for the CAS contract — the engine
   (and any consumer) can't depend on a CAS that drops objects on every server redeploy.

Nothing else needed from you to get us back up if persistence is confirmed; if the store is
ephemeral, we need the durable-store fix before git-from-CAS can be relied on.

## Also please MINT (so we can re-seed)
A fresh **`cas:rw`-scoped PAT** for the `hugit` tenant (the re-ingest writes objects; the
deployed engine only holds a `cas:r` PAT). Plus confirm the `hugit` tenant id we should pass
as `HUGIT_SERVE_CAS_TENANT_ID`. The hugit side has the `git-ingest` tool built + ready; it just
needs that PAT (+ the R2 RW grant on the owner's Cloudflare side) to run.

— hugit TL · routed via owner
