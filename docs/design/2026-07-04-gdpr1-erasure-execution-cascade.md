# GDPR1 Part 2 — the erasure EXECUTION cascade (the irreversible kernel)

> **Status:** DECIDED design. Part 1 (staging) is live on `main` (#252 contract + verb, #253
> seam + door + route). This doc is the decided plan for Part 2 — the `erasure.requested →
> erasure.executed` cascade that GENUINELY, VERIFIABLY, IRREVERSIBLY tombstones the subject.
> **Owner mandate:** hard go-live gate, no waiver, full execution.
> **Rigor:** this is the product's ONLY irreversible-deletion path. It ships **only after clw's
> independent cold adversarial audit** (they committed to it). Build order: DESIGN (this doc) →
> read-only PLANNER (safe) → [clw audits] → irreversible EXECUTOR (gated) → live-verify.

## The law we MUST honor (from X7/X12, already proven hermetically)

`crates/hugit-invariants/{x7/cascade.rs,x12/erasure.rs}` prove the erasure LAW. The production
executor is informed BY that law; it does **not** edit those hermetic invariant crates (they
re-model the stores in-memory on purpose, to keep their `x7/**`/`x12/**`-only ownership). The
binding invariants the executor inherits:

1. **Erasure operates on the OBJECT/content store, NEVER on the append-only provenance chain.**
   A repo's `EventLog` is never rewritten. The surviving links keep referencing the SAME content
   hash, which now resolves to a tamper-evident **Tombstone** bearing that exact hash. So
   `verify_chain` still verifies byte-for-byte AFTER erasure. A *silent re-link* (re-pointing a
   record at a substitute) mutates the payload and is caught fail-closed by `verify_chain`.
2. **No orphans:** every surviving provenance link resolves to a live object OR a tombstone —
   never a void. An erased object MUST leave a tombstone.
3. **Attestation re-seal OR fail-closed:** any attestation over a changed manifest is re-signed,
   or it fails closed under `verify_attestation` — never a silently broken seal.
4. **Mirror obligation = discharged OR an explicit, non-empty residual-risk disclosure.**

## The decisions

### D1 — the lifecycle (`requested → executed`, NOT instantaneous)

Execution is a SEPARATE authorized step from the request, deliberately:
- **Cancellation window (fail-safe against an irreversible mistake / a compromised session).** A
  single leaked session must NOT be able to instantly nuke an account. GDPR permits the controller
  up to 30 days; the grace window is a product/legal knob (`ERASURE_GRACE_SECS`, default TBD by the
  owner — a placeholder constant, not hardcoded policy).
- **Who triggers execute (v0):** the OPERATOR, via a dedicated authenticated, step-up-gated
  execution entry, AFTER the grace window has elapsed for a `requested` account with no intervening
  `erasure.cancelled`. (Rationale: v0 has no scheduler; an operator-run execution over the
  standing request is auditable + reversible-until-run. A self-serve auto-execute-after-grace is a
  P2 scheduler concern.) **This is the ONE place the operator IS permitted to act on another
  account — and it is gated: it can ONLY execute a request the SUBJECT itself staged (Part 1's
  `derive_owner_tenant` guarantees the `erasure.requested` subject == the authenticated requester),
  never mint one.** The operator executes a standing lawful request; it cannot originate an erasure.
- **A `requested` with no matching lawful request never executes** (the executor reads the account
  log; absent a `requested` in state, it is a 404/no-op — never a god-erase).

### D2 — WHAT gets erased, per store (the five legs, grounded in the real engine)

The subject account owns a set of resources. The executor enumerates + tombstones each:

1. **The account's repos (CAS/R2 object + manifest store).** Enumerate every repo whose genesis
   `repo.meta{owner_tenant}` == the subject (the SAME projection `count_owned_repos`/`me_repo_logs`
   use — `crate::authz::project_repo_meta`). For each owned repo:
   - Append a **terminal `repo.erased` tombstone record** to the repo's append-only `EventLog`
     (NEVER rewrite it) — carrying the erasure reason (the account slug + request id). The read
     path/authz gate treats a `repo.erased`-terminal repo as tombstoned: 404 to everyone, no
     content served, no clone. (New terminal state, additive to the meta projection.)
   - The repo's `refs.json`/`oid-index.json` manifests are overwritten with an **empty/tombstone
     manifest** (conditional `If-Match` PUT) so the git wire serves nothing.
   - The **CAS objects themselves** are content-addressed and **cross-tenant deduplicated** (the
     CoreLink CAS is shared — see the cross-tenant-dedup cautionary tale). An object referenced by
     ANY other tenant CANNOT be physically deleted without erasing another tenant's data. So per
     object: physical GC iff **account-exclusive** (referenced by no surviving manifest of any other
     tenant — a reachability check the CAS owner exposes, or defer to CoreLink's GC), else a
     **tombstone + residual-risk disclosure**. **v0 honest posture: tombstone at the manifest/repo
     level (content is unreachable + unserved) + a residual-risk disclosure for the shared-object
     CAS leg; physical CAS GC is a CoreLink-owned obligation (cross-repo interop seam).** This is
     the X7/X12 mirror-leg discipline applied to the dedup CAS — the honest disclosure IS the
     deliverable where physical delete is not unilaterally provable.
2. **The account log (`_accounts/{slug}`).** Append `erasure.executed` (below) — the log itself is
   the audit record of the erasure and is NOT purged (it carries no personal data beyond the slug).
3. **Context store.** Purge account-scoped context bytes (genuine removal) where an engine
   context-store erasure API exists; else a tracked residual-risk disclosure (the X3③/X7-leg-3 seam).
4. **GitHub mirror.** A `MirrorObligation` → discharged (if a live mirror-erase is wired) OR an
   explicit residual-risk disclosure (the documented P2 seam — identical to X12②).
5. **Experiment corpus / provenance.** Any sealed corpus datapoint bearing the subject's data →
   erased with a recorded `GateInvalidation` (X7 item ④); provenance chains stay verifiable
   (tombstone-by-hash, append-only).

### D3 — durability ordering (fail-closed `executed ⇒ tombstoned`, the receive-pack discipline)

The `erasure.executed` record is the CLAIM that the cascade completed. It is appended **LAST,
only after every durable tombstone/purge above has landed** (each a fail-honest `Result`). Mirror
the receive-pack `ok ⇒ durable` law: if ANY leg fails to durably tombstone, the executor returns
5xx and does NOT append `erasure.executed` — the account stays `requested`, re-runnable. There is
never an `executed`-without-tombstone. The `erasure.executed` payload records, per leg, the
outcome (tombstoned / purged / residual-risk-disclosed) so the claim is auditable + honest.

### D4 — idempotent + irreversibility-guarded

- **Idempotent replay:** an account already in `executed` state → the executor is a NO-OP (never a
  double-tombstone, never resurrects). Re-tombstoning an already-tombstoned repo is itself a no-op
  (tombstone is terminal), so a partial-then-retry converges.
- **Irreversibility guard:** there is no un-tombstone. A `repo.erased`-terminal repo can never
  return to a served state (the projection is terminal + append-only). `erasure.executed` is
  terminal on the account log.
- **Partial-progress safe:** because each leg is append-only-tombstone (idempotent) and `executed`
  is gated on ALL legs, a crash mid-cascade leaves a re-runnable `requested` with some legs already
  tombstoned (converges on retry) — never a half-`executed` claim.

### D5 — what this does NOT do in v0 (honest scope, tracked)

- No scheduler (operator triggers execute post-grace; auto-execute is P2).
- Physical cross-tenant-dedup CAS GC is a CoreLink-owned seam (residual-risk disclosed until wired).
- Live GitHub mirror-erase is the documented P2 seam (residual-risk disclosed).
- Context-store physical purge rides whatever engine context API exists (residual-risk if none).

## The adversarial-audit checklist (clw runs this INDEPENDENTLY before live enablement)

1. **No god-erase / no anon-erase at execute:** the operator can only EXECUTE a request the subject
   itself staged (subject == the `erasure.requested` author, Part-1-guaranteed); it can never mint
   one. Anon/non-operator cannot execute.
2. **`executed` ⇒ every leg durably tombstoned/purged/honestly-disclosed** — no `executed`-without-
   deletion on any leg (fail-closed durability ordering, D3).
3. **Provenance never rewritten:** `verify_chain` passes on every affected repo log AFTER the
   cascade; every surviving link resolves to a tombstone, zero orphans (X7 item ②).
4. **Idempotent replay = no-op; irreversibility holds** (no un-tombstone, no resurrection).
5. **Partial-progress converges** (crash mid-cascade → re-runnable `requested`, never half-executed).
6. **The tombstoned repo is genuinely unreadable/unclonable** across BOTH the `/v1` read path AND
   the git wire (read-authz re-decides from the terminal `repo.erased` state → 404, no oracle).
7. **Cross-tenant safety:** a shared (deduped) CAS object referenced by another tenant is NEVER
   physically deleted — only manifest-tombstoned + residual-risk disclosed (no collateral erasure).
8. **The residual-risk disclosures are non-empty + honest** (an empty disclosure fails closed — the
   X7/X12 `is_honestly_resolved` predicate).
9. **Grace/cancellation window respected:** execution before grace elapses, or over a `cancelled`
   request, is refused.
10. **Secret-scrub at every emitted record boundary** (the account slug / reason are structural, but
    the guard runs).

## Audit response — clw FIX-FIRST verdict (2026-07-04), all 3 must-fixes landed in the planner

clw ran the independent cold audit against #254 (design + planner) + #253 staging, verdict
**FIX-FIRST** (kernel sound; completeness + CAS legs needed fixing). All three land in the
design/planner BEFORE the executor, as required:

- **B1 (under-erasure — the worst class):** the planner enumerated only the in-memory loaded set,
  so a durable-but-unloaded repo (provisioned then dropped from the boot env) could survive while
  the subject was told "erased." **Fixed:** `AppState::authoritative_owned_repo_logs` now anchors on
  the **DURABLE** store listing (`LogSource::list_repo_slugs` → R2 ListObjectsV2 / Local dir) ∪ the
  in-memory set, fail-closed (503) on any listing/load fault. A durable-but-unloaded repo is now
  never missed (regression-tested).
- **B2 (identity divergence → right-to-erasure DoS):** `derive_owner_tenant` returned the raw Clerk
  org with no charset validation, while the account store keys on `is_safe_account_slug` — so an org
  like `Org_A` could OWN repos but never ERASE (ownable-but-unerasable). **Fixed:** `derive_owner_tenant`
  now enforces `is_safe_account_slug` (400) — ONE identity: ownership and erasability share a single
  traversal-safe slug by construction (a non-conforming org owns nothing, the safe direction).
- **#4 (CAS leg — account-exclusive is not erasure):** the v0 posture collapsed all CAS objects into
  one disclosure, so an **account-exclusive** object (pure subject data) survived physically. **Fixed
  (planner model):** the CAS leg is SPLIT — `DisclosureLeg::CasShared` (shared objects, defensible
  disclose) vs `CasGcObligation` (account-exclusive, physical GC REQUIRED). With the CAS-GC seam not
  wired in v0, `ErasurePlan::is_launch_blocked()` fires LOUDLY — a separately-tracked go-live blocker,
  never a soft disclosure. **The physical GC of account-exclusive objects is a CoreLink server/CAS-TL
  cross-repo seam (relayed by clw) and a HARD pre-launch blocker under the no-waiver bar.**
- **Nits:** the planner self-refuses a `!is_honest()` plan; `ResidualDisclosure.leg` is a closed
  `DisclosureLeg` enum (no planner/executor drift).

## Build order (each slice its own PR, gate-green)

1. ✅ **Part 1 — staging** (`erasure.requested`, the seam + door + route). #252 + #253, on `main`.
2. **The read-only PLANNER** — `plan_account_erasure(state, account) -> ErasurePlan`: enumerate the
   owned repos + the per-leg tombstone/purge/disclosure PLAN, mutating NOTHING. Fully hermetic +
   testable; zero risk. This is the "what" the executor drives. (Next slice.)
3. **[clw independent cold adversarial audit of THIS design + the planner]** — the 2nd gate. Point
   clw at the branch/PR before any executor code enables live.
4. **The EXECUTOR** — drive the plan against the real stores with the D3 fail-closed ordering + D4
   guards + the terminal `repo.erased` projection + the `erasure.executed` claim. Gated behind the
   audit; NOT route-enabled live until clw signs off + the owner enables.
5. **Route + grace gate** — the operator execute entry (step-up), post-grace, over a standing request.
6. **Live-verify** — a real Clerk tenant token stages a `requested`; operator executes post-grace;
   re-read proves the repo is 404/unclonable + the chain still verifies + the disclosures are honest.
