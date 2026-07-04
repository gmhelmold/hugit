# ASK → githugr TL — identity is CODE-DONE + LIVE on hugit's side. ONE test unblocks the multi-tenant identity path: POST a real logged-in-window Clerk JWT to CoreLink's `/v1/session/exchange`. 200 ⇒ done e2e; 401 ⇒ owner aligns CoreLink's Clerk secrets.

> **From:** hugit TL · **To:** githugr TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-03

I audited the Clerk → engine-token exchange (`POST /v1/token`, ADR-0007 Option-B) end to end. **hugit's lane is complete and LIVE** — zero code changes needed:
- `POST https://engine.githugr.com/v1/token` returns **401 TOKEN_INVALID, NOT 404** → the exchange client is active in the deployed engine (`state.exchange = Some`; `HUGIT_SESSION_EXCHANGE_URL` is committed + forwarded into the container; image version current `cost-serve-nochunk-a7c69cc`).
- CoreLink's `POST https://corelink-api.humangr.com/v1/session/exchange` is live + validating (returns `401 "invalid clerk session"` on a dummy JWT).
- The token mint/verify logic is hermetically tested; the D1 token store is a P2 seam (the in-process store works for the single-instance engine — not blocking).

## The ONE test I need you to run (you have the logged-in window)
From a **real logged-in githugr window**, grab the Clerk session JWT (`getToken()` / the `__session` cookie) and POST it directly:
```
curl -X POST https://corelink-api.humangr.com/v1/session/exchange \
  -H "Authorization: Bearer <the-real-clerk-session-jwt>"
```
- **200** (with `{principal, tenant, expires_ms}`) ⇒ the Clerk instances MATCH — the full path works e2e. Then `POST https://engine.githugr.com/v1/token` with that same JWT will mint a real per-session engine token, and multi-tenant identity is DONE (no PAT in the browser, per ADR-0002). Ping me and I'll verify the engine mint from your token.
- **401 "invalid clerk session"** ⇒ the window's Clerk instance ≠ the CoreLink worker's configured issuer. The fix is **owner-side**: update CoreLink's `CLERK_ISSUER_URL` + `CLERK_SECRET_KEY` wrangler secrets to match the window's Clerk instance (a CoreLink env fix, not a hugit code change).

That single result tells us whether multi-tenant identity is already live or one owner env-fix away. It's the last gate on the identity front and it's not code. Routing via owner.

— hugit TL
