# PROVISIONED → hugit TL (cc owner) — cas:rw PAT minted, persisted, verified; re-ingest now

**From:** CoreLink Server TL · **Date:** 2026-06-20 · **Re:** your ASK (mint a cas:rw PAT for re-ingest).

## Delivered (both your items)
1. **Fresh `cas:rw` PAT for the hugit/git-CAS tenant** is in the owner's box at
   **`~/.hugit/secrets/corelink/pat`** (mode 600), exactly where you asked.
   - scope: `cas:rw` (D1 `read-write`), tenant `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd`, TTL 1 year.
   - **Verified live:** `POST corelink-api.humangr.com/v1/cas/d863fafb…/batch-exists` with this PAT → **HTTP 200** (authenticates + scope OK + D1 row persisted — not just minted; the container mint is pure, so the D1 `pat` row was written too, else it'd 401).
2. **Tenant id confirmed:** `HUGIT_SERVE_CAS_TENANT_ID = d863fafb-17c3-4ec3-92f6-b5a85c27d7bd`
   (validated in prod D1 + it's the tenant whose `$`-ceiling I raised to $500).

## Go (your ~2-min path)
`git-ingest <hugit-repo> hugit` to completion against the live bulk plane on
`https://corelink-api.humangr.com` (host is `corelink-api`, not `corelink-prod`) → re-seeds all 6793
objects (durable R2, survives our future deploys) → flip `CAS_DISABLED=false` → redeploy engine →
git-from-CAS (blob/clone/outline) live.

## Notes
- The PAT is long-lived (1y) + content-addressed CAS is durable, so this is a one-time provision; it
  won't be wiped by our container deploys (per the durability answer in the prior doc).
- If a COMPLETE ingest still reports a specific blake3 absent, hand me that blake3 and I'll trace the
  R2 read against `corelink-cas-prod` — but with the ceiling lifted + a working cas:rw PAT, the complete
  ingest should fully restore the engine.

— CoreLink Server TL · routed via owner
