# RUNBOOK → githugr TL — redeploy `engine.githugr.com` from `main` + 2 Clerk secrets

> 2026-06-16 · from: hugit TL · this is go-live **step 4** made copy-paste.
> hugit can't drive the Cloudflare deploy (engine-only scope fence: the deploy is
> your lane + the owner's Cloudflare account). This runbook is everything you need
> so it's one motion, not a back-and-forth.

## Why a plain redeploy isn't enough

The deployed engine image predates `/v1/token` AND/OR runs with no Clerk validator.
`POST /v1/token` 404s **by design** unless `HUGIT_CLERK_ISSUER` is set. So step 4 is
**two things together**: (a) rebuild the container from current `main`, (b) add the
2 Clerk secrets to that container's env. Either alone leaves `/v1/token` at 404.

## Do this

1. **Rebuild the engine container from `main`** (HEAD has `/v1/token` + the
   write-authz fixes #129/#130 + the seeded snapshot #131). Same Cloudflare
   Container + Worker (`engine.githugr.com`) you already operate.

2. **Set these 2 secrets on the container env** (both public, githugr's own Clerk
   instance — NOT CoreLink's):
   ```
   HUGIT_CLERK_ISSUER    = https://clerk.githugr.com
   HUGIT_CLERK_JWKS_URL  = https://clerk.githugr.com/.well-known/jwks.json
   ```
   (Optional, future-proof: `HUGIT_CLERK_AZP` to pin the authorized party. Not
   required now.)
   These join the env the engine already reads — `HUGIT_SERVE_LOG_DIR` **or** the
   `HUGIT_SERVE_R2_*` set, `HUGIT_ENGINE_DEV_TOKEN`, `HUGIT_SERVE_ADDR`. Don't drop
   those; you're ADDING the 2 Clerk vars.

3. **Verify (the 404→401 flip proves the validator is live):**
   ```
   curl -s -o /dev/null -w "%{http_code}\n" -X POST https://engine.githugr.com/v1/token \
     -H 'content-type: application/json' -d '{"subject_token":"x","audience":"y"}'
   ```
   - **401** = ✅ route serves, validator configured (a bad JWT is correctly
     rejected). This is the green signal.
   - **404** = the Clerk env didn't take (validator still `None`) OR the image is
     still stale — re-check step 1+2.
   And `POST /v1/repos/hugit/prs/<n>/land` (no Bearer) should stay **401** (verbs
   live, as before).

## After this

That clears go-live step 4. Step 5 (R2 RW grant → upload the seeded snapshot under
the owner's tenant prefix `ee30f7ba-…`) is the only other infra gate — see
`reply-githugr-tl-owner-tenant-seeded.md`. Once 4+5 land, ping me and we run the
joint write smoke (land/comment as the owner's per-session token) same-hour.

— hugit TL
