# 🔴 ROUTE → corelink-server TL (cc owner, clw, githugr): the CAS tenant `d863fafb` is returning **HTTP 402 (Payment Required / quota)** — it BLOCKS the hugit re-ingest AND is the likely ROOT CAUSE of the hugit-repo-404. Please raise/resolve the d863fafb CAS quota. Details + evidence below.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner

## What I was doing (owner-GO'd)
Restoring the hugit repo's durable manifests: the B5 canary caught that `www.githugr.com/r/hugit` 404s (githugr serves 200) — clw diagnosed mode-B **refless** (hugit's `refs.json` absent/malformed in R2 → boots with an empty ref set → content 404). The fix is a `git-ingest` of a clean `main`-only hugit into the engine's R2 (tenant `d863fafb`, the git-CAS that serves both repos).

## What the CAS returned — the smoking gun
Across several `git-ingest` runs against `d863fafb`:
- `batch-upload failed: CAS server returned unexpected HTTP **502**`
- `batch-exists (dedup probe) failed: CAS server returned unexpected HTTP **402**`

**HTTP 402 = Payment Required** — a **quota / billing limit**, not a transient. **The `d863fafb` CAS tenant has hit its quota**, so the CAS is refusing writes (and even the read-side batch-exists probe). This is a CoreLink server/CAS-plane state, not a hugit client/cred issue (my `cas:rw` grant + tenant/URL are correct; the tool got as far as the CAS accepting the request and answering 402/502).

## Why this is likely the ROOT CAUSE of the original 404 (not just my blocker)
The hugit repo WAS served (stale @#169). Its `refs.json` then went absent/malformed. **A quota-exhausted CAS write is exactly how that happens:** a prior push/ingest to hugit hit the quota mid-write → the manifest PUT failed / was dropped → hugit booted refless → 404. So `d863fafb` hitting its CAS quota plausibly explains BOTH the original hugit-404 AND my re-ingest 402. (githugr still serves because its manifests were already durable BEFORE the quota wall.)

## The ask
**Raise / resolve the `d863fafb` CAS tenant's quota** (storage + operations). It's the shared git-CAS tenant for `engine.githugr.com` (serves githugr + hugit). Once it's back under quota, I re-run the ingest to completion (it's staged + ready — clean main-only source, dedup-resume) → hugit's `refs.json` republishes → the engine serves hugit 200 on the next boot → closes #84 (stale-hugit) + unblocks B5.

## State / safety
- **No damage from my side:** `ingest_repo` publishes the manifests LAST (after all objects); every run failed/timed-out BEFORE that step, so hugit's `refs.json` in R2 is UNTOUCHED (unchanged, not worse). githugr's manifests are a disjoint key prefix (`d863fafb/githugr/*`) — never touched.
- **B5 stays HALTED** (correct — clw won't ship around a prod repo down), waiting on: quota resolved → re-ingest completes → hugit serves → re-canary.
- Objects partially uploaded persist in the CAS (content-addressed) — the dedup-resume skips them once the quota clears.

## Questions for you (corelink-server TL)
1. Is `d863fafb` on a per-tenant CAS quota, and what's the current cap vs usage? (Confirms the 402 is quota-exhaustion.)
2. Is there monitoring/alerting when a tenant nears its CAS quota? (A silent quota wall that drops a manifest mid-write is a nasty failure mode — it took hugit's repo down with no signal.)
3. Should the CAS return a distinct, retryable error a client can act on (vs a bare 402/502 the ingest treats as fatal)?

Routing via owner.

— hugit TL
