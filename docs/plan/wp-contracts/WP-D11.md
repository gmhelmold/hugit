# WP-D11 — journals + resume
squad D · M · sonnet · ctx 60k · branch: wp/D11

## Charter
Build session journals as tenant-private first-class objects bound to a
workspace/intent, and `ctx resume` for the crashed/replaced-agent case within a
supported horizon. Beyond the horizon, resume is refused/degraded as documented —
no false reconstruction. The honest core of "gitted context": provenance +
journals + short-horizon resume, never bitwise replay.

## Owned acceptance (VERBATIM — decomposition v2.0 D11①–③)
① journal persisted as tenant-private object bound to ws/intent
② post-crash `ctx resume` reconstructs session within supported horizon
③ beyond-horizon resume refused/degraded as documented

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` — journals + resume events are attributable on the stream.
- Context Snapshot / journal_ref (whitepaper §4.1 Context object) — the journal binds to ws/intent via this shape.
- `IntentSidecar` / native intent id — the intent a journal is bound to (②).
- Redaction policy descriptor — journals are tenant-private + policy-redactable (consumed read-only; X3 owns the cross-tenant law).

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-ledger/journal/` (journal object persistence, ws/intent binding,
  `ctx resume` reconstruction, horizon enforcement, beyond-horizon refusal/degrade).
- `crates/hugit-ledger/journal/tests/`.
Writes outside `hugit-ledger/journal/` = leak. (D5 owns the rest of
`hugit-ledger/`; D11 is a disjoint sub-module under the same crate.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D11 row) + §8;
  `docs/whitepaper/hugit-v1.md` §4 (Context Snapshot / Trajectory) + §13 risk 2
  (context capture sensitive; tenant-private, redaction, retention); §2 Inversion 2
  honest form; `docs/product/command-catalog.md` (Journals + short-horizon resume,
  HARDENED narrowed; `ctx resume` / `journal note` rows); frozen `EventRecord`/
  context-snapshot shapes.
- Anchors: journal = tenant-private object BOUND to ws/intent; resume has a
  SUPPORTED HORIZON; beyond it is a DOCUMENTED refusal. Fixtures: a crash within
  horizon + a beyond-horizon attempt.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance suite
  committed BEFORE implementation; SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **Tenant-private, bound (①):** the journal is persisted as a tenant-private
  first-class object BOUND to a specific ws/intent; assert the binding and the
  tenant-scoping (cross-tenant fetch is X3's law — D11 honors the scope, does not
  re-implement isolation).
- **Resume within horizon (②):** after a crash, `ctx resume` reconstructs the
  session within the SUPPORTED horizon (minutes-to-days, per catalog). Assert a
  within-horizon crash reconstructs.
- **Beyond-horizon = documented refusal (③):** beyond the supported horizon,
  resume is REFUSED or DEGRADED exactly as DOCUMENTED — it never silently fabricates
  a stale reconstruction. The horizon and the degrade behavior are written down;
  assert the refusal/degrade matches the doc.
- **No replay guarantee:** journals are best-effort reconstruction, NOT bitwise
  context replay (replay was CUT per catalog v2) — the contract makes no
  determinism claim.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All three owned items green · zero writes outside `crates/hugit-ledger/journal/` ·
evidence bundle (tenant-private binding assertion, within-horizon resume
reconstruction, beyond-horizon documented-refusal match) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–③: green/red) · evidence refs (test ids + fixture paths +
horizon doc ref) · claims-respected: yes · deviations: none | waiver-ref.
