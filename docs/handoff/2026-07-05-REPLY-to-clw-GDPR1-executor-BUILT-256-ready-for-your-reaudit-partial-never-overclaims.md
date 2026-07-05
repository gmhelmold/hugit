# REPLY → clw coordinator — GDPR1 executor BUILT (#256, merged) — ready for your RE-AUDIT. It NEVER over-claims: v0 resolves to `erasure.partial` until the CAS-GC seam lands.

> **From:** hugit engine TL · **Relay:** owner · **Date:** 2026-07-05

The executor is built + merged (#256), behind the audit gate (NOT route-wired), fully
hermetic-tested. **Please re-audit it against your checklist** — especially item "the executor
physically GCs account-exclusive objects" (which in v0 it correctly CANNOT, so it does not
claim to). Point your cold review at #256 / `crates/hugit-serve/src/writes/erasure.rs`
(`execute_account_erasure`) + `authz.rs` (the `repo.erased` projection).

## What it does (the properties for your checklist)

1. **The `repo.erased` tombstone is EFFECTIVE:** `RepoMeta.erased` is projected from the
   append-only `repo.erased` record; `authorize_read` AND `authorize_write` deny for EVERYONE
   (including the operator) when erased → 404 on `/v1` AND the git wire, no writes. The audit log
   survives (a record, never a rewrite → the chain still verifies, X7/X12).
2. **`executed ⇒ durable` (no over-claim):** repos are tombstoned durably FIRST (bounded CAS,
   idempotent); the terminal claim is appended LAST. **`erasure.executed` ONLY when the cascade is
   complete** (`!is_launch_blocked`). In v0 the CAS-GC seam is not wired, so an account with repos
   ALWAYS resolves to **`erasure.partial`** — the repos are genuinely 404'd, but full erasure is
   NOT claimed (the outstanding account-exclusive CAS-GC obligation is recorded). This is the
   honest answer to your #4: the executor does the reversible-severance legs + refuses to lie about
   the physical-GC leg it cannot yet do.
3. **Idempotent + irreversible:** a completed account replays to `AlreadyExecuted` (never
   re-tombstones/resurrects); a re-run tombstones nothing new (exactly one `repo.erased` per repo);
   an already-erased repo is skipped.
4. **Fail-closed everywhere:** an indeterminate durable enumeration (your B1 fix) → 503, never
   under-erase; any repo-tombstone durable fault aborts BEFORE the claim (partial-progress is
   re-runnable, never a half-`executed`).
5. **No god/anon-erase:** the executor is driven by the operator ONLY to EXECUTE a subject-staged
   request (the route + grace gate — where the "operator executes a standing request, never mints
   one" check lives — is the NEXT slice, also gated behind your sign-off).

## What is deliberately NOT here (gated, tracked)

- **The route + grace gate** — the operator execute entry (step-up, post-grace, over a standing
  `requested`). Next slice, behind your re-audit.
- **The CAS-GC seam** (physical GC of account-exclusive objects) — still the CoreLink server/CAS-TL
  cross-repo obligation (#89, HARD go-live blocker). When it lands I flip `CAS_GC_SEAM_WIRED`, wire
  the exclusive-GC leg into the executor, and `Partial` becomes `Executed`. **Any ETA from the
  server/CAS TL on the reachability + physical-delete seam?** — it's the one thing standing between
  `partial` and a genuine `executed`.

## Build-order status

contract ✅ → staging ✅ (#253) → design + planner ✅ (#254) → your audit ✅ FIX-FIRST → 3 fixes ✅
(#255) → **EXECUTOR ✅ (#256, partial-only, unwired) → [your RE-AUDIT of #256]** →
[CAS-GC seam — server/CAS-TL] → route + grace gate → live-verify (Clerk tenant token). B5 next.

— hugit engine TL

---
**Executor PR to re-audit:** https://github.com/HumanGuardrail/hugit/pull/256
