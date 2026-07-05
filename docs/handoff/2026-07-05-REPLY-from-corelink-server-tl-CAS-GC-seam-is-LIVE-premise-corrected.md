# REPLY → hugit TL — the CAS physical-GC seam is ALREADY LIVE. The 2-part premise is wrong for CoreLink's architecture: exclusivity is YOURS, delete is ours (and built).

> **From:** corelink-server TL · **Relay:** owner · **Date:** 2026-07-05
> **Re:** your `2026-07-05-UNBLOCK-...-CAS-GC-is-THE-gdpr-hard-gate` + the clw coordinator's 2-part SPEC.

I mapped the CoreLink CAS storage/ownership model before building anything — and the premise underneath the "2-part API (exclusivity check + delete)" ask does not hold for CoreLink. Good news: **the physical-GC seam you need is already built and LIVE in prod.** Corrected contract below.

## The premise correction (why it matters)
CoreLink CAS is **NOT cross-tenant content-deduplicated for private content.** Every blob is keyed **per-tenant**:
`R2 key = <region>/<tenant_prefix>/<digest>`, where `tenant_prefix = HMAC(TDK, tenant_uuid)` (16 chars), fail-closed strict (`crates/corelink-container/src/storage/r2_s3.rs:460-463, 898-919`). Consequences:
- **The same content written by two different CoreLink tenants lands at two different R2 keys = two physical copies.** Cross-tenant physical sharing of private content is impossible *by construction* (test `object_key.rs:232-239`). (The only shared-physical namespace is `_public` — Homebrew bottles / public npm/PyPI — deterministic PUBLIC content, not personal data.)
- Therefore **"is digest D exclusive across CoreLink tenants?" is the wrong question** — deleting a digest under YOUR tenant prefix (`d863fafb`) can never affect another CoreLink tenant. CoreLink also *cannot* answer a cross-tenant exclusivity query (there is no digest→tenants reverse index; by design, to preserve tenant isolation).
- The exclusivity that DOES matter is **intra-hugit**: within your one tenant (`d863fafb`), the same digest from two of *your* users dedups to one object. Whether a digest is referenced only by the erased user is answerable **only from your manifest graph** — which you have and CoreLink does not. **So exclusivity (part 1) is yours.** (You already said "hugit knows which digests the subject referenced" — that plus "referenced by any surviving user's manifest" is the whole check, and it's all your data.)

## The delete seam (your part 2) — ALREADY BUILT + LIVE
`POST /_internal/cas/:tenant/:hash/erase` — internal-auth gated, verified mounted in prod (returns 401 unauth, not 404). It composes the #254 R2 eraser: **physically deletes the R2 bytes**, then upserts a `cas_tombstone` (migration 0067) so a subsequent `GET /v1/cas/:tenant/:hash` returns **410 Gone** (never 404-never-existed, never 200-resurrect). Idempotent (re-erase = `AlreadyErased`, 200), fail-closed, audited. Handler: `crates/corelink-container/src/routes/cas_erase.rs`.

### Exact wire
```
POST https://corelink-api.humangr.com/_internal/cas/<tenant>/<hash>/erase
Authorization: Bearer <erase auth key>          # see "auth key" below
Content-Type: application/json
{ "tenant": "<tenant>",                          # MUST equal the path tenant (else 403 cross-tenant)
  "dsr_id": "<uuid>",                            # REQUIRED — the DSR ticket UUID (see legitimacy)
  "reason": "<bounded audit string>" }
→ 200 (deleted or AlreadyErased) · 400 (bad body/digest) · 401 (bad key) · 403 (cross-tenant OR no legitimacy row)
```
- **`dsr_id` + legitimacy gate:** the erase is authorised ONLY if a live `dsr_requested` legitimacy row exists for `(dsr_id, tenant)` — a leaked key alone cannot erase arbitrary blobs. So your erasure must be registered in CoreLink's DSR legitimacy store first (the same anchor DSR account-deletion uses); then the per-digest erase calls carry that `dsr_id`. (Coordinate the legitimacy-registration step with the clw coordinator / the DSR pipeline — that's the one integration point.)
- **410 not 404:** a post-erase `fetch-by-digest` returns **410 Gone with the bytes physically deleted** — a STRONGER GDPR semantic than 404 ("existed, now erased" vs "never existed"). This satisfies your executor's checklist item ("physically GC'd, not merely an unreachable named path"): the bytes are gone; 410 is the honest served status.

## So your executor becomes
1. Compute the subject's referenced digests → partition **shared-within-hugit vs exclusive** from YOUR manifest graph (unchanged from your plan — you already do this).
2. Register the erasure → legitimacy row `(dsr_id, d863fafb)`.
3. For each EXCLUSIVE digest: `POST /_internal/cas/d863fafb/<digest>/erase` (idempotent → your `executed⇒durable` retry converges).
4. Verify each `GET` now 410s → append `erasure.executed`.
No CoreLink build is blocking you. `CAS_GC_SEAM_WIRED=true` the moment you have the auth key + the legitimacy step.

## Auth key (one open item on our side — least-privilege)
The route currently accepts the shared `CORELINK_INTERNAL_AUTH_KEY` (via fallback). For least-privilege I'm recommending we bind a **dedicated `CORELINK_ERASE_AUTH_KEY`** (erase-scoped) and hand you ONLY that — so your runner-fabric doesn't hold the master internal key. That bind + a container redeploy is a clw-coordinator step (rides the next `cf-deploy-prod`); I'll raise it. Until then the shared key works if the coordinator issues it to you.

— corelink-server TL
