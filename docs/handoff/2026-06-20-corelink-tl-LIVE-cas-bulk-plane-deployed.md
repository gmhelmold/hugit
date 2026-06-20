# LIVE → hugit TL (cc owner) — CAS bulk plane deployed + PAT-smoke verified

**From:** CoreLink Server TL · **Date:** 2026-06-20 · **Relay:** owner. **Re:** your bulk client (#152)
waiting on the server bulk plane — it's live now.

## Bulk is on (real verification, not a probe)
The new prod container (`158d1615-r1`, carrying #370 batch upload/read/exists + #371/#372) is
**deployed to prod and live.** PAT-authenticated smoke:
```
POST https://corelink-api.humangr.com/v1/cas/<tenant>/batch-exists  →  HTTP 200
```
A real 200 from the bulk route (the old container 405'd here). This is the green light I said I'd
only give after a PAT-authenticated smoke — done.

## Host correction
Prod API = **`https://corelink-api.humangr.com`** (not `corelink-prod...`). Regionals are
`{sam,lhr,nrt,syd}.corelink-api.humangr.com` (geo-tier; the main host is what you call).

## Contract reminder (unchanged — your client already matches)
- batch-exists / batch-read framing is exactly what you confirmed against the #370 code (manifest +
  blank line + length-framed bytes; over-cap read → 413; over-cap exists → no per-hash regression).
- Your 405/404 → per-object auto-fallback (#153) stays valuable for any env where bulk isn't deployed.

Nothing pending on the server side. Your bulk client (#152) should now exercise the fast path live.

— CoreLink Server TL · routed via owner
