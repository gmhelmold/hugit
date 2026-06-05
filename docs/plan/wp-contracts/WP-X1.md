# WP-X1 — tenant isolation red-team
squad X · L→M · opus *(red-team)* · 90k · branch: wp/X1 · scheduled: **sprint 1 (before any external tenant)**

## Charter
Prove the CoreLink-inherited tenant boundary holds for hugit: private bytes
never cross tenants, memo-key forgery cannot poison, public-deterministic
sharing is real AND proven leak-free, and no side-channel (existence/timing)
betrays a private artifact. This is a red-team WP — the owned items ARE the
attack scripts. It must land before any external tenant exists.

## Owned acceptance
① tenant B requesting a key whose private bytes came from tenant A → miss/deny, never served
② forged/collision memo-key attempts → deny + alert, no poisoning
③ public-deterministic artifact IS shared, with proof no private bytes rode along
④(R6) side-channels: private artifacts produce NO cross-tenant hits (no existence/timing signal possible); the existence signal inherent to PUBLIC-deterministic sharing is documented as by-design disclosure

## Contract deps
- `CheckDef`, `CheckResult` (frozen — the memo-keyed objects whose `H(tree‖def‖toolchain)` keys the cross-tenant tests; consumed read-only).
- `AttestationChain {tree, def, runner, model, principal, sig}` (frozen — used to prove no private principal/runner identity rode along on a shared hit).
- Tenant boundary = **HMAC-derived prefixes** (CoreLink model, whitepaper §9): the keyspace partition this WP attacks; consumed as a surface, never modified.
- No frozen type is modified here.

## Claims
- `crates/hugit-invariants/x1/` (test crate + red-team fixtures for X1 only).
- No production crate paths. X-WPs write ONLY their own test crates + red-team fixtures; they CONSUME other squads' surfaces (B2 checks/AC client, C5 fence/broker), never modify them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X1 row) + §7 (DAG: X1⇠{B2,C5}) · `docs/whitepaper/hugit-v1.md` §9 (tenant boundary, the five locks) · `docs/product/command-catalog.md` (memoized-checks invariant) · the frozen `hugit-contracts` types above.
- Anchors: each owned item ①–④ is one adversarial test module under `crates/hugit-invariants/x1/`; the attack is the test body, red→green.
- Conventions: failing acceptance suite committed BEFORE any harness wiring; every deny/alert path asserts an audit event; fixtures name the producing/consuming tenant explicitly.

## Implementation notes (every fork PRE-DECIDED)
- **Tenant boundary under test = HMAC-derived prefixes** (CoreLink model). Item ① drives a tenant-B request for a key whose bytes are tenant-A-private and asserts miss/deny — never served — at the AC client surface (B2's surface, consumed read-only).
- **Item ② (forgery/collision):** craft a memo key colliding with a private entry and a forged key; assert deny + alert + zero poisoning of the AC. The poisoning check reads back the AC entry and asserts the private bytes are unchanged.
- **Item ③ (public sharing is real):** a public-deterministic artifact (e.g. a hermetic toolchain layer) IS served cross-tenant; prove via `AttestationChain` that the served object carries NO tenant-A principal/runner bytes — the shared object is the platform-anonymized form (the X2④ honesty property; X1 asserts the no-leak side).
- **Item ④ (side-channels):** assert private artifacts yield no cross-tenant existence or timing signal — same response shape/latency-class for "private miss" and "absent". The existence signal inherent to public-deterministic sharing is **documented as by-design disclosure**, not treated as a leak.
- This WP consumes B2's AC/memo surface and C5's fence/broker surface as-built; it modifies neither. Tenant fixtures derive prefixes via the HMAC model, never by ad-hoc string namespacing.
- **Dedicated red-team pass required** (routing & verification law §8): a non-author red-team agent attempts to break ①–④ beyond the committed scripts; the SEAL records the attempt log.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–④ red→green · cold-verify pass by a non-author · **plus the dedicated red-team pass** (X1 is a named red-team WP, §8) · zero writes outside claims · security review at the sprint-1 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x1/` · evidence bundle (attack scripts, audit-event assertions, attestation no-leak proof, red-team attempt log, by-design-disclosure note) attached to the sprint-1 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + red-team log ref), deviations = none | waiver-ref.
