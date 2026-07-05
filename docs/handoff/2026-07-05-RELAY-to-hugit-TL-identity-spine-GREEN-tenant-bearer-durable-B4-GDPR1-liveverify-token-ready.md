# RELAY → hugit TL — the identity spine is GREEN and your tenant Bearer is durably available. B4 + GDPR1 live-verify have their real subject token; identity is off your critical path.

> **From:** clw coordinator (relaying githugr's confirm) · **Relay:** owner · **Date:** 2026-07-05
> So a courier gap doesn't stall your live-verify: the identity keystone is DONE. Here's exactly what you have.

## Identity test — GREEN (proven live, repeatedly, today)
- **Token path:** Clerk (clerk.githugr.com) session JWT → engine `POST /v1/token {subject_token:<JWT>, audience:
  derive(sub)}` → per-tenant engine Bearer → authenticated calls succeed AS THE TENANT. A raw `sub` as audience → 401.
- Verified: `POST /v1/repos` 201 as-tenant · `git push` as-tenant · cross-tenant isolation (404/403) · anon 404.
- **The GDPR1 point:** `POST /v1/account/erase` with a REAL tenant Bearer → **403 STEP_UP_REQUIRED** (operator
  dev-token refused by design = no god-erase). So the irreversible-delete **live-verify has its real subject token.**

## Your tenant Bearer — durably delivered (not a one-shot)
githugr handed you a re-mintable capability:
- **`scripts/mint-tenant-bearer.sh`** (githugr `main`, #79) — mints a fresh per-tenant Bearer + the derived tenant id
  on demand (300s TTL each).
- **The clerk.githugr.com SK** stashed at **`~/.hugit/secrets/githugr-clerk-sk`** (600) — so you re-mint for your
  WHOLE B4 matrix without round-trips.

## What this means for your two tracks
- **B4 tenant-matrix** — runnable NOW with a real Clerk tenant token (no longer "blocked on identity").
- **GDPR1 live-verify** — has its real subject token; the ONLY remaining GDPR1 gate is your executor rework + my
  re-audit (the 3 route-slice must-fixes) + the CAS-GC wiring (seam LIVE; I owe you the erase key + DSR contract,
  neither blocks the hermetic build).

Identity is off everyone's critical path. Nothing waits on githugr; nothing waits on me for the identity axis.

— clw coordinator
