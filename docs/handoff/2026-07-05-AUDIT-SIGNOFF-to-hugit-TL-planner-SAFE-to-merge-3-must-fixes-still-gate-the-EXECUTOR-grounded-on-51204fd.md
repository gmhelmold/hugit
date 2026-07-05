# AUDIT SIGN-OFF → hugit engine TL — the read-only PLANNER is SAFE (sign-off granted); the 3 must-fixes STILL gate the EXECUTOR. Grounded on merged `51204fd`, not the design prose.

> **From:** clw coordinator (independent cold audit) · **Relay:** owner · **Date:** 2026-07-05
> Re-ran the audit against the **actually-merged code** (`51204fd`, #254), not the design doc alone.
> This supersedes my pre-merge `2026-07-04-AUDIT-VERDICT` with file:line-grounded evidence.

## ✅ Slice 1 (the read-only PLANNER) — SIGNED OFF, safe to have merged
Verified against the merged source, not claims:
- **Mutates nothing.** `plan_account_erasure` (`writes/erasure.rs`) only reads `owned_repo_logs` +
  `repo_is_erased` (a `.records().any(kind == "repo.erased")` scan) + attaches static disclosures. No store write.
- **Honesty invariant real.** `ErasurePlan::is_honest()` rejects any empty residual disclosure; the three v0
  disclosures (`cas-dedup` / `github-mirror` / `context-store`) are non-empty by construction. Good — this is the
  X7/X12 `is_honestly_resolved` mirror, enforced in code.
- **Fail-closed on a load fault.** `owned_repo_logs` returns `None` (→ planner `503`) if ANY known candidate log
  won't `load_verified` — the planner never plans off a half-read set.
**Merging the read-only planner does NOT touch my FIX-FIRST verdict** — that verdict gates the **EXECUTOR**, and
the planner is `mutates-nothing`. Clean sequencing. This sign-off is the planner slice ONLY.

## ⛔ The 3 must-fixes STILL gate the EXECUTOR (grounded on the merged code)
"Planner merged" ≠ "audit cleared." The irreversible executor slice does NOT enable live until these close:

### B1 — under-erasure (CONFIRMED in `51204fd`, not theoretical)
`AppState::owned_repo_logs` (state.rs, the new +39) enumerates **`self.repos` (boot) ∪ `repos_runtime`
overlay ONLY** — its own doc comment: *"enumerates only the in-memory loaded set (never an R2 listing scan)."*
Its fail-closed path triggers only when a **known candidate** won't load; it is **silent** when a repo exists
durably in R2 but was **never loaded** into either map → it's simply absent from `names`, and the planner
returns `Ok(plan)` **missing it**. The executor drives this plan → the subject is told `executed` while a
durable owned repo survives readable. **This is the GDPR-fatal leg.**
→ **Fix before the executor:** anchor completeness on the **durable R2 owned set** (list-and-verify), OR compare
the in-memory owned count against a durable count and **fail-closed 503 on divergence** — never plan off a set
that can silently under-report. (The planner's fail-closed is correct in shape but scoped to the wrong risk.)

### #4 — account-EXCLUSIVE CAS objects survive physically (CONFIRMED)
`v0_residual_disclosures()` leg `cas-dedup` states, honestly: *"physical GC of account-exclusive objects is a
CoreLink-owned obligation."* The disclosure correctly **separates** shared-dedup objects (persist by design —
defensible, deleting them would erase another tenant) from **account-exclusive** ones. But under the owner's
no-waiver bar, an **account-exclusive** object still fetchable by digest after `executed` is a **real erasure
gap, not a disclosure-satisfiable one**. Disclosure is the right answer for the SHARED leg; it is NOT for the
EXCLUSIVE leg.
→ **Fix before the executor:** implement (or consume) the **reachability check** that classifies an object as
account-exclusive, and **physically GC** those. This is a cross-repo CAS-GC dependency on CoreLink — I have
already flagged it to the server TL (`2026-07-04-FLAG-to-server-TL-GDPR1-CAS-GC-account-exclusive-objects`).
The executor must not claim `executed` on the CAS leg for exclusive objects until GC is wired OR the object is
provably shared.

### B2 — erasure-DoS via identity divergence (Part-1 code, still open)
Not in this diff (it lives in `write_provision.rs::derive_owner_tenant`), but it gates the executor/route slice:
the enumeration identity (`repo.meta{owner_tenant}`, raw Clerk `{org}`) and the account store-key identity
(`is_safe_account_slug`: `[a-z0-9-]`, ≤64) can **diverge** — an org with uppercase/dots/>64 can OWN repos but
its account-erase can never key/store, so it can never erase. Reconcile the two identities (validate/normalize at
provision so the ownable identity == the erasable identity), or the right-to-erasure is unreachable for those
accounts.

## The gate, explicitly
- **Planner (#254):** ✅ signed off, correctly merged.
- **Executor slice:** ⛔ my sign-off is WITHHELD until B1 + #4 + B2 are closed. When the executor PR is up, point
  me at it — I re-audit the full 10-item checklist against the real executor code (same file:line rigor as this),
  with focus on B1 completeness + the #4 account-exclusive GC. Then the owner enables live.

The KERNEL remains sound (reserved `_accounts/{slug}` key, no god/anon/cross-tenant, `executed⇒durable` D3
ordering, D4 idempotency/irreversibility). The 3 fixes are surgical, not architectural.

— clw coordinator (independent cold audit)
