# WP-D14 — forge authz
squad D · M · opus · ctx 60k · branch: wp/D14

## Charter
Build forge authorization: every mutating endpoint (push / land / undo / policy)
rejects unauthorized principals. The permission model is documented and
golden-tested per principal class (human / orchestrator / worker agent / model),
and every authz denial is audited. The mutation-surface guard for the forge.

## Owned acceptance (VERBATIM — decomposition v2.0 D14①–③)
① mutating endpoints (push/land/undo/policy) reject unauthorized principals
② permission model documented + golden-tested per principal class
③ authz denials audited

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` — authz denials are emitted as audited events (③); the mutation endpoints append events.
- `QueueApi` — the `land` mutation surface guarded (consumed; D14 guards entry, does not modify queue internals).
- Principal-identity shape (whitepaper §3: human / orchestrator / worker / model as first-class identities) — the per-class permission model (②).
- `RegenGate` / policy descriptor — the `policy` mutation surface guarded.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-refstore/authz/` (the authorization guard over the mutating forge
  endpoints push/land/undo/policy, the documented per-principal-class permission
  model, the denial audit emitter).
- `crates/hugit-refstore/authz/tests/` (golden per-principal-class fixtures).
Writes outside `hugit-refstore/authz/` = leak. (D1 owns the rest of
`hugit-refstore/` [event log, refs, undo mechanics]; D4 owns
`hugit-refstore/intent/`; D14 is a disjoint guard sub-module — it AUTHORIZES the
mutations those modules implement, modifying neither.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D14 row) + §6 (D14⇠D1) +
  §7 (D1→D14) + §8; `docs/whitepaper/hugit-v1.md` §3 (every principal a first-class
  identity with permissions/budgets/attribution) + §6.5 (event log + undo) + §9
  (security locks); `docs/product/command-catalog.md` (per-principal command
  surface — the principal classes); frozen `EventRecord`/`QueueApi` + principal shape.
- Anchors: the FOUR mutating endpoints are push/land/undo/policy; the FOUR principal
  classes are human/orchestrator/worker/model. Golden fixtures cover each
  (principal class × endpoint) cell — authorized lands, unauthorized rejected.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance suite
  committed BEFORE implementation; opus route (security); SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **Mutation guard, fail-CLOSED (①):** every mutating endpoint — push, land, undo,
  policy — REJECTS unauthorized principals. The guard is fail-closed: an
  unrecognized/unauthorized principal is denied, never defaulted-allow. Assert each
  endpoint rejects an unauthorized principal.
- **Documented per-class model, golden-tested (②):** the permission model is
  WRITTEN DOWN per principal class (human / orchestrator / worker agent / model)
  and golden-tested — a fixture per (principal class × endpoint) asserts the exact
  authorized/denied outcome against the documented matrix.
- **Denials audited (③):** every authz denial emits an `EventRecord` (which
  principal, which endpoint, when) — assert the denial event is appended and
  attributable. Denials are never silent.
- **Guards, never owns the endpoints:** D14 AUTHORIZES the mutations D1/D3/D4/D6
  implement; it modifies none of their internals — it sits at the entry seam.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All three owned items green · zero writes outside `crates/hugit-refstore/authz/` ·
evidence bundle (per-endpoint unauthorized-reject proof, per-principal-class golden
matrix, denial audit events) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–③: green/red) · evidence refs (test ids + fixture paths +
golden permission matrix) · claims-respected: yes · deviations: none | waiver-ref.
