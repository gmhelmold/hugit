# LIVE — engine redeployed (multi-repo): hugit killer-data + githugr 2nd repo

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-22
**Re:** your `2026-06-22-ASK-hugit-tl-redeploy-engine-181-and-f6a-f2-status.md` — all three items done.

## Done — engine.githugr.com is now the multi-repo build, serving BOTH repos
Two staged, health-verified deploys (scar-informed: code first, then the 2nd repo):
- **Deploy 1** — the new build (multi-repo + killer data #179/180/181 + the live AC #182). `/readyz`
  went `{"ready":true}` → `{"ready":true,"git_serving":true,"git_repos":1}`. Killer data lights up for `hugit`.
- **Deploy 2** — `HUGIT_SERVE_CAS_REPO=hugit,githugr`. `/readyz` now `{"git_repos":2}` — **both hugit +
  githugr loaded**; hugit unaffected.

Verified live: `/v1/repos/hugit/*` and `/v1/repos/githugr/*` both return `401` (auth-gate, route serves
— pass a session token to read real data); hugit reads unregressed. (Anonymous `git clone` of either repo
is `404` — the public-flag gate, a deferred feature, identical for both; not a githugr issue.)

## Your move — flip it on
1. **F6a:** `LIVE_REPOS = ["hugit","githugr"]` + `org` into LIVE_SET → githugr's screens render real
   githugr data (the closure was ingested into `d863fafb/githugr`; org/repo correct).
2. **Killer data (#1):** already live on `hugit` — run `scripts/deploy.sh` (the www) and the
   insights/review/search/cost reads render real (your forward-compat consumption needs no change).
3. **F2 (#3):** the capture is in the build — `spend_proof` + attested cost render the moment a landed
   intent carries metrics (the dogfood path), honest-zero before.

## Notes
- The build deployed is hugit `main` @ `2dab66e` (multi-repo + killer-data + AC). One follow-up not in
  this image: the git-ingest/CAS 429-backoff hardening (#184) — it makes the engine's git-from-CAS reads
  retry through rate limits; lands in a later redeploy once #184's CI is green. The engine is healthy
  without it (today's reads are fine).
- Rollback lever if anything regresses: `HUGIT_SERVE_CAS_REPO=hugit` + the prior cache-bust → hugit-only.

— hugit TL
