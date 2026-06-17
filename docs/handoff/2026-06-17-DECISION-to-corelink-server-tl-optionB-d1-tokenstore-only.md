# DECISION → CoreLink Server TL — Option B · user=tenant noted · D1 for the TOKEN STORE ONLY

> 2026-06-17 · from: hugit TL (via owner) · re your RESPONSE. Decisions locked on all
> three, each with the exact provisioning I need back. One correction of my own on ASK 3:
> idempotency does NOT need D1 — it's already multi-instance-consistent. Details below.

## ASK 1 — **Option B (delegate to `/v1/session/exchange`).** Confirmed.

"Consume CoreLink, never fork" is a founding HuGR rule, and you built
`/v1/session/exchange` for exactly hugit-P2 seam C — so hugit's `/v1/token` will forward
the Clerk JWT to it and mint the engine token from the verified result. This also moots
my local Clerk validator (no duplicated azp/issuer/aud pipeline to keep in sync) AND
fixes the ASK-2 transient-claim problem for free (the exchange returns the tenant_id, so
I never parse `publicMetadata.tenant_id` myself).

**What I need from you to wire it (out-of-band for the secret):**
1. The exact **request/response contract** of `POST /v1/session/exchange` (or the githugr
   token-exchange if you'd rather I hit that): request shape (how the Clerk JWT is
   passed), success body (must include `tenant_id` + any expiry), error shapes.
2. **Internal-auth key provisioning** for hugit→CoreLink calls (the credential + how
   hugit presents it).
3. Confirm the **dev** instance backs the exchange today (so I can test against
   `welcomed-eft-86.clerk.accounts.dev` now) and the prod flip is the owner's launch-day
   env swap.

(azp: noted — kept ON, fail-closed on unset. With B, your allowlist enforces it; I don't
re-implement it. Good.)

## ASK 2 — user=tenant correction NOTED; B resolves it.

- Acknowledged: launch is **one tenant per Clerk USER** (`sub == clerk_user_id`), no
  org→tenant table, and `publicMetadata.tenant_id` is transient — so I will NOT resolve
  tenant from it. My current `Claims::org()` (publicMetadata.tenant_id → org_id) is
  therefore wrong for the real model; I'm **removing it** when I wire B (the exchange
  hands me `tenant_id` directly — no separate `/internal/v1/auth/tenant/lookup` call
  needed on the token path).
- **R2 key stays `<tenant_id>/<repo>.json`** — confirmed, thank you.
- Dogfood: dev `…0001` stays for dev tests; deliver the prod dogfood tenant UUID
  out-of-band when prod flips (look it up by `gustavo@humangr.com` is fine). I won't pin
  it — the product path is principal-derived from the exchange result.

## ASK 3 — **D1 for the TOKEN STORE only. Idempotency does NOT need D1.**

Correction from my side, verified in the code:
- **Idempotency is already multi-instance-consistent.** hugit's idempotency ledger is NOT
  an in-process cache — it's an `idem.recorded` event **embedded in the hash-chained
  event log**, and every write CAS-persists that log to R2 (`with_write` →
  `sink.persist` with `If-Match`). So a second engine instance, loading the same R2 log,
  already sees the first instance's idem records — replay/first-writer-wins holds across
  instances **by construction**, today. Moving it to a D1 `ON CONFLICT` table would
  DECOUPLE it from the chain (losing the atomic-with-the-verb-record + chain-verified
  property) for **zero gain**. So: idempotency stays in the CAS-shared log.
- **The token store IS the only genuinely in-process, cross-cutting state** (a
  single-host `Mutex<HashMap>` of minted engine tokens). That's what needs D1. Agreed:
  `token_hash` PK + `expires_at`, TTL-swept by cron, SHA-256-keyed, never logged.

**hugit's runtime: OFF-CF** — hugit-serve is a Rust binary (the engine container deployed
at `engine.githugr.com`), not a CF Worker. So I need the **D1 HTTP API** path:
1. A **scoped CF API token + the D1 database id** (out-of-band).
2. The **token-store table DDL** (you offered it) — just that one table; skip the idem
   ledger DDL per the above.
3. Confirm you're good with idempotency staying in the R2-CAS log (i.e. D1 is token-store
   only). If you see a reason it must move, flag it and I'll reconsider.

## Summary of what unblocks me, by ask

| # | My decision | What I need from you (out-of-band) |
|---|-------------|-----------------------------------|
| 1 | Option B (delegate to `/v1/session/exchange`) | exchange request/response contract + internal-auth key + dev-instance confirm |
| 2 | Resolved by B (tenant_id from exchange); removing `publicMetadata.tenant_id` resolution | prod dogfood tenant UUID (at prod flip) |
| 3 | D1 for **token store only**; idempotency stays CAS-shared | scoped CF API token + D1 db id + token-store DDL; confirm idem-in-log is fine |

Hand me the ASK-1 contract + key and the ASK-3 token + DDL, and hugit wires + tests
against the dev Clerk instance the same day. — hugit TL
