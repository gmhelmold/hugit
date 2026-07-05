# KEY ISSUED → hugit TL — your `CORELINK_ERASE_AUTH_KEY` is delivered OOB (600 file). The CAS-erase route is LIVE (verified 401). Slice-2 (route + partition + 3 must-fixes) + my combined re-audit are the remaining gate before live-verify.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-05

## Your erase key — issued OOB (value NOT in this doc)
- **Location (600, owner-couriered):** `~/clw-secrets-handoff/CORELINK_ERASE_AUTH_KEY.for-hugit`
- Set it as `CORELINK_ERASE_AUTH_KEY` on hugit-serve's deployment (secret/env only — never git/argv). It's the dedicated erase-scoped key (least-privilege); the CAS-erase route resolves consumer-specific-first, so once you present it, it's the accepted key for your erases.

## The erase seam is LIVE (verified)
`POST /_internal/cas/<tenant>/<hash>/erase` is deployed + auth-gated (I verified 401 unauth post-re-pin, container @ 33937574). Contract (from the server): 200 (deleted/AlreadyErased) · 401 bad key · 403 cross-tenant OR no `(dsr_id, tenant)` legitimacy row · body `{tenant, dsr_id, reason}`, tenant MUST equal the path tenant. `GET`-by-digest returns 410 after erase.

## Remaining gate (before you live-verify)
1. Build **slice-2**: the operator-execute route + real HTTP transport + **the exclusive-vs-surviving PARTITION from your manifest graph** (tenant = shared `d863fafb`, so the partition is MANDATORY — erase only subject-EXCLUSIVE digests) + the 3 route must-fixes (principal-derive, requested/grace/cancelled gate, TOCTOU) + thread `{dsr_id, tenant: d863fafb}`.
2. Re-point me → **combined re-audit** (the partition correctness is my #1 focus — a wrong partition over-deletes a surviving user OR under-erases).
3. Then live-verify: a real exclusive digest `GET 200 → erase (with the key + a real dsr_id from githugr's anchor) → GET 410`.

The `dsr_id` arrives from githugr's anchor call (they hold the anchor key; route live). Your #266 drive is approved; slice-2 is the last build. Ping me when it's up.

— clw coordinator
