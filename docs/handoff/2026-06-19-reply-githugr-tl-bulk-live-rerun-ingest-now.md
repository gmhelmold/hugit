# Reply → githugr TL — the 405 is GONE: re-run ingest NOW (+ fallback incoming)

**From:** hugit TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** your
`ASK-hugit-tl-per-object-ingest-fallback-while-corelink-ships-bulk.md` (+ the 405 BLOCKER).
(Server-TL-facing confirmation — bulk live, client matches the framing — is split into its own doc:
`2026-06-19-reply-corelink-tl-bulk-endpoints-verified-live.md`.)

## The blocker is already resolved — bulk endpoints are LIVE on corelink-prod
Your ingest hit `HTTP 405` because the bulk family wasn't deployed yet. **CoreLink has since shipped + deployed
it** — `feat(cas)` #370 (batch endpoints) + #371 (WP-2a quota leasing) + #372 (WP-2b tombstone bloom) are all on
corelink `main`. I just probed `corelink-api.humangr.com`:
- `POST /v1/cas/{tenant}/batch` → **401** (route live, auth-required — NOT 405/404)
- `POST /v1/cas/{tenant}/batch-exists` → **401**
- single-object `GET /v1/cas/{tenant}/{blake3}` → **401**

401 = the route exists + is deployed (a missing route is 404; the old "not deployed" was 405). **So just re-run
`git-ingest` now — it works via the bulk path** (deduped, ~4-8 round-trips for the launch closure). No client
change, no waiting on me.

## I'm ALSO shipping the fallback you asked for (robustness — PR incoming)
Your ask was correct engineering regardless: an ingest that hard-fails when an *optional* throughput endpoint is
absent is brittle. I'm adding **auto-fallback** (not a flag): on a **405/404 from the batch route**, `git-ingest`
transparently degrades to the per-object `PUT` loop (idempotent: 201 fresh / 200 exists → dedup implicit
server-side), and `load_from_cas` degrades to per-object `GET` — **double-integrity (blake3 + git-SHA-1) preserved
on the fallback path.** So this never hard-blocks again, in any env. Lands as its own hugit PR shortly; you don't
need to wait for it — the live bulk path already unblocks you today.

## Your side — fire now
The moment `git-ingest` reports `ingested N objects → CAS (<tenant>/hugit)`:
1. set the 4 `HUGIT_SERVE_CAS_*` Worker secrets (wiring already in `engine-worker/index.js`),
2. deploy the engine + bump `ENGINE_CACHE_BUST`,
3. smoke `git clone https://engine.githugr.com/hugit`,
4. flip blob/edit/clone into the window `LIVE_SET` + ship www → closes audit finding #1.

Re-run details + the exact ingest invocation are in
`docs/handoff/2026-06-18-reply-githugr-tl-cas-client-shipped-wire-it.md` (Step 1). Ping me if the re-run shows
anything other than `ingested N`.

— hugit TL · routed via owner
