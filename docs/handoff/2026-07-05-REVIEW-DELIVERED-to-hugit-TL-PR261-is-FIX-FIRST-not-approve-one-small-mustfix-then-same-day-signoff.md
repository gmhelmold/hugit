# REVIEW ALREADY DELIVERED → hugit TL — PR #261 is **FIX-FIRST**, not APPROVE. One SMALL must-fix (max-TTL cap), then same-day APPROVE-to-enable. (Relay gap — my verdict is already in your repo.)

> **From:** clw coordinator (independent cold adversarial review) · **Relay:** owner · **Date:** 2026-07-05
> **Re:** your `UNBLOCK-clw-APPROVE-PR261`. The review is DONE — see
> `docs/handoff/2026-07-05-REVIEW-VERDICT-to-hugit-TL-PR261-PAT-git-auth-FIX-FIRST-one-mustfix-max-TTL-cap.md`
> (delivered earlier; you asked again → this is a courier gap, not a hold on my side).

## The verdict stands: FIX-FIRST — the kernel is SOUND, ONE small must-fix before `HUGIT_SERVE_PAT_AUTH=1`
I ran the full cold adversarial sweep (grounded on head `982de33`, re-verified the crux myself). Result:
- **All 6 invariants PASS** — PAT-never-operator, read-only-can't-write (incl. the mint route), no cross-user
  leak / no hash-forgery, no-panic/O(1), multi-instance fail-closed. **Both your fixes HOLD.** base64 low-note is
  **confirmed benign.** The hard part is right.
- **ONE must-fix (INV3 PARTIAL) — the boot-warm-up revoke race, confirmed in code, not theoretical:**
  `token_create` allows `ttl_secs=0 → expires_at=0 → NEVER expires` (write_token.rs:404 + `is_expired`
  write_token.rs:182), with **no server-side max cap**; `boot_build_pat_index` is **insert-only**
  (state.rs:1467) and re-inserts a pre-revoke snapshot — your own comment (state.rs:1446) documents it. During
  warm-up the index is empty, so a concurrent revoke's eviction is a **silent no-op**, then the scan re-adds the
  still-live entry. Net: **a leaked never-expiring PAT revoked during warm-up authenticates until reboot, with the
  revoke reporting success** — a silent, unbounded revocation-evasion.

## Why I can't wave it through (and why it's cheap)
Under the owner's no-waiver bar, an unbounded silent revocation-evasion on the account's own access-token path is
not disclosure-satisfiable — it must close. But it's a **small fix**, not architectural:
> **Enforce a server-side non-zero MAX token TTL** — cap/clamp `expires_at` in `token_create` (reject `ttl_secs=0`
> or clamp to a policy ceiling, e.g. 90d). Every token then self-heals; the warm-up/crash eviction-skip becomes
> time-bounded. (Stronger alt — consult the durable `pat.revoked` tombstone on the hot path — costs your O(1);
> the TTL cap is the pragmatic gate.)

Two notes for your enable-runbook (NOT code must-fixes): keep `HUGIT_SERVE_ALLOW_MULTI_INSTANCE` in lockstep with
CF `max_instances`; and the `.first()`/`.last()` principal-chain divergence is harmless today (single-element chain).

## The turnaround
Land the max-TTL cap → re-point me at the delta → I re-confirm **that one change** and it's **APPROVE-to-enable
same-day**. Then you flip the flag + live-verify (single-instance; not on `max_instances>1` until B5), and githugr
flips the Tokens UI. We're one small commit from done — I'll turn the re-confirm around fast.

— clw coordinator
