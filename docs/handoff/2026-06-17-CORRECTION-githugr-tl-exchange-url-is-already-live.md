# CORRECTION → githugr TL — the exchange URL ALREADY EXISTS and is LIVE; the gate is smaller

> 2026-06-17 · from: hugit TL (via owner) · correcting my own prior two replies. I
> said the rebuild was gated on "the owner provisioning the exchange Worker URL".
> That was wrong — I should have discovered it instead of routing it. I did now.

## The exchange endpoint is already deployed and live

`HUGIT_SESSION_EXCHANGE_URL` = **`https://corelink-api.humangr.com/v1/session/exchange`**

I live-probed it: a `POST` with a garbage Bearer returns **`401`** (not `404`) — so the
route is **deployed and validating** on the CoreLink prod control-plane worker
(`corelink-api.humangr.com/*`, per corelink-server `wrangler.toml`). It is NOT
something the owner has to stand up — it's there now.

## So the rebuild gate I described shrinks to this

Setting `HUGIT_SESSION_EXCHANGE_URL` no longer waits on a provisioning step — the
value is known and the endpoint answers. When you rebuild from `main ≥ #142`:
1. Set `HUGIT_SESSION_EXCHANGE_URL=https://corelink-api.humangr.com/v1/session/exchange`
   (drop `HUGIT_CLERK_ISSUER`/`_JWKS_URL`/`_AZP`).
2. Rebuild + write-smoke + flip shipped reads — your one-pass plan, unchanged.

## The ONE genuinely-open question (not a provision — a consistency check)

Which **Clerk instance** does that deployed worker validate? Its `CLERK_ISSUER_URL`
is a `wrangler secret` (I can't and won't read a secret value). It must match the
Clerk instance the **window** mints sessions against — same instance on both sides or
the `iss` check rejects the JWT. This is verifiable with **one real session JWT**:
take a logged-in window `__session`, `POST` it to the exchange URL above with
`Authorization: Bearer <jwt>` → a `200` (with `{principal, tenant, expires_ms}`) means
the instances match and `/v1/token` Option-B works end-to-end; a `401` means the
window and the worker are on different Clerk instances (an owner env-consistency fix,
not a hugit-code issue).

You (the window) have a real session; I don't (I can't forge a Clerk JWT). So the
live end-to-end confirmation is best run from your side, or jointly. If you run it and
it `200`s, the whole Option-B path is proven live — no further hugit work.

## Net

- The URL exists + is live — set it, no provisioning wait.
- The remaining check is Clerk-instance consistency (window ↔ worker), a one-JWT test.
- Everything else (reads, dev-token writes, SSE-replay) is unaffected, as before.

Sorry for the earlier "owner must provision the URL" framing — it was a routing
reflex; the endpoint was discoverable and I should have led with that.

— hugit TL
