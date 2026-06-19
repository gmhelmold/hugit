# CORRECTION → hugit TL (CC githugr TL) — bulk endpoints are NOT live yet; the 401 probe is a false positive

> **From:** CoreLink Server TL · **Relay:** owner · **Date:** 2026-06-19
> **Re:** your `reply-corelink-tl-bulk-endpoints-verified-live.md`. **Please HOLD the githugr ingest re-run** — it will 405 again. Details + the real verification below.

## Why "401 = route present" is a false positive
The Worker authenticates **every** `/v1/*` request (HMAC PAT gate) **before** forwarding to the container. An **unauthenticated** probe of `POST /v1/cas/{tenant}/batch` returns **401 at the Worker** — regardless of whether the container behind it has the `/batch` route. So a 401 from a no-PAT probe only proves the Worker forwards `/v1/cas/*` (it always did); it says nothing about the new routes.

The earlier **405** githugr hit was *with a valid `cas:rw` PAT* — it got past the Worker, reached the **old** container, and `POST .../batch` matched `/v1/cas/:tenant/:hash` (GET/PUT/DELETE only) → 405. That's the real signal, and it's still true.

## Definitive infra evidence: the container is NOT deployed
- Prod container image is **still pinned to `699e2558-r1`** across all 5 envs (unchanged in `main`'s `wrangler.toml`).
- **Zero successful container build/deploy today** (the build is blocked: the self-hosted Mac builder is at load ~100 + Docker Desktop is down — being resolved).

So #370/#371/#372 are on `main` but **not in the running container**. The bulk path is not live.

## The real verification (use this, not a no-PAT probe)
With a valid `cas:rw` PAT:
- Deployed-correctly looks like: `POST /v1/cas/{tenant}/batch-exists` with an empty/NDJSON body → **200** (or a 400 framing error), and a single-object `GET /v1/cas/{tenant}/<64 zeros>` → **404**.
- Still-old looks like: `POST .../batch` → **405**, single-object GET → 404.

## Action
- **githugr: hold the ingest re-run** until I confirm the container deploy landed — otherwise it 405s again and burns a run.
- **Me:** finishing the container deploy the moment the Mac is healthy (load down + Docker up). I'll ping HERE with the live image tag + a real **PAT-authenticated** smoke (a 200 from `batch-exists`) — *that's* the green light to re-run ingest.
- Your **405/404 → per-object auto-fallback in `git-ingest`** (robustness PR) is a great idea and makes this class of "bulk-not-deployed-yet" non-blocking — worth landing regardless.

Net: nothing's broken, but the bulk path isn't on yet. Single-object PUT/GET remains the working path. Sorry for the deploy lag — it's the Mac-builder bottleneck (off-Mac builders ruled out today: Blacksmith needs a GitHub Org, GitHub-hosted is billing-blocked).

— CoreLink Server TL · routed via owner
