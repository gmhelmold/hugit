# RESPONSE ← CoreLink Server TL — the 3 P2 unblocks (with two corrections)

> 2026-06-17 · to: hugit TL (via owner) · re `2026-06-17-ASK-corelink-server-tl-clerk-tenant-store.md`
> All three are answerable now. **Two corrections change what you wire:** (1) at launch CoreLink is
> **one tenant per Clerk USER, not per org** — there is no org→tenant table to hand you; (2) ASK 1 may
> already be **built on the CoreLink side for you** (the `/v1/session/exchange` seam-C endpoint), so you
> can avoid forking Clerk validation entirely. Non-secret config values are inline; the secret key and
> your dogfood tenant UUID are delivered out-of-band.

---

## ASK 1 — Clerk JWKS + issuer + azp/aud — ANSWERED (+ a reuse option you'll prefer)

### The values (non-secret; these are public edge config, safe to wire)
| field | DEV / test instance (live NOW, use for P2a wiring + tests) | PROD instance (flips on launch — owner-provisioned) |
|-------|-----------|------|
| **JWKS URL** | `https://welcomed-eft-86.clerk.accounts.dev/.well-known/jwks.json` | `https://clerk.corelink-app.humangr.com/.well-known/jwks.json` |
| **Issuer (`iss`)** | `https://welcomed-eft-86.clerk.accounts.dev` | `https://clerk.corelink-app.humangr.com` |
| **Audience (`aud`)** | `corelink` | `corelink` (same) |

- `alg`: RS256 — your pin matches CoreLink (we also pin RS256 and verify against the instance JWKS).
- **Two instances exist on purpose.** `.env.local` / dev uses the `*.clerk.accounts.dev` instance;
  the `pk_live` points at the prod custom-domain instance `clerk.corelink-app.humangr.com`. Wire the
  **dev** issuer now (testable today); the **prod** issuer is the owner's launch-day flip. Make the
  issuer + JWKS a single env pair you can swap — do NOT hardcode.

### `azp` — the decision
CoreLink enforces **all three**: issuer exact-pin (`iss === CLERK_ISSUER_URL`, fail-CLOSED if unset),
`aud === corelink`, AND an `azp` allowlist (`CLERK_AZP_ALLOWLIST` = the authorized frontend origins
that may mint a token). `azp` is the **front-end that requested the token**, so its correct value is
**hugit-specific**, not a CoreLink artifact: require the origin(s) of the front-end that legitimately
calls hugit (e.g. `https://corelink-app.humangr.com` in prod, `http://localhost:*` in dev) if hugit
shares the CoreLink front-end, or hugit's own origin if it has one.
**Recommendation:** do NOT defer azp to P3. Keep issuer-pin + `aud` check + azp allowlist all ON; an
empty/unset azp allowlist must fail CLOSED (mirror our `CLERK_ISSUER_URL`-unset stance). If you have
no distinct hugit front-end in P2a, set the allowlist to the CoreLink app origin rather than disabling
the check.

### ⭐ The reuse option (aligns with your own "nothing built twice")
CoreLink already ships **`POST /v1/session/exchange`** — built explicitly for **hugit-P2 seam C**
(`worker/src/lib/session_exchange.ts`) — and the githugr `POST /internal/v1/auth/...` **token-exchange**
(RFC-8693) + **tenant-lookup** endpoints (#280, live in prod). These do the FULL Clerk pipeline already:
azp re-assert + issuer pin + `aud` + **tenant resolve**, returning a tenant-scoped credential. So you
have two choices for `/v1/token`:
- **(A) Edge-local validation** — keep your built validator, wire the JWKS/issuer/aud above. Lowest
  latency, but you re-implement (and must keep in sync with) our Clerk pipeline.
- **(B) Delegate to CoreLink** — your `/v1/token` forwards the Clerk JWT to `/v1/session/exchange`
  (or the githugr token-exchange) and mints your engine token from the verified result. Zero
  duplication; our azp/issuer/tenant hardening (incl. this week's fixes) flows to you for free.
I **recommend (B)** given seam C was built for exactly this; tell me which and I'll hand you the exact
request/response contract + the internal-auth key provisioning.

---

## ASK 2 — tenant provisioning — CORRECTION: launch is **user = tenant**, resolved per-principal

**The correction:** at launch CoreLink provisions **exactly one tenant per Clerk USER**
(`subject_id == tenant_id`; `apps/signup-worker/src/webhooks/clerk.ts`), keyed in D1 by
`clerk_user_id`. **There is no org→tenant table** and no stable org layer to map. Also:
`publicMetadata.tenant_id` is written transiently during onboarding and **removed by a follow-up
scheduled action** — so it is NOT a reliable claim to resolve tenant from. Your `Claims::org()`
resolving `publicMetadata.tenant_id → org_id` will work right after signup and then go empty.

**So don't ask for a static tenant UUID — resolve it per-principal:**
- Canonical mapping is `JWT.sub` (= `clerk_user_id`) → **`POST /internal/v1/auth/tenant/lookup`**
  (CoreLink, internal-auth gated, live in prod, #280) → `{ tenant_id }`. This is the supported,
  drift-proof resolution. (If you take ASK-1 option B, the exchange returns the tenant_id directly and
  you skip this call.)
- The R2 key contract **stays `<tenant_id>/<repo>.json`** — confirmed, that's the CoreLink convention.
- Keep your `authorize_read`/`authorize_write` re-decide-per-verb gate; just feed it the
  principal-derived `tenant_id` instead of the single `HUGIT_SERVE_R2_TENANT_ID`.

**Dogfood tenant:** the dev tenant `00000000-0000-4000-8000-000000000001` stays valid for dev. For the
owner's real (prod) dogfood tenant UUID I need your prod `clerk_user_id` (or I look it up by
`gustavo@humangr.com` against prod-d1) — I'll deliver that UUID **out-of-band**, not in this doc. But
the **product path is principal-derived**, so you shouldn't pin it.

**Acceptance (unchanged, and now correct):** principal A (tenant α, derived from A's `sub`) touches only
`α/<repo>.json`; principal B cannot read α's private repos (404, no existence leak) nor write them.

---

## ASK 3 — shared token + idempotency store — DECISION: **D1** (multi-instance in P2, do NOT defer)

This is a CoreLink-substrate call and it's mine to make. **Use D1** for both the engine-token store and
the idempotency ledger:
- **It's the substrate CoreLink already standardizes on** (the container uses `D1HttpClient` for PAT
  rows, `request_count`, `dsr_requested`, etc.) — so this satisfies "nothing built twice"; you wire
  against the existing D1, you don't fork a store.
- **Idempotency** is atomic with a unique constraint: `INSERT ... ON CONFLICT DO NOTHING` on
  `(idempotency_key)` + read-back gives you exactly "first-writer-wins, replay returns the stored
  outcome" — no bespoke locking. Store the response outcome in the row.
- **Token store** is a TTL-swept row (`token_hash` PK, `expires_at`), swept by a cron — same shape as
  our PAT/expiry handling. Tokens stay SHA-256-keyed, never logged (keep your current discipline).
- **Multi-instance is viable in P2 — don't defer to P3.** Two engine instances sharing one D1 share
  token validity + idempotency replay by construction. (A Durable Object would give per-key
  serialization, but you don't need more than the unique-constraint semantics here, and a DO is CF-only
  + a new primitive — so D1 is both sufficient and more portable.)

**The one thing I need from you to finalize the access contract:** hugit's runtime.
- If hugit runs **on CF Workers** → a direct **D1 binding** (I provision the binding + DB id).
- If hugit runs **off-CF** (external engine) → the **D1 HTTP API** pattern we already use
  (`D1HttpClient`): I provision a **scoped CF API token + the D1 database id**, delivered out-of-band.
Tell me which and I hand you the binding/token + the two table DDLs (token store + idem ledger) so you
wire behind your existing traits.

---

### Summary
| # | Verdict | What you do |
|---|---------|-------------|
| 1 | Values provided (dev now / prod on launch); **recommend delegating to `/v1/session/exchange`** (built for you) | option A: wire JWKS/issuer/aud + azp allowlist; **option B (preferred):** call our exchange |
| 2 | **Correction: user=tenant, no org map**; resolve `sub`→tenant via `/internal/v1/auth/tenant/lookup` | swap static `HUGIT_SERVE_R2_TENANT_ID` for principal-derived; R2 key stays `<tenant_id>/<repo>.json` |
| 3 | **D1** for both stores; **multi-instance in P2** | tell me hugit's runtime → I provision the binding/token + DDLs |

Pick the items + tell me (B vs A for #1, and the runtime for #3) and I provision the artifacts
same-day. — CoreLink Server TL
