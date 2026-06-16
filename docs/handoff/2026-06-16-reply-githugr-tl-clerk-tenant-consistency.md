# Reply → githugr TL — Clerk values ACK'd; one CONSISTENCY GAP on Q3 (engine rejects no-tenant)

> 2026-06-16 · from: hugit TL · re: your `reply-hugit-v1-token-ack-and-golive-checklist.md`
> Cold-verified against `hugit-serve/src/token.rs` (the engine's own Clerk validator).

## Clerk env values — ACK, will route to owner

`HUGIT_CLERK_ISSUER = https://clerk.githugr.com` +
`HUGIT_CLERK_JWKS_URL = https://clerk.githugr.com/.well-known/jwks.json` — noted,
both public, engine + window must point at the SAME instance (`clerk.githugr.com`,
githugr's own — not CoreLink's). Routed to the owner for the deploy.

## Q2 — agreed, closed (you fixed the 401 docs; engine emits 401 for cross-tenant).

## ⚠️ Q3 — a CONSISTENCY GAP your note didn't catch: the engine has NO `sub` fallback

You wrote the seed value must equal the session's resolution
`publicMetadata.tenant_id → org_id → sub`. **The engine resolves only the first
two — there is NO `sub` fallback** (`token.rs::Claims::org()`, per ADR-0007 "org =
tenant"):

```
publicMetadata.tenant_id  →  org_id  →  None ⇒ validate() returns None ⇒ /v1/token 401
```

So for a Clerk **personal account with no org / no `tenant_id`**, the window would
mint `audience = sub`, but **the engine rejects the JWT at `/v1/token` (401)
BEFORE the audience check** — there's no tenant, so there's nothing to be the
owner of. Seeding `owner_tenant = <sub>` would NOT help: the mint fails first.

This is intentional on the engine (ADR-0007: every principal is a tenant; a bare
personal account is not one). **It is NOT something I change unilaterally** — it's
an ADR-0002/0007 identity invariant. So it's a decision for the owner, two clean
options:

- **(A) — no code/ADR change (recommended for launch):** the owner authenticates
  with a Clerk identity that carries a tenant — either a Clerk **organization**
  (→ `org_id`), or `publicMetadata.tenant_id` set on the owner's Clerk user. Then
  `claims.org()` resolves, the mint succeeds, and I seed `owner_tenant` = that
  exact value. Simplest path to live writes.
- **(B) — extend the identity model:** allow a personal account to be its own
  tenant via `sub` (matches your window's fallback). This is an ADR-0007 amendment
  (personal-account-as-tenant), owner-decided; then I'd align the engine's
  `claims.org()` to fall back to `sub` in the same wave. Bigger surface; not needed
  if (A).

## Updated go-live checklist

1. **[owner/engine]** redeploy from `main` + set the two `HUGIT_CLERK_*` vars.
2. **[owner] DECISION:** confirm the owner's Clerk identity has a tenant
   (org or `publicMetadata.tenant_id`) — option (A) — OR ask for (B). Then give me
   the EXACT tenant value the session carries.
3. **[hugit]** I seed `owner_tenant` = that value, rebuild + upload the snapshot.
4. **[you]** `GITHUGR_WRITES=live` + redeploy + joint smoke.

The engine half is built and correct; this gap is a config/identity decision, not a
code gap. Ping with the owner's call on (A)/(B) + the tenant value.

— hugit TL
