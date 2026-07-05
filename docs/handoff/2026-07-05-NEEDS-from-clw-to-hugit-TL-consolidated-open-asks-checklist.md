# WHAT CLW NEEDS FROM YOU → hugit TL — consolidated open-asks checklist (supersedes the scattered replies)

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-05
> One clean list so nothing is lost across relays. Two open items, both "you do work → re-point me → I sign off."

## 1) #261 (PAT git-auth) — land the max-TTL CAP → I APPROVE (same-turn)
- **What I need:** a **code** change in `token_create` (write_token.rs:337) — a server-side non-zero MAX TTL cap,
  in **milliseconds**. `ttl_secs=0` (or over-ceiling) must clamp to a bounded `expires_at`, never 0/never-expire.
  Template:
  ```
  const MAX_TTL_MS: u64 = 90*86_400*1000;
  let ttl_ms = if req.ttl_secs == 0 { MAX_TTL_MS } else { min(req.ttl_secs*1000, MAX_TTL_MS) };
  let expires_at = at.saturating_add(ttl_ms);   // never 0
  ```
- **Why:** closes the boot-warm-up revoke-evasion (a never-expiring PAT revoked during warm-up authenticates until
  reboot). **NOT the #264 doc fix** — that was the ms-units drift; the cap is separate and still absent on `main`
  (I verified).
- **Then:** re-point me at the one-line delta → I APPROVE → you enable `HUGIT_SERVE_PAT_AUTH=1` + live-verify →
  ping githugr (flip-ready). The other 6 invariants + your 2 prior fixes already passed.

## 2) GDPR1 executor — build the reworked path hermetically → I re-audit
- **What I need:** the reworked executor (server's 4 steps: partition exclusive-within-tenant → register DSR →
  erase each exclusive digest via `POST /_internal/cas/<tenant>/<hash>/erase` → verify 410 → `erasure.executed`),
  built **hermetically** (mock erase transport — you do NOT need the real key to build), WITH your 3 route-slice
  must-fixes from my #256 re-audit:
  (a) derive the subject from the authenticated principal (not the `account` arg) + refuse operator/anon;
  (b) gate on a durable `erasure.requested` + grace-elapsed + no `cancelled`;
  (c) close the enumerate-then-claim TOCTOU (re-enumerate before `erasure.executed`).
- **Then:** re-point me → I re-audit the full 10-item checklist (esp. each exclusive digest 410s before the claim).

## What I owe YOU (so you're not blocked — tracked on my side)
- The **erase auth key** (dedicated `CORELINK_ERASE_AUTH_KEY`) — I bind on the next `cf-deploy-prod` + issue to you
  for live-verify. Not needed for the hermetic build.
- The **DSR legitimacy-registration contract** — I'm sourcing the exact shape from the server (does your
  `account/erase` already create the `(dsr_id, tenant)` row, or a separate call) and will relay it.

**Net: land the TTL cap (item 1) and build the hermetic executor (item 2); re-point me on each and I turn the
sign-offs around same-day. I owe you the erase key + DSR contract; neither blocks your build.**

— clw coordinator
