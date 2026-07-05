# REVIEW VERDICT → hugit engine TL — PR #261 (PAT git-auth wiring): FIX-FIRST. All 6 invariants sound, both your fixes hold, base64 benign. ONE must-fix before `HUGIT_SERVE_PAT_AUTH=1`: cap the token TTL. Grounded on head `982de33`.

> **From:** clw coordinator (independent cold adversarial review) · **Relay:** owner · **Date:** 2026-07-05
> Cold reviewer, diff-only, prompted to REFUTE + re-derive from code (did NOT trust your self-audit). I
> spot-re-verified the one must-fix myself against `982de33` before signing.

## ✅ The 6 invariants — SOUND (verified file:line, not asserted)
- **INV1 PAT-never-operator: PASS** — a PAT resolves only to `pat.created.user` (a `clerk:{org}:{user}` from
  `derive_owner_tenant`, which refuses any non-`clerk:` prefix); `is_operator` classifies Operator only for
  `orchestrator:`. Tier-1.5 (`two_tier_auth_ctx`, server.rs:340-349) runs BEFORE the dev-token and `return`s on a
  hit → no fall-through to the god-path. Downstream `authorize_write` still gates cross-tenant.
- **INV2 read-only-can't-write: PASS** — every mutation routes through `write_auth` (server.rs:381) → 403
  `SCOPE_INSUFFICIENT` unless `pat.can_write()`; git push checks `!ctx.write_ok → 403` in both receive-pack arms;
  the mint route 403s read-PAT (scope) AND write-PAT (`PAT_CANNOT_MINT`).
- **INV4 no cross-user leak / no hash-forgery: PASS** — index key = `sha256(secret)`, value = the minter's own
  principal; presented secret is hashed and looked up (one-way) — a leaked stored hash can't forge a token; Basic
  decode discards the username so identity can't be redirected.
- **INV5 no panic / O(1): PASS** — `decode_base64_std` / `candidate_pat_secrets` / `resolve_pat` are total on
  adversarial `Authorization` (malformed base64, non-UTF8, empty, prefix edges); poison-safe locks; O(1) in-memory
  lookup, no per-request R2; header size bounded upstream by tiny_http.
- **INV6 multi-instance fail-closed: PASS** — `pat_auth_multi_instance_guard` hard-fails boot (`?` on
  `Result<Self,String>`) when `pat_auth_enabled && allow_multi_instance`; unit-tested.
- **FIX-1 (detached boot index): HOLDS** — detached thread, no join/deadline blow-up; warm-up window fails CLOSED
  for auth (empty index → 401/anon), never fail-open; merge-not-clear preserves a warm-up create; `catch_unwind`
  isolates a scan panic to an empty (still fail-closed) index.
- **FIX-2 (`PAT_CANNOT_MINT`): HOLDS** — `is_pat` set only on Tier-1.5; mint route refuses a PAT after `write_auth`;
  revoke stays open to a PAT (DELETE uses `two_tier_auth`, no `is_pat` gate).
- **base64 low-note: CONFIRMED BENIGN** — non-canonical `len%4∈{2,3}` decodes to different bytes → different
  sha256 → miss; no forgery. Leave as-is.

## ⛔ ONE must-fix before you flip `HUGIT_SERVE_PAT_AUTH=1` (INV3 is PARTIAL because of it)
**The boot warm-up revoke race on a never-expiring PAT — confirmed in code, not theoretical:**
- `token_create` (write_token.rs:404): `ttl_secs == 0 → expires_at = 0`, and `is_expired` (write_token.rs:182)
  is `expires_at != 0 && …` → **`expires_at==0` NEVER expires. There is no server-side max-TTL cap.**
- `boot_build_pat_index` (state.rs:1450) is **insert-only** (`idx.insert`, :1467) and merges a pre-revoke
  snapshot — your own comment at state.rs:1446 documents it ("`pat.revoked` post-dates the scan's read: the
  scanned still-live entry is re-added"). During warm-up the index is empty, so a concurrent revoke's
  `pat_index_remove` (:1504) is a **silent no-op**, then the scan re-inserts the still-live entry.
- **Net:** a leaked `ttl_secs=0` token revoked during the (per-boot, attacker-can't-time but real) warm-up window
  keeps authenticating with **full owner scope until the next reboot**, and the revoke **reports success**. An
  unbounded, silent revocation-evasion. Steady-state crash-window eviction-skips self-heal; this one does not,
  because never-expire is an allowed (and likely default) choice.

**Must-fix (cheap, converts unbounded → time-bounded self-heal):** enforce a **server-side non-zero MAX token
TTL** — cap `expires_at` in `token_create` (reject or clamp `ttl_secs==0`/over-max to a policy ceiling, e.g. 90d).
Then every token self-heals and the warm-up/crash eviction-skip is bounded. (Stronger alternative — consult the
durable `pat.revoked` tombstone on the hot path — costs the O(1) property; the TTL cap is the pragmatic gate.)
Flag OFF today makes this a pre-enable gate, not an incident — but it must land before enable.

## Two notes for the enable-runbook (not code must-fixes)
- **EXTRA-1 (LOW, defense-in-depth):** the multi-instance guard trusts `HUGIT_SERVE_ALLOW_MULTI_INSTANCE` as the
  instance-count truth (state.rs:873). If a deploy sets CF `max_instances>1` in wrangler WITHOUT that env var, the
  guard silently permits PAT auth on >1 instance (eviction-consistency + intermittent-401 problems). Pin in the
  enable-runbook: the two settings MUST move in lockstep. (Pre-existing convention, not new; not a false-accept.)
- **EXTRA-2 (INFO):** `caller_identity` uses `principal_chain.last()` while `derive_owner_tenant` uses `.first()`;
  they coincide for the only reachable single-element `clerk:` chain today. Harmless now — note it if the chain
  shape ever grows.

## Verdict
**FIX-FIRST** — land the max-TTL cap, then APPROVE-to-enable. Everything else is sound; the kernel (tiering,
scope-gating, no-forgery, no-panic, multi-instance fail-closed) is right. When the cap lands, re-point me and I
confirm the one delta, then you flip the flag + live-verify (single-instance; not on `max_instances>1` until B5).

— clw coordinator (independent cold adversarial review)
