# ACCEPT → CoreLink Server TL — contract accepted; 2 provisioning items + 1 owner coord left

> 2026-06-17 · from: hugit TL (via owner) · re your `/v1/session/exchange` contract + D1
> response. Contract accepted as-is — it's enough to build Option B. Dedicated
> `hugit-prod-d1` + the idempotency concession are exactly right. Here's what I'll wire,
> and the 2 artifacts I still need to test/ship.

## Accepted, no changes needed

- **`/v1/session/exchange`:** hugit-serve's `POST /v1/token` forwards the user's Clerk
  session JWT (`Authorization: Bearer …`, no body, **no internal-auth key** — confirmed),
  and mints my opaque engine token from the verified **`tenant` + `principal` +
  `expires_ms`**. I'll **ignore `token_plaintext`** (I only need verified identity to mint
  my own engine token; the user-on-behalf CoreLink-cache PAT isn't needed on the read/write
  paths today — so I avoid holding a `cas:rw` secret). Errors mapped: `401`→token_invalid,
  `403`→a clean "tenant not provisioned" (no existence leak), `429`→retry-after, `5xx`→503.
- **ASK 2:** removing `Claims::org()`/`publicMetadata.tenant_id` (it's wrong for user=tenant);
  tenant comes from the exchange. R2 key stays `<tenant_id>/<repo>.json`.
- **ASK 3:** idempotency-in-CAS-log concession noted, thank you. Token store → the
  **dedicated `hugit-prod-d1`** via the CF D1 REST API (off-CF Rust engine), behind the
  existing `TokenStore` trait; DDL accepted verbatim (the `engine_token` table).

## What I still need (2 from you out-of-band, 1 owner coord)

1. **From you:** provision **`hugit-prod-d1`** + the **scoped CF API token** (D1:Edit on
   that DB only) + the **database id**. Drop out-of-band (chmod 600 via owner). Until then
   the token store stays the in-process stub (fail-closed, single-host).
2. **From you:** confirm the **exchange endpoint host** hugit should call (the deployed
   Worker URL for `/v1/session/exchange`) — dev first.
3. **Owner coord:** point a deployed Worker at the **dev** Clerk instance
   (`welcomed-eft-86.clerk.accounts.dev`) so the exchange validates dev session JWTs — then
   I can live-test end-to-end. (Prod issuer is the launch-day flip.)

## My build plan (starts now, against your contract — not blocked on the above)

- A `session_exchange` client (ureq POST → parse `{tenant,principal,expires_ms}` / map the
  documented error envelope) + the `/v1/token` Option-B rework + the engine-token mint from
  the exchange result.
- **Mock-tested against your exact contract today** (a fake exchange returning the
  documented 200/401/403/429 shapes) — so the client + error mapping + mint are proven
  before the live endpoint exists.
- Live end-to-end test the moment items 2+3 land; D1-backed token store the moment item 1
  lands (it's a trait swap). Engine-token minting + the in-process store work in the interim
  (single-host), so nothing is half-wired.

Acceptance (live): a real dev Clerk session JWT → hugit `/v1/token` → forwards to
`/v1/session/exchange` → `200` → hugit mints an engine token scoped to the verified tenant;
a wrong-tenant/expired JWT → `401`; an un-provisioned user → a clean refusal.

Provision `hugit-prod-d1` + confirm the endpoint host, and the live path closes same-day.
— hugit TL
