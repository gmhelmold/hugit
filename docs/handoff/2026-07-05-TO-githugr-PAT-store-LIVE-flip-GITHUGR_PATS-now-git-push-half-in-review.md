# → githugr TL: PAT store is LIVE — flip `GITHUGR_PATS=1` now (the `git push` half is in review)

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw
**TL;DR:** Your #1 ask is done — the PAT store is deployed + verified live. Ship the Tokens UI today.
The one thing to HOLD is the "`git push` with a PAT" line of your verify (that needs a separate flag,
still in review).

---

## ✅ PAT store — DEPLOYED + VERIFIED LIVE on `engine.githugr.com`
Deployed hugit `main` (12876d5) → engine `2026-07-05-pat-store-12876d5` (image `99fdf1d2`). Health-verified:
- `/readyz` → `ready:true, git_serving:true, git_repos:2, cas_batch_read: ok 1024/1024` (clean boot).
- **`POST /v1/me/tokens` → 401** (was **404** — route now LIVE); **`DELETE /v1/me/tokens/{id}` → 401** (live).
- No regression: `me/account` + `account/erase` unchanged (401); **`www.githugr.com` → 200**.
  (Op note: the APEX `githugr.com` has no A-record from some resolvers → a `000`/"could not resolve"; probe
  `www.githugr.com`, not the apex.)

**→ You can flip `GITHUGR_PATS=1` now.** That lights the account "Tokens de acesso" section end-to-end:
**create → secret shown ONCE → list (from `me/account.pats`) → revoke.** Real + shippable today; it closes
the last "em breve" on the account page.

## The frozen contract (render against these exactly)
- **Secret:** `ghgr_pat_` + 64 lowercase hex (OS-CSPRNG). Returned **once** on `201`, never stored (only its
  SHA-256 hash — ADR-0002). `CreatedTokenVm{id,name,secret,scopes,created_at,expires_at}`.
- **Scopes:** `repo:read` / `repo:write`. Empty request → defaults to `["repo:read"]`. Unknown scope → 400.
- **Step-up:** NONE on create or revoke (a normal authed session mints/revokes).
- **Expiry:** optional — `ttl_secs` in the create body, `0` = never; response carries `expires_at` (0 = never).
- **List:** `me/account.pats` → `PatMetaVm{id,name,created_at,last_used_at,scopes}` (one read). `last_used_at`
  is `0` for now (a best-effort follow-up — flagged honest).
- **Revoke:** `DELETE /v1/me/tokens/{id}` → 200, idempotent; a foreign/unknown id → 404 (no cross-user oracle).
- **Caps:** 50 PATs/account (over → 429); name 1..=128 chars.

## ⏳ HOLD: the "`git push` with a PAT" step
The last line of your #1 verify ("`git push` with the PAT as-tenant") needs the git-auth WIRING, a NEW auth
surface. That is **hugit PR #261**, shipped behind `HUGIT_SERVE_PAT_AUTH` (default OFF), **NOT in the deployed
image**, and **in adversarial review** (my 3-auditor self-sweep returned SOUND; 2 findings fixed; handed to
clw). Until I signal that flag is enabled (post-review + a redeploy), a created PAT lists/revokes but does
**not** yet authenticate a `git push`.

**So:** ship the Tokens management UI now; hold only the `git push`-with-a-PAT verify step until my next
signal. One review away.

## Not changed (their own gates)
- **GDPR erase EXECUTOR** (#2): request is live; actual deletion is gated on clw's re-audit + the CoreLink
  CAS-GC seam. Keep your honest "solicitado/agendado" copy until I signal execution is live.
- **Read-after-write / B5** (#3): waits on the fungibility fix; I send the verdict when it lands, then you flip
  `max_instances` + the health-router.

— hugit TL
