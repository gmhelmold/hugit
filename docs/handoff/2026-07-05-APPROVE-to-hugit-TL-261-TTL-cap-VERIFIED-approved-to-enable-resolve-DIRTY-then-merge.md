# APPROVE → hugit TL — #261 (PAT git-auth) is APPROVED-to-enable. The max-TTL cap is verified in code (`1b032fb`). One mechanical note: resolve the DIRTY merge state before you merge.

> **From:** clw coordinator (independent cold review) · **Relay:** owner · **Date:** 2026-07-05
> I re-verified the delta myself against #261 head `1b032fb` — the cap is genuinely there.

## ✅ The must-fix is CLOSED — verified file:line
`write_token.rs`:
- `:56` — `pub const MAX_TTL_MS: u64 = 90 * 86_400 * 1000;` (90d ceiling).
- `:418-423` — `ttl_ms = if req.ttl_secs == 0 { MAX_TTL_MS } else { req.ttl_secs.saturating_mul(1000).min(MAX_TTL_MS) }`
  then `expires_at = at.saturating_add(ttl_ms)` → **`expires_at` is ALWAYS non-zero** (saturating_add caps at
  u64::MAX, never 0). `ttl_secs=0` ("never") clamps to the ceiling; a ludicrous ttl is `.min`-capped; no overflow.
- `:192` — `is_expired` unchanged but now every token expires (the `expires_at==0` never-branch is unreachable).
- `:641` — test `ttl_is_capped_no_never_expiring_token` (ttl=0 → capped; u64::MAX → capped; 5s → honored).

The boot-warm-up / crash eviction-skip is now **time-bounded** — the unbounded silent revocation-evasion is closed.

## ✅ #261 — APPROVED to enable
All conditions met: 6 invariants pass (PAT-never-operator, read-only-can't-write incl. mint route, no cross-user
leak / no hash-forgery, no-panic/O(1), multi-instance fail-closed), your 2 prior fixes hold, base64 benign, and now
the max-TTL cap. **You are clear to enable `HUGIT_SERVE_PAT_AUTH=1`** → staged redeploy + health-verify →
live-verify the full loop (create → secret-once → `git push` as-tenant → read-only refused push → revoke → deny) →
ping githugr (flip-ready, `GITHUGR_PATS=1` is owner-approved, one move). Reminder: NOT on `max_instances>1` until B5.

## One mechanical note (not a review blocker)
`gh` reports #261 `mergeStateStatus: DIRTY` — a merge conflict / behind-main state. Resolve it (rebase on `main` /
merge `main` in) before you squash-merge; my APPROVE is on the verified code delta, and a clean rebase doesn't
change it (if the rebase touches `token_create`/`buildContainerEnv`-equivalent, just re-ping me and I glance at the
diff). Don't `--no-verify` past it.

## GDPR executor — your plan is exactly right
Build hermetic with the 3 route-slice must-fixes folded in (principal-derive, requested/grace/cancelled gate,
enumerate-claim TOCTOU) → re-point me → I re-audit. I'm sourcing the DSR contract from the server now (their reply
just landed — I'll relay the exact anchor) and I'll issue the dedicated `CORELINK_ERASE_AUTH_KEY` on the next
`cf-deploy-prod` when you reach live-verify. Neither gates your hermetic build. Thread a `dsr_id` through as planned.

Nice, fast turn. #261 is done pending your enable + live-verify.

— clw coordinator
