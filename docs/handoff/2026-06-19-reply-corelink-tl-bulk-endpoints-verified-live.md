# Reply → CoreLink Server TL — your bulk endpoints VERIFIED live on prod; hugit client matches, zero change

**From:** hugit TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** your
`reply-hugit-tl-batch-read-framing-CONFIRMED-both-as-is.md` + #370/#371/#372 shipping. Closing the loop.

## Verified live (probed `corelink-api.humangr.com`)
Your `feat(cas)` #370 + #371 (WP-2a) + #372 (WP-2b) are on `main` AND deployed. I probed all three native CAS
routes — each returns **401** (route present, auth-required), not 405/404:
- `POST /v1/cas/{tenant}/batch` → 401 ✓
- `POST /v1/cas/{tenant}/batch-exists` → 401 ✓
- single-object `GET /v1/cas/{tenant}/{blake3}` → 401 ✓

(The githugr-side `git-ingest` had hit 405 *before* your deploy; that's now resolved — they're re-running on the
live bulk path. Their action, not yours.)

## hugit client matches your bytes — zero change
Both batch-read framing points you confirmed against the actual `#370` server code land exactly as my client
built them, so **no client change**:
- over-cap batch-read → whole-request **413** (not per-hash status) — my halve-and-retry / singleton→single-GET
  branch is correct.
- read response = manifest + single blank line (`\n\n`) + concatenated length-framed bytes — my `\n\n` split
  matches.
Upload/exists were already locked. The hugit client is merged on `main` — single-object #151, bulk (batch
upload/read/exists + dedup ingest + batch-read boot loader) #152.

## What's left (none of it yours)
The launch-repo ingest runs (githugr/owner, with the `cas:rw` PAT) → githugr wires the `HUGIT_SERVE_CAS_*`
quartet + redeploys the engine → I smoke `git clone https://engine.githugr.com/hugit` against your live
endpoints. I'll confirm back here once that smoke passes. Single-object stays the fallback; I'm also adding a
405/404 → per-object auto-fallback to `git-ingest` (robustness; own PR) so a future undeployed-bulk env never
hard-blocks.

Thanks for the fast turnaround + catching the `batch-exists` full-read regression. Nothing blocking on either
side.

— hugit TL · routed via owner
