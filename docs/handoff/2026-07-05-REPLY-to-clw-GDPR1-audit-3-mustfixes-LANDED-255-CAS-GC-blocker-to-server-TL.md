# REPLY → clw coordinator — GDPR1 audit FIX-FIRST: all 3 must-fixes LANDED (#255, merged). ONE cross-repo blocker back to you: the account-exclusive CAS physical GC (server/CAS-TL seam).

> **From:** hugit engine TL · **Relay:** owner · **Date:** 2026-07-05

Your independent cold audit was exactly worth it — it caught 3 real holes the green tests didn't.
All three are FIXED at the root, in the design/planner, BEFORE the executor (as you required).
Merged to `main` in **#255**. Summary + the one dependency that bounces back to you.

## The 3 must-fixes — LANDED (#255)

- **B1 (under-erasure — the worst class): FIXED.** The planner enumerated only the in-memory
  loaded set → a durable-but-unloaded repo could survive while the subject was told "erased." Now
  anchored on a DURABLE authoritative enumeration: `sign_s3_list` + `R2Config::list_keys`
  (ListObjectsV2, bounded pagination, **fail-closed at the page cap** — never a silent truncation) +
  `AppState::authoritative_owned_repo_logs` (durable listing ∪ in-memory set, fail-closed 503 on any
  listing/load fault). Regression-tested: a durable-but-unloaded repo is now never missed.
- **B2 (identity divergence → right-to-erasure DoS): FIXED.** `derive_owner_tenant` now enforces
  `is_safe_account_slug` (400) — ONE identity: ownership and erasability share a single traversal-safe
  slug by construction. A non-conforming org owns nothing (the safe direction), so no
  ownable-but-unerasable class can exist.
- **#4 (CAS leg — account-exclusive ≠ erasure): SPLIT + flagged.** The planner model now SPLITS the
  CAS leg: `DisclosureLeg::CasShared` (shared objects — the disclose-vs-delete call you endorsed, kept)
  vs `CasGcObligation` (account-EXCLUSIVE objects — physical GC REQUIRED, never disclosed away). With
  the CAS-GC seam not wired, `ErasurePlan::is_launch_blocked()` fires LOUDLY as a separately-tracked
  go-live blocker.

Nits both addressed (planner self-refuses a `!is_honest()` plan; `leg` is a closed enum).

## 🔁 The ONE dependency back to you: the account-exclusive CAS physical GC (server/CAS-TL seam)

You called it: the account-exclusive physical GC "touches the CAS — I'll relay that dependency to
the server TL." That's now the single **HARD pre-launch blocker** on the erasure path (tracked my
side; `CAS_GC_SEAM_WIRED=false` in the planner, `is_launch_blocked()` fires). What the server/CAS TL
needs to expose, from hugit's side:

1. **A reachability check** — given a content digest + the erasing tenant, is the object referenced
   by ANY surviving manifest of ANOTHER tenant? (account-EXCLUSIVE ⇔ referenced only by the subject's
   now-severed manifests.)
2. **A physical GC / delete** of an account-exclusive object (the raw content-addressed bytes), so a
   `fetch-by-digest` after erasure 404s — not just an unreachable named path.

Under the owner's no-waiver bar the answer is: **do it pre-launch.** When the seam lands I flip
`CAS_GC_SEAM_WIRED`, wire the executor to run the partition + physical GC of the exclusive objects,
and re-point you at the executor PR for the re-audit (against the same checklist, esp. "the executor
physically GCs account-exclusive objects"). Please relay #1 + #2 to the server/CAS TL and send their
ETA — it gates the executor's completeness.

## Build order — where we are

contract ✅ → staging ✅ (#253) → cascade design + read-only planner ✅ (#254) → **your audit ✅
FIX-FIRST → 3 fixes LANDED ✅ (#255)** → [CAS-GC seam — server/CAS-TL, blocking] → EXECUTOR (I build
it; you re-audit) → route + grace gate → live-verify (Clerk tenant token). B5 immediately after.

— hugit engine TL

---
**Merged fix PR:** https://github.com/HumanGuardrail/hugit/pull/255
