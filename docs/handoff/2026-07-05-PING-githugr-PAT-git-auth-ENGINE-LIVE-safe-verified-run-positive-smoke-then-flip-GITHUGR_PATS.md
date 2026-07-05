# 🟢 PING → githugr TL: PAT git-auth is ENABLED + LIVE on the prod engine (safe-verified). Run your positive create→push smoke with a tenant token → then flip `GITHUGR_PATS=1`.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw

## ✅ Engine-side go-live DONE (two-stage deploy, owner-GO'd)
The prod engine (`engine.githugr.com`) is redeployed to current main (with #261) and **`HUGIT_SERVE_PAT_AUTH=1`
is LIVE**. Version `2026-07-05-pat-wiring-s2-645ba69`.
- **Stage 1** (main, flag OFF): cutover clean — `/readyz {ready:true, git_serving:true, git_repos:2, cas_batch_read:"ok 1024/1024"}`; reads unchanged.
- **Stage 2** (flag ON): cutover clean — the **detached `_accounts/*` boot-scan did NOT block/crash the boot** (`cas_batch_read:ok`, `git_repos:2`). PAT auth is on.

## ✅ Safety-verified from here (the reject + neutrality paths)
- Invalid PAT via **`Bearer`** → **401** (no crash). Invalid PAT via the **git-CLI `Basic` password** (`-u x:<pat>`) → **401** (the base64 decode + resolver handle a bogus token cleanly).
- **Behavior-neutral for non-PAT traffic:** anon `/v1/me/account` → 401 and anon `hugit` reads → 404, **identical to before** (a non-`ghgr_pat_` bearer short-circuits to `None` → dev-token/anon path unchanged). Your dev-token render-verify is unaffected.
- Engine **healthy after the adversarial probes** (`/readyz` 200, `git_repos:2`).

## 🟡 The ONE remaining check before you flip: the POSITIVE loop — your smoke
I verified the engine boots safe + rejects bad tokens, but the **positive** loop (a REAL PAT authenticates a
`git push`/read as-tenant) needs a real tenant/session credential to (a) create the PAT (`POST /v1/me/tokens` needs a
session — a PAT can't mint) and (b) push as-tenant. **That credential is yours** (the `mint-tenant-bearer` tool +
your Clerk SK; the tenant-bearer I have here 401s — it's the SK, not an engine token). Per our split (the authed
smoke is the githugr TL's), **please run the positive smoke:**
1. Mint a tenant session token (your tool).
2. `POST /v1/me/tokens` (scopes `["repo:write"]`) → get the `ghgr_pat_…` secret once.
3. `git clone`/`push` to `engine.githugr.com/<repo>` with that PAT as the HTTP Basic password → expect it to
   authenticate as your `clerk:{org}:{user}` (NOT operator).
4. A `repo:read`-only PAT on a push → **403 `SCOPE_INSUFFICIENT`**. Revoke → the same PAT then **401**.
- If you'd rather I run it, send me a minted tenant bearer (or point me at the SK path + I run `mint-tenant-bearer`)
  and I'll do the positive loop + confirm.

**On a green positive smoke → flip `GITHUGR_PATS=1`.** Everything else (your UI: 90d-cap, ms, eye-gate) is ready.

## ⚠️ One reproducibility to-do on YOUR repo (the config-drift lesson)
The deploy set two config changes in `../githugr` that are **live on prod but currently UNCOMMITTED** on your
`pr-b-gdpr1-account-erase` branch — please commit them to your canonical engine config so a future redeploy doesn't
silently regress PAT auth to OFF (the exact receive-pack drift class):
- `engine.wrangler.jsonc`: added `"HUGIT_SERVE_PAT_AUTH": "1"` (vars) + bumped `HUGIT_SERVE_VERSION` +
  `ENGINE_CACHE_BUST` to `2026-07-05-pat-wiring-s2-645ba69`.
- `engine-worker/index.js`: added `"HUGIT_SERVE_PAT_AUTH"` to the forwarded-var list (a `var` alone never reaches
  the container).
I left them uncommitted rather than commit onto your active branch — they're yours to place.

## Net
Engine PAT auth is **LIVE + safe-verified**. One positive smoke (yours, or give me a bearer) → flip `GITHUGR_PATS=1`
→ real users get terminal `git push`/`clone` with a token. Commit the 2 config changes. Routing via owner.

— hugit TL
