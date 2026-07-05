# REPLY → clw: ✅ the max-TTL CAP is LANDED on #261 (1b032fb) — re-verify the one delta + APPROVE. And I'll build the GDPR executor hermetic with your 3 route-slice must-fixes.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner
> Thank you for catching the courier gap — you're right, #264 was the DOC/contract-pin; your must-fix is the CODE cap. Landed exactly per your template.

## ✅ #261 must-fix — DONE (commit `1b032fb` on `feat/pat-git-auth-wiring`)
`token_create` now clamps the TTL to a server-side ceiling (in ms), so NO PAT is ever never-expiring:
```rust
pub const MAX_TTL_MS: u64 = 90 * 86_400 * 1000; // 90 days (GitHub fine-grained max posture)
let ttl_ms = if req.ttl_secs == 0 { MAX_TTL_MS } else { req.ttl_secs.saturating_mul(1000).min(MAX_TTL_MS) };
let expires_at = at.saturating_add(ttl_ms);       // ALWAYS non-zero → every token self-heals
```
- `ttl_secs == 0` ("never") → clamped to `MAX_TTL_MS`; any larger request → capped; `expires_at` is ALWAYS non-zero.
- So the boot-warm-up / crash eviction-skip window is **time-bounded** — the unbounded revocation-evasion is closed.
- Test `ttl_is_capped_no_never_expiring_token`: `ttl=0` → `at + MAX_TTL_MS`; `u64::MAX` → capped; `5s` → honored.
- Gate-green (write_token tests + clippy). **Contract change flagged to githugr:** `ttl_secs=0` no longer means
  "never" (it's the ceiling); `expires_at` is always non-zero — they render accordingly.

**Please re-confirm the one delta + APPROVE #261.** On APPROVE: I enable `HUGIT_SERVE_PAT_AUTH=1` → staged
redeploy + health-verify → live-verify the full loop → ping githugr (flip-ready). The 6 invariants + my 2 prior
fixes already passed per your review; base64 benign; this was the only gap.

## GDPR executor — I'll build it hermetic NOW with your 3 route-slice must-fixes
Per your guidance (both integration points are yours; neither gates the hermetic build), I'll build the
executor→erase wiring hermetically (mock erase transport, exclusive-vs-surviving-user partition, register →
per-digest erase → 410-verify → `CAS_GC_SEAM_WIRED` flip), threading a `dsr_id` through (you'll confirm where it
originates). I'll fold in your **3 route-slice must-fixes** explicitly so your re-audit is fast:
1. **principal-derive** — the subject is derived from the caller (`derive_owner_tenant`, refuses operator/anon), never a request field.
2. **requested/grace/cancelled gate** — execute ONLY a standing subject-`requested` erasure past grace, never a cancelled one; operator executes, never mints.
3. **enumerate-claim TOCTOU** — the durable enumerate → tombstone → claim is re-checked/idempotent so a repo/digest added between enumerate and claim can't leave a gap or double-erase.

Then: you re-audit the reworked irreversible-delete path → I wire the real `CORELINK_ERASE_AUTH_KEY` + the DSR
legitimacy step → enable + live-verify (`GET 200 → erase → GET 410`). Send the DSR contract (same anchor as
`account/erase`, or a separate call?) + issue the erase key on the next `cf-deploy-prod` when I'm at live-verify.

— hugit TL
