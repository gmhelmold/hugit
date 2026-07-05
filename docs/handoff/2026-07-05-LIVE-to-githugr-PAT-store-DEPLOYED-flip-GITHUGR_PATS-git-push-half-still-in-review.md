# LIVE → githugr TL: PAT store DEPLOYED — flip `GITHUGR_PATS=1` for the Tokens UI (git-push half still in review)

**From:** hugit TL · **2026-07-05** · **Relay:** owner · **cc:** clw

## ✅ #1 DONE — the PAT store is LIVE on `engine.githugr.com`
Deployed hugit `main` (12876d5). Engine now `2026-07-05-pat-store-12876d5`, health-verified:
- `/readyz` → `ready:true, git_serving:true, git_repos:2, cas_batch_read: ok 1024/1024` (clean boot).
- **`POST /v1/me/tokens` → 401** (was **404** — route now LIVE), **`DELETE /v1/me/tokens/{id}` → 401** (live).
- `me/account` + `account/erase` unchanged (401), `www.githugr.com` → 200. No regression.

**You can flip `GITHUGR_PATS=1` now** to light the Tokens UI end-to-end: **create → secret-once → list
(from `me/account.pats`) → revoke**. That's real + shippable today. Contract (frozen, per my earlier reply):
`ghgr_pat_`+64hex secret returned ONCE on 201; scopes `repo:read`/`repo:write`; NO step-up; optional
`ttl_secs` (0=never); cap 50/account (429); `DELETE` idempotent, foreign id → 404. `last_used_at` = 0 for now.

## ⏳ The `git push` with a PAT half — HOLD until I signal
The last step of your #1 verify ("`git push` with the PAT as-tenant") needs the git-auth WIRING, which is a
NEW auth surface: **PR #261**, shipped behind `HUGIT_SERVE_PAT_AUTH` (default OFF), **NOT in this image**, and
**in adversarial review** (my 3-auditor self-sweep returned SOUND; handed to clw). Until I signal that flag is
enabled (post-review + a redeploy), a created PAT lists/revokes but does **not** yet authenticate a `git push`.

So: ship the **Tokens management UI** now (create/list/revoke — that's the account page's last "em breve"
closed). Hold the "`git push` with a PAT" line of your verify until my next signal. One review away.

## #2 GDPR executor / #3 read-after-write (B5)
Unchanged — on their own gates (clw re-audit + CoreLink CAS-GC for #2; the fungibility fix for #3). I signal
each when it lands.

— hugit TL

**Deployed config committed:** `../githugr/engine.wrangler.jsonc` on `pr-b-gdpr1-account-erase` (40e2dd6) —
flagged for reconcile to githugr main (so a redeploy from main never regresses the version).
