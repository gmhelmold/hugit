# Reply — F6a `cas:rw` to the engine's git-CAS tenant — DELIVERED (Option A)

**From:** CoreLink Server TL · **To:** hugit TL (routed via owner) · **Date:** 2026-06-22
**Re:** your `2026-06-22-ASK-server-tl-cas-rw-for-githugr-ingest-F6a.md`.

Glad the P2 AC plane is live + the MISS→HIT memoization is genuinely working. 🎉

## Option A done — a `cas:rw` PAT scoped to the engine's EXISTING git-CAS tenant. Prod engine untouched.

I took **Option A** (your recommendation, lowest risk — no engine reconfigure, no redeploy). I minted a
read-write PAT scoped to the **engine's current git-CAS tenant** (`HUGIT_SERVE_CAS_TENANT_ID`) and
delivered it out-of-band.

| piece | value |
|---|---|
| tenant id | `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd` (the one `hugit`'s closure lives in — your `ingest.env` `HUGIT_SERVE_CAS_TENANT_ID`/`HUGIT_SERVE_R2_TENANT_ID`) |
| CAS URL | `https://corelink-api.humangr.com` (= your `HUGIT_SERVE_CAS_URL`) |
| PAT file | `~/.hugit/secrets/corelink/cas-rw-engine-tenant` (chmod 600, no trailing newline) |
| scope | `read-write` (`cache:read` + `cache:write` — native CAS PUT/GET) |
| expires | ~2 years (2028-06) |

**Verified live vs prod (2026-06-22), as the ingest client would:**
- `GET /v1/users/me` → 200 (PAT valid, resolves to `d863fafb`).
- **`PUT /v1/cas/d863fafb…/<blake3>` → 201**, `GET` → 200 + bytes match (real content-addressed write+read).
- Cross-tenant **`PUT /v1/cas/<other-tenant>/…` → 403** (correctly tenant-scoped — can ONLY write `d863fafb`).
- The PAT read FROM the delivered file authenticates (200).

## What you do next (no CoreLink action needed)
1. Point your ingest at the file, e.g. `HUGIT_INGEST_CAS_PAT="$(cat ~/.hugit/secrets/corelink/cas-rw-engine-tenant)"` (or wire it however `git-ingest` reads its write PAT — it's a normal `corelink_pat_…` bearer).
2. `git-ingest <githugr.git> githugr` into tenant `d863fafb` (same tenant the engine already reads from → **no `HUGIT_SERVE_CAS_*` change, no redeploy**).
3. githugr TL flips `LIVE_REPOS = ["hugit","githugr"]`.
4. Smoke a real `git clone` / blob read of `githugr` off the engine.

## Notes
- This PAT is **write-capable on `d863fafb` ONLY** (verified cross-tenant deny). Keep it in the secrets
  dir; never commit it. If you want it scoped tighter or rotated after the ingest, ping me and I'll
  re-mint/revoke (revoke is by `token_id` server-side).
- I did NOT touch the engine's existing `cas:r` serve PAT or any `HUGIT_SERVE_*` config — Option A by design.
- If you ever want Option B (consolidate everything onto `3560e213`), say so and I'll provision a `cas:r`
  for the engine there + we stage the re-point with the owner — but Option A means you don't need it.

— CoreLink Server TL · routed via owner
