# hugit → githugr TL (+ owner): the `/v1` engine backend is READY — deploy handover

**From:** hugit techlead · **Date:** 2026-06-13 ·
**Re:** the request `../githugr/docs/handoff/2026-06-13-hugit-http-server-request.md` ·
**Status:** ✅ **backend built, audited (twice), merged to hugit `main`** (PR #111). The
DEPLOY (hosting the backend + putting githugr.com live) is **your lane** (window + infra,
+ owner for the Cloudflare account) — this doc hands it over. The hugit (engine) session
stays out of the site-deploy per the engine-only scope fence.

## 1. What's done (hugit's lane)
The `/v1` HTTP server that serves the window's frozen contract from REAL engine state:
- **`hugit-http-contracts`** — the frozen wire VMs (byte-perfect twin of `githugr-vm`,
  audit-verified 34 types), round-trip parity-tested.
- **`hugit-serve`** — sync HTTP server (`tiny_http`, no TLS — the edge terminates) over
  the 5 Wave-1 reads (`home·landing·prs/{n}·checks·commits`) + `/readyz`. Real engine
  data where it exists, documented honest defaults elsewhere (never faked). Bearer
  auth-stub, redaction at the read boundary (5 secret-matrix guard tests), path-traversal
  guard, panic isolation, tampered-log→503 fail-honest, absent→404 no-leak. 68 tests +
  whole-workspace green; `cargo deny` clean.

## 2. How to run the backend (the binary)
`cargo run -p hugit-serve` (or the release binary) with three env vars:
| Env | Meaning |
|---|---|
| `HUGIT_SERVE_LOG_DIR` | dir holding one canonical event log per repo: `<dir>/<repo>.json` |
| `HUGIT_ENGINE_DEV_TOKEN` | the Bearer token the window presents (Wave-1 stub; required — fail-closed) |
| `HUGIT_SERVE_ADDR` | bind address (default `127.0.0.1:8787`) |
Then the window points `GITHUGR_ENGINE_URL` at it + sends `Authorization: Bearer <token>`.

## 3. Deploy on Cloudflare (your call; high-level only — NOT prescribed from here)
On Cloudflare, a Rust binary of this size runs on **Cloudflare Containers** (Workers are
JS/WASM-only — not viable for the engine). The container runs `hugit-serve`; a Worker
fronts it on `engine.githugr.com` (TLS/edge native). **Data source** is the one real
decision (pick per your timeline):
- **Baked log** (instant demo): a real hugit log snapshot in the image → live `live`
  showing real engine projections of a snapshot. Zero engine code change.
- **R2** (real, updatable, Cloudflare-native, no P2 needed): logs in an R2 bucket;
  `hugit-serve` reads `<repo>.json` from R2. **Needs a small engine change** (an R2 read
  source in `state.rs`) — request it from hugit and we'll add it; it's bounded.
- **CoreLink CAS** (the full real): the **P2 tenant** path (`docs/handoff/2026-06-08-corelink-p2-tenant-request.md`).

## 4. P2-gated (disclosed, not faked) — needs the CoreLink tenant
Real Clerk JWKS / RFC-8693 auth (replaces the dev-token stub), live fleet KPIs, live AC,
SSE stream + writes (Wave 2). Until then the read-path serves real-where-real + honest
defaults; the auth is the dev-token stub.

## 5. What we need back from you (the window/owner lane)
1. **Owner:** the Cloudflare account/access to deploy (the engine session won't drive it).
2. **githugr TL:** flip `GITHUGR_MODE=live` route-by-route once the backend answers; run
   the window deploy (your W-D wave) + the `engine.githugr.com` route.
3. If you want **R2** data (recommended over baked-log for a real site): tell hugit to add
   the R2 read source — small, bounded engine change, then it's Cloudflare-native real.

— routed via owner; no `path`/`git` dependency between repos. The backend is on hugit
`main`; everything else here is the window + infra lane.
