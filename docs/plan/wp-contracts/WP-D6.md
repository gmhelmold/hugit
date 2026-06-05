# WP-D6 — policy engine
squad D · M · sonnet · ctx 60k · branch: wp/D6

## Charter
Build the declarative policy engine v0: gates expressed as config, locally
testable, identically enforced at the forge, fail-CLOSED. The house's own gate
museum (DCO / changelog / secrets) is ported as test case #1. Every policy
change is an audited event. This is the control layer other WPs bind against.

## Owned acceptance (VERBATIM — decomposition v2.0 D6①–③)
① 3 ported gates local≡forge
② engine down→landing blocks (kill-test)
③ policy change = audited event

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` — policy changes are emitted as audited events (③).
- `QueueApi` — the landing path the engine gates (consumed; engine returns a
  gate verdict, the queue blocks on it — D6 does not modify the queue).
- Policy config schema (declarative gate descriptor) — defined/owned here under
  Claims; only the EventRecord/QueueApi seams are frozen contracts.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-policy/` (the entire crate: declarative gate evaluator, local
  test harness, fail-closed enforcement adapter, audit-event emitter, the 3
  ported gate configs).
- `crates/hugit-policy/tests/`.
Writes to refstore/queue/ledger/contracts = leak. (D12 regen-gate and D14 authz
consume this crate but own their OWN paths — see those contracts.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D6 row) + §8 (governance
  laws, routing); `docs/whitepaper/hugit-v1.md` §9 lock 4 (policy gates fail
  closed) + §6.4 (landing); `docs/product/command-catalog.md` (`hugit policy
  edit/test` rows); frozen `EventRecord`/`QueueApi` from `crates/hugit-contracts/`;
  the house gate definitions (DCO, changelog, secrets) as porting source.
- Anchors: gates are pure functions over a tree/intent; local and forge call the
  SAME evaluator. Fixtures are the 3 ported gates with known pass/fail inputs.
- Conventions: workspace house stack (Rust); fmt+clippy+test+audit; failing
  acceptance suite committed BEFORE implementation; SEAL with evidence.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **local≡forge (①):** the policy evaluator is one pure function; `hugit policy
  test` locally and the forge enforcement call the identical codepath — assert
  byte-identical verdicts on the 3 ported gates (DCO, changelog, secrets) across
  both invocation sites.
- **Fail-CLOSED is a CONTROL, not advisory (②):** if the engine is down /
  unreachable / degraded, landing BLOCKS — it never falls open to "allow". This
  is a kill-test: kill the engine, assert the landing path refuses, no false
  green. (Symmetric to the fail-closed posture of D8⑥ and B9⑥.)
- **Audited change (③):** every policy edit emits an EventRecord (who, what,
  when, old→new) onto the stream; the change is never silent. Assert the event
  is appended and attributable.
- **No queue surgery:** D6 returns a gate verdict; the queue (B4/QueueApi)
  consumes it. D6 does not modify queue internals — the seam is the frozen API.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All three owned items green · zero writes outside `crates/hugit-policy/` ·
evidence bundle (local≡forge verdict diff on 3 gates, engine-down kill-test
transcript showing landing blocked, policy-change audit event) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–③: green/red) · evidence refs (test ids + fixture paths +
kill-test transcript) · claims-respected: yes · deviations: none | waiver-ref.
