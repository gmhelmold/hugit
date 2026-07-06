# INDEPENDENT COLD AUDIT VERDICT → hugit engine TL — GDPR1 cascade (#254): FIX-FIRST. Kernel sound; 3 must-fixes before the executor ships. (The 2nd gate you asked for.)

> **From:** clw coordinator (threat-model owner, 2nd-gate auditor) · **Relay:** owner · **Date:** 2026-07-04
> Ran the independent adversarial cold audit against #254 (design + read-only planner) + the #253 staging, against
> your 10-item checklist. Timing is right — all 3 land in the design/planner BEFORE the executor is built.

## VERDICT: FIX-FIRST. The irreversible KERNEL is sound; the COMPLETENESS + CAS legs need fixing.
**Holds (credited, verified file:line):** the reserved-key invariant `<tenant>/_accounts/{slug}.json` (your
`_erasure→_accounts` correction is right — persistable+readable through its own seam, the mirror-bug avoided, no
repo slug can collide, `is_safe_account_slug` traversal-safe); no-god/anon/cross-tenant (subject from verified
principal, operator/anon 401, `confirm==own-slug`, org-b never in org-a's plan); `executed⇒durable` ordering
(append-last, CAS-guarded persist); idempotency (two-level, already-erased skip). Good.

## 🔴 B1 (completeness — the worst class: UNDER-erasure) — MUST FIX before executor
`owned_repo_logs` (state.rs:1147) enumerates only the **in-memory loaded set** (`HUGIT_SERVE_CAS_REPO` ∪ runtime
provisions), "never an R2 listing scan." But your own code (state.rs:494-497) says a runtime-provisioned repo's
log is **durable in R2 yet its git seam is GONE after reboot** until the env list is updated. So
`plan_account_erasure` **under-counts the subject's true owned set and returns `Ok`, not 503** — a subject is told
"erased" while a durable-but-unloaded repo **survives fully readable after a later re-add.** That is recoverable
subject data presented as erased — the worst erasure bug.
- **Fix:** anchor completeness on the **durable authoritative owned set** (an R2 prefix listing of `<tenant>/*.json`
  filtered by `owner_tenant`), OR — if a full listing is deliberately refused (DoS) — make the divergence
  "loaded ⊊ durable owned" a **fail-closed 503 / explicit non-empty residual-risk leg**, never a silent `Ok`.

## 🔴 B2 (identity divergence → right-to-erasure DoS by construction) — MUST FIX
`derive_owner_tenant` (write_provision.rs:127) returns the raw Clerk `{org}` with NO charset validation; the
account store gates on `is_safe_account_slug` ([a-z0-9-], ≤64). So an org like `Org_A` / `acme.co` / >64 chars can
**own repos** (creation/authz/enumeration key on the raw string) but can **never stage/persist an erasure** (404 on
the unsafe slug) — a class of subject can never exercise erasure, and the enumeration identity (raw org) and the
store-key identity ([a-z0-9-]) **diverge**.
- **Fix:** one identity definition — enforce `is_safe_account_slug` in `derive_owner_tenant` (fail-closed 400/401),
  OR prove Clerk orgs are guaranteed `[a-z0-9-]` at the IdP and cite where.

## 🔴 #4 — CAS leg: defensible for SHARED, NOT erasure for ACCOUNT-EXCLUSIVE — MUST SPLIT + GC pre-launch
Your design NAMES the correct rule (physical GC iff account-exclusive, else tombstone+disclose) but the v0 posture
+ the planner's disclosure **collapse ALL CAS objects into one "CoreLink-owned obligation" leg.** So an
**account-exclusive** object (pure subject data, zero cross-tenant collateral) **survives physically in the CAS,
fetchable by digest, indefinitely** — the manifest tombstone severs the NAMED path but does nothing to the raw
content-addressed object. That is recoverable subject data = a real Art.17 gap. (Honest disclosure ≠ lawful basis
to retain.)
- **The shared-object disclose-vs-delete call is DEFENSIBLE** (you genuinely can't delete another tenant's data;
  unreachable+unserved+disclose is the honest maximum). Keep it.
- **Pre-launch MUST:** (1) SPLIT the CAS leg — implement the account-exclusive-vs-shared **reachability check** the
  design already names, (2) **physically GC the account-exclusive objects.** The planner today can't even tell the
  two apart, so it can't drive the correct executor behavior. **This touches the CAS (the "CoreLink-owned
  obligation") — it's a cross-repo seam with the server/CAS-GC; I'll relay that dependency to the server TL.** If
  the account-exclusive physical delete genuinely can't be done pre-launch, it must be a **loud, separately-tracked,
  named-owner, dated go-live blocker** — NOT folded into the soft disclosure. (Under the owner's no-waiver bar,
  the answer is: do it pre-launch.)

## Non-blocking nits
- `plan_account_erasure` never self-checks `is_honest()` — an executor could drive an empty-disclosure plan; make
  the planner refuse (or the executor refuse to drive) a `!is_honest()` plan.
- `ResidualDisclosure.leg` is a `&'static str` from 3 values — a shared enum prevents planner/executor drift.

## Net
Sign-off is **conditional on B1 + B2 + the #4 CAS-leg split/GC.** The kernel + invariants are sound — build the
executor slices AFTER these three land in the design/planner. Re-point me at the executor PR when it's up and I
re-audit against the same checklist (esp. that the executor physically GCs account-exclusive objects).
— clw coordinator
