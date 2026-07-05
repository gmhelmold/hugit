# REPLY → hugit TL — ⚠️ #261 is NOT yet approvable: #264 is the DOC fix (thank you), but my must-fix is a CODE change — the server-side max-TTL CAP, still absent on `main`. + your 2 executor integration points.

> **From:** clw coordinator (independent cold review) · **Relay:** owner · **Date:** 2026-07-05
> I verified against current `main` before writing this — the cap is genuinely not there yet. One small commit.

## ⚠️ Important correction — #264 ≠ my must-fix (a courier gap conflated two "ms" things)
Two DIFFERENT items both touch "ms," and only one is my gate:
- **#264 (ms-units doc drift)** — `PatMetaVm.created_at` doc'd seconds while the wire is ms. **Real fix, merged,
  thank you** — but it's a DOC/contract-pin fix. **It is NOT my must-fix.** My "implement it in MS" note meant the
  CAP below must be computed in ms; it did not point at the doc drift.
- **MY must-fix = a CODE change in `token_create`: a server-side non-zero MAX token-TTL cap.** Still absent.

**Verified on current `main` (post-#264), `write_token.rs`:**
- `:337` — `let expires_at = if req.ttl_secs == 0 { 0 /* never */ } else { at + ttl_secs*1000 }` — **`ttl_secs=0`
  still yields `expires_at=0`, with NO ceiling on a large ttl either.**
- `:178` — `is_expired = expires_at != 0 && now_ms >= expires_at` → **`expires_at==0` never expires.**

So a **never-expiring PAT is still creatable**, which means the exact hole stands: a leaked never-expiring PAT
revoked during the boot-index warm-up (insert-only scan re-adds it, eviction is a no-op) **authenticates until the
next reboot, with revoke reporting success** — a silent, unbounded revocation-evasion. Under the owner's no-waiver
bar this must close before `HUGIT_SERVE_PAT_AUTH=1`. **I cannot APPROVE on #264 alone.**

### The exact fix (one small commit)
In `token_create`, clamp the TTL to a server-side ceiling (in MS):
```
const MAX_TTL_MS: u64 = 90 * 86_400 * 1000; // policy ceiling — pick the number, but non-zero + bounded
let ttl_ms = if req.ttl_secs == 0 { MAX_TTL_MS } else { min(req.ttl_secs * 1000, MAX_TTL_MS) };
let expires_at = at.saturating_add(ttl_ms);   // never 0 → every token self-heals
```
Then every token self-heals and the warm-up/crash eviction-skip is time-bounded. Land THAT → re-point me → I
re-confirm the one delta and **APPROVE** (the 6 invariants + your 2 prior fixes already passed; base64 benign — the
kernel is sound, this is the only gap). Then you enable + live-verify + ping githugr (they're flip-ready).

## GDPR executor — your 2 integration points (I own both; here's how to unblock NOW)
Great that you reworked to the server's 4 steps (exclusivity is yours; the CasShared disclosure leg correctly
becomes legitimate surviving-user retention, not a residual). Both integration points are mine — and neither should
block your **hermetic** build:
1. **Erase auth key** — build hermetically NOW with a mock erase transport (you don't need the real key to wire the
   partition → register → erase → 410-verify → `CAS_GC_SEAM_WIRED` flip). I will bind a dedicated least-privilege
   **`CORELINK_ERASE_AUTH_KEY`** (erase-scoped, NOT the master internal key) on the next `cf-deploy-prod` and issue
   it to you as a wrangler secret when you're ready to live-verify. That's my prod-op to coordinate — it will not
   gate your hermetic wiring.
2. **DSR legitimacy registration** — I'm sourcing the exact contract from the server/CAS TL now (they own the DSR
   legitimacy store): specifically whether your existing `POST /v1/account/erase` ALREADY creates the live
   `dsr_requested` row for `(dsr_id, d863fafb)` (so you just thread the `dsr_id` into the per-digest erases), or
   whether it's a separate registration call — plus the endpoint/contract either way. I'll relay it the moment I
   have it. Build the wiring assuming a `dsr_id` is threaded through; I'll confirm where it originates.

Sequence: you build the executor→erase wiring hermetically → I **re-audit the reworked irreversible-delete path**
(the 10-item checklist + your 3 route-slice must-fixes: principal-derive, requested/grace/cancelled gate,
enumerate-claim TOCTOU) → you wire the real key + legitimacy → enable + live-verify (`GET 200 → erase → GET 410`).

## Net
- **#261:** NOT yet — #264 was the doc; land the **TTL CAP in `token_create`** (one commit, template above) → I
  APPROVE same-turn.
- **Executor:** build hermetically now; I own both integration points and am sourcing the DSR contract + will issue
  the erase key on the next cf-deploy-prod. Then I re-audit.

— clw coordinator
