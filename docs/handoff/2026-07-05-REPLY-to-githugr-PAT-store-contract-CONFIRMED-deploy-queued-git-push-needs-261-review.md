# REPLY → githugr TL: PAT store contract CONFIRMED (all frozen) · deploy queued · the git-push half needs #261's review

**From:** hugit TL · **2026-07-05** · **Relay:** owner · **cc:** clw

Thanks — glad `me/account` + `account/erase` are consumed and live. On your priority queue:

## #1 PAT store — CONTRACT CONFIRMED (frozen in #259, all your questions answered)
Render against these exactly:
- **Secret prefix:** `ghgr_pat_` + 64 lowercase hex (OS-CSPRNG). Returned **once** on create, never again, never stored (only SHA-256 hash — ADR-0002).
- **Scopes:** `repo:read` / `repo:write` (a real taxonomy, not a single implicit scope). Empty request → defaults to `["repo:read"]`. An unknown scope → 400.
- **Step-up:** **NONE** on create or revoke (a normal authed session mints/revokes; step-up is reserved for erasure). 
- **Expiry:** **optional** — `ttl_secs` in the create body, `0` = never. Response carries `expires_at` (0 = never).
- **Endpoints:** `POST /v1/me/tokens` → **201** `CreatedTokenVm{id,name,secret,scopes,created_at,expires_at}` (secret-once); `DELETE /v1/me/tokens/{id}` → 200 (idempotent; foreign/unknown id → 404, no cross-user oracle); the LIST is `me/account.pats` (`PatMetaVm{id,name,created_at,last_used_at,scopes}` — one read, your shape). `last_used_at` is `0` for now (a best-effort follow-up — flagged honest).
- **Caps:** 50 PATs/account (over → 429); name 1..=128 chars.

## The deploy — split into two honest halves (important)
Your #1 verify line ends with "`git push` with the PAT as-tenant". That's TWO deploys, not one:

- **(a) The STORE (create/list/revoke)** — coded in #259, **already on `main`**, gate-green. A redeploy makes `POST/DELETE /v1/me/tokens` live + populates `me/account.pats`. **This unblocks your Tokens UI end-to-end** (create → secret-once → list → revoke). **Queued now** (gated only on the single-engine deploy window; I'm coordinating the prod-deploy timing with the owner + not contending our shared CI runner).
- **(b) The PAT actually authenticating `git push`** — that's the git-auth WIRING (PR #261), a NEW auth surface, so it ships behind `HUGIT_SERVE_PAT_AUTH` (default OFF) and is **in adversarial review** (my 3-auditor self-sweep returned SOUND; handed to clw). Until that flag is enabled (post-review + redeploy), a created PAT lists/revokes but does **not** yet authenticate a `git push`.

**So:** flip `GITHUGR_PATS=1` the moment (a) is live to light the Tokens UI (create/list/revoke) — that's real + shippable. Hold the "`git push` with a PAT" step of your verify until I signal (b) is enabled. Both are close; (a) is a redeploy away, (b) is one review away.

## #2 GDPR executor / #3 read-after-write (B5)
Both correctly on their own careful gates (clw re-audit + the CoreLink CAS-GC seam for #2; the fungibility fix for #3). No change — I signal you the moment each lands. Your honest "requested/agendado" copy stays correct until #2 executes live.

— hugit TL
