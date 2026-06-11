# Pending-seams register — hugit

> 2026-06-11. Tracked-deferral register. Each entry below is a seam that is
> intentionally deferred with a findable owner, acceptance criteria, and the
> governing document. This file kills "vapor-deferred" (P-DEFERRALS from the
> 2026-06-11 adversarial Round 1 review). Update this file when a seam ships
> or is superseded; never silently drop an entry.

---

## PS-1 — Recorder verbs: `hugit check` / `hugit verdict` / `pr.landed` producer

**Status:** DEFERRED — P2/wave-2+ work  
**Adversarial finding:** P-WEDGE-HOLLOW (Tier 3, `docs/review/2026-06-11-adversarial-round-1.md`)  
**Governing docs:** `docs/review/2026-06-11-adversarial-round-1.md` §P-WEDGE-HOLLOW;
SOTA audit `docs/review/2026-06-11-sota-audit.md` §P1;
`docs/plan/wp-contracts/WP-B2a.md` (executor WP);
`docs/plan/wp-contracts/WP-B2b.md` (runner-side)

**What is deferred:**
- `hugit check` is reserved/undispatched (`HUGIT_RESERVED_VERBS`). No production
  caller appends `check.recorded` events to the event log. As a consequence:
  `hugit checks show` reads+aggregates `check.recorded` events from the log, but
  returns all-null KPIs on every real agent log because no producer exists yet.
- `hugit verdict` is similarly reserved; no producer appends `verdict.recorded`.
- `pr.landed` producer: the landing queue appends `intent.landed` correctly, but
  the downstream `pr.landed` event that feeds the queue-wedge KPIs is not yet
  emitted.

**Impact:** `hugit checks show` and `hugit queue show` return structurally correct
but operationally empty views on every real fleet log. The memoization wedge (the
primary value proposition) is invisible through the porcelain in practice.

**Owner:** hugit techlead  
**Acceptance criteria:**
1. `hugit check [--local]` is dispatched end-to-end (not reserved); execution
   appends a `check.recorded` event carrying `memo_key`, `hit/miss`, `duration_ms`.
2. `hugit checks show` returns non-null hit-rate, last-miss-key, and duration
   aggregates on any log that ran at least one check.
3. `hugit verdict` is dispatched; appends `verdict.recorded`.
4. `pr.landed` event emitted by the landing queue on a successful land.
5. The `hugit checks show` oracle is upgraded from all-null negative-control to
   a real aggregation test against a log with seeded `check.recorded` events.

**Unblocked by:** P2 CoreLink tenant is NOT required for this seam — the recorder
verbs can run against the in-process `EventLog` + `InMemoryAc` today.

---

## PS-2 — `--author-kind` authn binding (P2/identity)

**Status:** DEFERRED — requires ADR-0002 identity/P2  
**Adversarial finding:** P-DEFERRALS (Tier 4, `docs/review/2026-06-11-adversarial-round-1.md`);
SOTA audit S3 (`docs/review/2026-06-11-sota-audit.md`)  
**Governing docs:** `docs/adr/0002-hugr-identity.md`;
SOTA audit §S3;
`docs/handoff/2026-06-09-hugr-identity-rollout.md`

**What is deferred:**
- The D14 authz guard (`hugit_refstore::authz::append_authorized` /
  `AuditedGuard`) is golden-tested and wired to the CLI mutation path (Wave A),
  but `--author-kind` is caller-supplied: a subagent can claim `orchestrator` at
  the CLI without any cryptographic check. The guard enforces authorship rules
  correctly given the claimed kind, but the claimed kind is unauthenticated.
- Full fix: author-kind must bind to the authenticated principal from the HuGR
  Clerk-backed identity session (ADR-0002); the forge reads the session token,
  derives `author_kind` from it, and the CLI flag becomes advisory/override for
  the authenticated case only.

**Owner:** hugit techlead (identity rollout assigned to owner milestone P2)  
**Acceptance criteria:**
1. `--author-kind` is verified against the authenticated principal's session
   claims (Clerk / PAT scope); a subagent cannot escalate to `orchestrator` by
   flag alone.
2. The D14 guard oracle is extended with a mutation-verified test: a subagent
   claiming `orchestrator` is denied unless the session token grants it.
3. The CHANGELOG entry for WA2 is amended to reflect the interim limitation
   (already disclosed in the CHANGELOG: "authn binding honestly disclosed as
   the identity/P2 seam").

**Unblocked by:** ADR-0002 identity rollout (`docs/handoff/2026-06-09-hugr-identity-rollout.md`),
which requires P2 CoreLink tenant provisioning.

---

## PS-3 — Refstore cold-tier event-payload erasure (the deferred half of WA3)

**Status:** DEFERRED — production cold-store trait gap  
**Adversarial finding:** SOTA audit S4 (`docs/review/2026-06-11-sota-audit.md`);
implicitly referenced in CHANGELOG WA3 entry  
**Governing docs:** SOTA audit §S4;
`docs/adr/` (ADR-0001 trajectory-tier decision, cold-store decision);
`crates/hugit-invariants/x7/` (X7 acceptance suite);
`crates/hugit-invariants/x12/` (X12 acceptance suite)

**What is deferred:**
- WA3 shipped tombstone erasure on the `hugit_refstore::ColdStore` trait:
  `GetOutcome::{Present,Erased,Absent}` + resurrection-refused. The X7 and
  X12 invariant suites prove the erasure cascade against this trait.
- However, the REAL production cold-store (CloudFlare R2 via the CoreLink CAS
  surface) does not yet implement the `erase` method: `cold_store.rs` (the
  live seam adapter) lacks an erase path. X7/X12 proofs run against the
  in-process `InMemoryObjectStore` toy, not the production trait.
- The event-payload erasure half of the cascade (CAS tombstone → event-log
  event-payload purge on the real R2/D1 backend) is unimplemented.

**Owner:** hugit techlead (requires CoreLink P2 tenant + R2/D1 erase API)  
**Acceptance criteria:**
1. The production `ColdStore` adapter (over CoreLink CAS/R2) implements `erase`:
   issues a real tombstone-write to R2 keyed by content hash.
2. X7 item ① and X12 item ① acceptance suites are re-targeted to run against
   the REAL adapter (not `InMemoryObjectStore`) when `HUGIT_CORELINK_AC_URL` +
   tenant are set (run-not-skip).
3. The cold-store erase path is proven load-bearing: removing the erase call in
   the production adapter turns X7① RED.

**Unblocked by:** P2 CoreLink tenant provisioning (R2 + D1 access);
`docs/handoff/2026-06-08-corelink-p2-tenant-request.md`.

---

## Closed seams (reference — do not re-open without owner approval)

| Seam | Shipped | Governing commit |
|---|---|---|
| `cost_usd f64 → cost_usd_micros u64` (WA4, contract 1.2.0) | 2026-06-11 | CHANGELOG WA4 entry; corelink-runners contract §12 amendment |
| D14 authz guard wired to CLI mutation path (interim) | 2026-06-11 (WA2) | CHANGELOG WA2 entry |
| WA3 tombstone erasure on `ColdStore` trait (in-process) | 2026-06-11 | CHANGELOG WA3 entry |
| `hugit checks show` / `hugit queue show` CLI verbs (WB2) | 2026-06-11 | CHANGELOG WB2 entry |
| `hugit campaign` / `hugit intent` / `hugit pr` porcelain | 2026-06-10 | CHANGELOG CLI porcelain entry |

*Change protocol: amend this file with a `docs(truth):` commit whenever a seam ships or a new deferral is introduced. Never silently drop a pending seam — move it to the Closed table.*
