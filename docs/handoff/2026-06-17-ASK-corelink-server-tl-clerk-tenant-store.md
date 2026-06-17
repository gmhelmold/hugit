# ASK → CoreLink Server TL — 3 P2 unblocks hugit needs from auth/identity/tenancy

> 2026-06-17 · from: hugit TL (via owner) · These are the auth/identity/tenancy seams
> that hugit has **built behind a fail-closed stub** and cannot flip to live without an
> artifact only CoreLink Server owns. Each item below is self-contained: what I need,
> the exact shape, why it blocks, what I do the instant I have it, and the acceptance
> check. No `path`/`git` coupling — deliver the values out-of-band; I wire + test.

---

## ASK 1 — Live Clerk JWKS + issuer + `azp` (unblocks `POST /v1/token`, the identity seam)

**What hugit has today:** `POST /v1/token` (RFC-8693 Clerk JWT → short-lived opaque
engine token) is fully built but its validator is `None`, so the route **404s by
design** (we don't expose it in dev-only mode). `alg` is pinned to RS256 on the
untrusted header before any key lookup; tokens are SHA-256-keyed, TTL-swept, never
logged. `Claims::org()` resolves `publicMetadata.tenant_id` → `org_id` → reject.

**What I need from you (exact shape):**
| field | env var hugit reads | example / format |
|-------|--------------------|------------------|
| JWKS URL | `HUGIT_CLERK_JWKS_URL` | `https://<clerk-domain>/.well-known/jwks.json` (RS256 keys) |
| Issuer | `HUGIT_CLERK_ISSUER` | `https://<clerk-domain>` (must equal the JWT `iss`) |
| Mandatory `azp` | (config) | the authorized-party value hugit must require on every token, OR an explicit "no azp enforcement in P2a" decision |
| Tenant claim path | (confirm) | confirm `publicMetadata.tenant_id` is the canonical org→tenant claim (hugit rejects a token lacking it) |

**Why it blocks:** without the live JWKS, hugit cannot verify a real Clerk session JWT,
so the engine stays on the dev-token stub — no real per-user identity, no per-tenant
authz from a real principal.

**What I do on receipt:** set the two env vars on the engine, drop the `azp` requirement
into the validator, redeploy. The route flips from 404 → live token exchange; the
two-tier auth gate (Clerk-minted token, else dev-token) starts minting real tokens.

**Acceptance:** a real owner-session Clerk JWT → `POST /v1/token` → `200` opaque engine
token; a token with wrong `iss`/`azp`/`alg` → `401`; a cross-tenant `org` → `401`.

---

## ASK 2 — P2 tenant provisioning (org = tenant) (unblocks real per-tenant data + live AC)

**What hugit has today:** the R2 source key is `<tenant_id>/<repo>.json`, with
`HUGIT_SERVE_R2_TENANT_ID` pinned to the single **dev tenant**
`00000000-0000-4000-8000-000000000001` (disclosed, not faked). The per-tenant authz
gate (`authorize_read`/`authorize_write`) already re-decides on every verb against the
repo's `owner_tenant`.

**What I need from you (exact shape):**
- The **real tenant id** for the HuGR org (the Clerk `org_id`/`tenant_id` UUID that
  replaces the dev `…0001`), and confirmation that the R2 key contract stays
  `<tenant_id>/<repo>.json` per CoreLink's convention.
- Confirmation of how a token's `org` claim maps to that tenant id (so the engine keys
  reads/writes to the right prefix per authenticated principal — today it's a single
  configured value, P2 makes it principal-derived).

**Why it blocks:** until the org→tenant mapping is real, every request resolves to the
one dev tenant; multi-tenant isolation is configured, not enforced from identity.

**What I do on receipt:** switch the tenant resolution from the single
`HUGIT_SERVE_R2_TENANT_ID` env to principal-derived (`org` claim → tenant prefix),
behind the same verified loader + authz gate. Add a conformance test for the mapping.

**Acceptance:** principal A (tenant α) reads/writes only `α/<repo>.json`; principal B
(tenant β) cannot read α's private repos (404, no existence leak) nor write them.

---

## ASK 3 — Decision: multi-instance shared token + idempotency store (unblocks horizontal scale)

**What hugit has today:** the engine-token store and the write-door's idempotency ledger
are **single-host / in-process**. The CAS write durability (R2 `If-Match`) is already
multi-writer-safe for the LOG; the gap is the in-memory token store + idem replay cache,
which a second engine instance wouldn't share.

**What I need from you:** this is a CoreLink primitive, not hugit's to fork — a decision
on the shared-store substrate (Durable Object / D1 / KV) and its access contract, OR an
explicit "single-instance is the P2 posture, defer multi-instance to P3."

**Why it blocks:** I will not build a bespoke shared store inside hugit (it would
duplicate a CoreLink primitive — against the "nothing built twice" stack rule). I need
the substrate decision to wire against.

**What I do on receipt:** wire the token store + idem ledger reads/writes to the chosen
substrate behind their existing traits (the seams are already trait-isolated). If you
say "single-instance P2," I mark it decided and stop — no work, no debt.

**Acceptance:** two engine instances share token validity + idempotency replay (a
lost-response retry to instance B replays instance A's outcome).

---

### Summary table

| # | Ask | Artifact you provide | hugit work on receipt | gated capability |
|---|-----|---------------------|----------------------|------------------|
| 1 | Live Clerk JWKS | JWKS URL + issuer + `azp` | set env + azp, redeploy | real identity / `/v1/token` |
| 2 | P2 tenant | real tenant id + org→tenant mapping | principal-derived tenant resolution | multi-tenant isolation |
| 3 | Shared-store decision | DO/D1/KV substrate OR "single-instance P2" | wire behind existing traits, or mark decided | horizontal scale |

Pick any one and hugit moves the same day. None of these need hugit code first — they
need the artifact, which is yours.

— hugit TL
