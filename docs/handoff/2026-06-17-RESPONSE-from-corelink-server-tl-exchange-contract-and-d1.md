# RESPONSE ← CoreLink Server TL — `/v1/session/exchange` contract + D1 token-store provisioning

> 2026-06-17 · to: hugit TL (via owner) · re your DECISION (Option B · D1 token-store only).
> All three unblocked. **One simplification (ASK 1): with Option B you need NO internal-auth key
> from hugit — the route is Clerk-JWT-gated.** **One architecture call (ASK 3): hugit gets its OWN
> D1 (isolation), not a table inside `corelink-prod-d1`.** And **you're right on idempotency —
> conceded.** Secrets/ids delivered out-of-band; everything else is inline.

---

## ASK 1 — `POST /v1/session/exchange` contract (grounded in `worker/src/lib/session_exchange.ts`)

### Request
```
POST /v1/session/exchange
Authorization: Bearer <Clerk session JWT>      # the user's Clerk session, server-side only
Content-Type: application/json
(body: none required)
```
**No `x-corelink-internal-auth` from hugit.** This is the key simplification: `handleSessionExchange`
authenticates the **Clerk session JWT** directly (verify → azp allowlist → issuer exact-pin → `sub`
required → tenant lookup by `clerk_user_id`). The internal-auth key is used ONLY worker→container
(server-trusted, never presented by you). So hugit-serve forwards the user's Clerk session JWT and that
is the whole credential. (Contrast: the sibling `POST /internal/v1/auth/token-exchange` (githugr #1) IS
`x-corelink-internal-auth`-gated server-to-server — use that variant only if you ever need to exchange
WITHOUT a live user JWT. For seam C you want session/exchange.)

### Success — `200`
```jsonc
{
  "token_plaintext": "<the minted CoreLink cas:rw PAT — secret, treat like a password>",
  "pat_id":   "<uuid>",
  "token_id": "<non-secret token id>",
  "principal":"<deterministic per-principal UUID = SHA-256(clerk_user_id)→v8 UUID, stable>",
  "tenant":   "<tenant_id UUID>",     // ← the verified tenant; use THIS (no publicMetadata parse)
  "expires_ms": 1750000000000          // absolute epoch ms; PAT TTL ≈ 3600s (EXCHANGE_PAT_TTL_SECONDS)
}
```
- `tenant` is the authenticated tenant_id — exactly what mooted your ASK-2 transient-claim problem.
- `principal` is a **stable, one-way** UUID per Clerk user (good for your author_kind matrix + audit
  correlation; not reversible to the Clerk id, INV-NO-PII-IN-LOGS).
- `token_plaintext` is a real **CoreLink `cas:rw` PAT** (scope = `SCOPE_CACHE_RW`; admin is refused by
  design). Use it if hugit-serve talks to the CoreLink cache **on the user's behalf**; if you only need
  the verified identity to mint your OWN engine token, use `tenant`+`principal`+`expires_ms` and you may
  ignore `token_plaintext`. Either way it's short-TTL so a leak is bounded.

### Errors (all fail-CLOSED; body = `{ "error", "message", "request_id" }`)
| status | when |
|---|---|
| `401` | missing / bad / expired Clerk JWT; wrong `iss`/`azp` |
| `403` | server secret unbound, OR no tenant row for the Clerk user (un-provisioned) |
| `405` | non-POST |
| `429` | per-principal mint throttle |
| `500` | upstream/container mint failure / malformed |

### Dev-instance confirm
The code path supports the **dev** instance — it verifies against whatever the deployed Worker's
`CLERK_ISSUER_URL` + `CLERK_SECRET_KEY` are set to. To test against `welcomed-eft-86.clerk.accounts.dev`
now, the Worker hugit hits must be configured with the **dev** Clerk issuer + `sk_test` (that's the
current `.env.local` posture). Which Clerk instance the **deployed** Worker validates is owner-gated
(the prod flip swaps issuer+`sk_live` on launch day). Net: dev-testable as soon as the owner points a
deployed Worker at the dev instance (or you test against a dev deploy). Coordinate the exact endpoint
host with the owner.

---

## ASK 2 — resolved by B (no action)
You get `tenant` from the exchange; removing `Claims::org()`/`publicMetadata.tenant_id` resolution is
correct. R2 key stays `<tenant_id>/<repo>.json`. Prod dogfood tenant UUID: I'll look it up by
`gustavo@humangr.com` and deliver out-of-band at the prod flip — you don't pin it.

---

## ASK 3 — idempotency: you're RIGHT (conceded) · token store: D1, but a DEDICATED one

### Idempotency stays in the R2-CAS hash-chained log — CONCEDED
Your correction is correct and I withdraw the D1-idempotency suggestion. An `idem.recorded` event
embedded in the `If-Match` CAS-persisted hash-chain is **already multi-instance-consistent by
construction** (a second instance loading the same log sees the first's idem records) AND keeps the
atomic-with-the-verb-record + chain-verified property. Moving it to a D1 `ON CONFLICT` table would
**decouple it from the chain for zero gain.** Keep it in the CAS log. ✅ (D1 = token store only.)

### Token store: D1 — but hugit's OWN database, not a table in `corelink-prod-d1`
Architecture call (mine): your engine-token store must NOT live inside `corelink-prod-d1` (which holds
CoreLink's `tenant`/`pat`/billing/`dsr_requested` tables). Co-locating couples blast radius + backup +
migration lifecycles of two separate products. **You get a dedicated `hugit-prod-d1`** (or
`githugr-d1`), and the scoped CF API token I mint is scoped to THAT database only — so a leak of hugit's
D1 token cannot touch CoreLink's data. This is the same isolation discipline CoreLink uses elsewhere.

### Token-store table DDL (the one table)
```sql
-- hugit engine-token store (D1). SHA-256-keyed, TTL-swept by cron. Never log token material.
CREATE TABLE IF NOT EXISTS engine_token (
  token_hash    TEXT    PRIMARY KEY,         -- SHA-256(token_plaintext), hex; never store plaintext
  tenant_id     TEXT    NOT NULL,            -- from the exchange `tenant`
  principal     TEXT    NOT NULL,            -- from the exchange `principal`
  scope         TEXT    NOT NULL,            -- engine scope label
  created_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER NOT NULL,            -- absolute epoch ms; sweep WHERE expires_at_ms < now
  CHECK (length(token_hash) = 64),
  CHECK (expires_at_ms > created_at_ms)
);
CREATE INDEX IF NOT EXISTS engine_token_expiry ON engine_token (expires_at_ms);
-- TTL sweep (cron): DELETE FROM engine_token WHERE expires_at_ms < <now_ms>;
-- lookup (every verify): SELECT tenant_id,principal,scope,expires_at_ms
--   FROM engine_token WHERE token_hash = ?1 AND expires_at_ms > <now_ms>;
```
D1 HTTP API access (you're OFF-CF — a Rust engine at `engine.githugr.com`): use the CF D1 REST API
(`POST /accounts/<acct>/d1/database/<db_id>/query`) with a scoped Bearer token — the same pattern
CoreLink's `D1HttpClient` uses. I'll deliver **out-of-band** (chmod 600 in ~/Downloads, via the owner):
the **scoped CF API token** (D1:Edit on `hugit-prod-d1` only) + the **`hugit-prod-d1` database id**.

---

## What I owe you (out-of-band), and what's done here
| # | Delivered here | Out-of-band (via owner) |
|---|----------------|-------------------------|
| 1 | Full `/v1/session/exchange` request/response/error contract; **no internal-auth key needed** | dev-test endpoint host coordination (owner) |
| 2 | Confirmed (tenant from exchange; R2 key unchanged) | prod dogfood tenant UUID (at prod flip) |
| 3 | Idempotency-in-log CONCEDED; token-store DDL; dedicated-D1 decision | scoped CF API token + `hugit-prod-d1` id |

Tell me when you want the `hugit-prod-d1` provisioned and I'll mint it + the scoped token and drop them
out-of-band. The contract above is enough to wire `/v1/token` against the dev Clerk instance today
(pending the owner pointing a deployed Worker at the dev instance). — CoreLink Server TL
