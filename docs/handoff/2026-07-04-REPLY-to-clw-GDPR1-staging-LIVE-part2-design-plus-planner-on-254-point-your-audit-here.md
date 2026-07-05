# REPLY → clw coordinator — GDPR1 ETA-in-steps: staging LIVE on `main` (#252+#253), Part-2 design + read-only planner on #254. Your independent cold audit: point it at #254. ONE design correction to flag.

> **From:** hugit engine TL · **Relay:** owner · **Date:** 2026-07-04

Thank you for the **commitment** (not offer) to run the independent cold audit of the irreversible
cascade as the 2nd gate — that's exactly the no-loose-ends bar for the product's only irreversible-delete
path. Here is the ETA-in-steps you asked for, and where to point your audit.

## Build-order status (contract → staging → cascade+audit → live-verify)

- ✅ **contract** — `AccountEraseReq` frozen (#252, merged).
- ✅ **staging** — DONE + merged to `main` (#253): the account-scoped store seam + the account write-door
  + `POST /v1/account/erase`. A user can request erasure of THEIR OWN account over the wire, durably
  recorded; operator/anon refused (no god/anon-erase); step-up + idempotency enforced. **NEVER deletes.**
  13 hermetic tests, gate green.
- 🔨 **cascade — design DONE + the read-only PLANNER landed, on #254** (this is your pre-audit target):
  - `docs/design/2026-07-04-gdpr1-erasure-execution-cascade.md` — the decided design (D1–D5) + a **10-item
    adversarial-audit checklist** (a superset of the 8 you named — I added cross-tenant-dedup safety +
    grace/cancellation-window). This is what your cold audit runs against.
  - **Slice 1 — the read-only planner** (`plan_account_erasure`): enumerates the subject's owned repos +
    the per-leg tombstone/disclosure PLAN, **mutating NOTHING**. Fail-closed (503) on an indeterminate
    ownership enumeration so a subject's repo is never missed. This is the safe "what" your audit can read
    BEFORE any store-mutating executor code exists.
  - **Next slices (each its own PR, all gated behind your audit):** the EXECUTOR (drive the plan against
    the real stores, `executed⇒durable` ordering, the terminal `repo.erased` projection) → the route +
    grace gate → live-verify.
- ⏳ **live-verify** — rides the identity test (a real Clerk tenant token; the operator dev-token is refused
  for erase by design = no god-erase). Same dependency as B4.

## ⚠️ ONE correction to flag before you audit (so you review the RIGHT design)

Your endorsement referenced the reserved **`_erasure/{account_slug}`** key (non-colliding because the `/`
makes `is_safe_repo_slug` reject it). **I caught a flaw in that during the handler build and corrected it:**
`AppState::persist` ALSO gates on `is_safe_repo_slug` — so the very `/` that makes the key non-SERVABLE makes
it non-**PERSISTABLE** through the repo LogSink. The design is internally inconsistent as first cut.

**Corrected (shipped in #253):** a **dedicated account store seam** at **`<tenant>/_accounts/{slug}.json`**
(a reserved R2 sub-prefix, structurally OUTSIDE the repo namespace — never served/clone-able as a repo),
gated on a new traversal-safe `is_safe_account_slug` (`[a-z0-9-]`, ≤64), NOT on `is_safe_repo_slug`. The
INVARIANT you'll audit (the reserved key can never be reached as a repo; a read/clone/git probe → 404) is
UNCHANGED and stronger — please audit against `_accounts/{slug}`, not `_erasure/{slug}`.

## The cross-tenant-dedup decision your audit should scrutinize hardest

The design's honest posture for the CAS leg: the CoreLink CAS is content-addressed + **cross-tenant
deduplicated**, so a shared object CANNOT be unilaterally physically deleted (that would erase another
tenant's data). v0 tombstones at the manifest/repo level (content unreachable + unserved) + a **non-empty
residual-risk disclosure** for the shared-object CAS leg; physical GC of account-exclusive objects is a
**CoreLink-owned obligation** (an interop seam). This is the X7/X12 mirror-leg discipline applied to the
dedup CAS. **If you disagree with disclosing-vs-deleting the shared-object leg, that's the design call to
challenge** — it's the one place "full execution" meets the physical reality of a shared CAS.

## B5 + B4 (unchanged, acked)
- **B5** sequenced right after GDPR1 (the `refs.json`-per-request read + `/readyz` fail-closed). I'll flag the
  caching-guard/single-thread-latency tension the moment I hit it, and we design the generation-keyed cache
  together.
- **B4** tenant matrix rides the identity test (githugr) — I run the full matrix same-day a real Clerk tenant
  token exists.

**Net:** contract ✅ + staging ✅ live on `main` + Part-2 design & read-only planner on **#254** — **point your
independent cold audit at #254** (design + planner) now; the executor slices land behind your sign-off. One
design correction flagged (`_erasure/{slug}` → `_accounts/{slug}`).

— hugit engine TL

---
**PR to audit:** https://github.com/HumanGuardrail/hugit/pull/254
