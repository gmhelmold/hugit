# WP-D12 — regen gate
squad D · M · opus · ctx 70k · branch: wp/D12

## Charter
Build the regenerative-rebase gate: regen runs ONLY on opt-in scope; it lands
only if acceptance re-passes AND a fresh independent adversarial verdict approves;
missing/failing either → blocked + reported. Every regen is its own auditable
revision whose attestation RECORDS the authorizing gate-verdict ref (provenance
closure). Anti-smuggling: a file not provably derived cannot be classified derived
to bypass the gate.

## Owned acceptance (VERBATIM — decomposition v2.0 D12①–⑤)
① regen only on opt-in scope; non-opted repo never regens
② regen lands only if acceptance re-passes AND fresh independent adversarial verdict approves
③ missing/failing either → blocked + reported
④ **🔧 every regen auditable as its own revision, whose attestation RECORDS the authorizing gate-verdict ref (provenance closure: this regen was permitted because report-vX passed)**
⑤ **(R5) anti-smuggling: a file not provably derived (regeneration command must deterministically produce it from sources) CANNOT be classified derived — bypassing the regen gate via a false "derived" declaration is blocked + audited**

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `RegenGate {optin_scope, repass, indep_verdict}` — the gate's frozen shape; D12 IMPLEMENTS its mechanics (D8 BINDS its promotion).
- `VerdictObject` — the fresh independent adversarial verdict required to land (②); produced by D7 panels, consumed here.
- `AttestationChain {tree, def, runner, model, principal, sig}` — the regen revision's attestation records the gate-verdict ref (④).
- `EventRecord` — blocks, lands, and false-derived rejections are audited.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-checks/regen/gate/` (the regen GATE: opt-in scope enforcement,
  the re-pass + independent-verdict precondition, the derived-classification
  predicate + anti-smuggling check, the per-regen revision attestation with
  gate-verdict ref).
- `crates/hugit-checks/regen/gate/tests/`.
Writes outside `hugit-checks/regen/gate/` = leak. (C4 owns the regen DRIVERS
[`hugit-checks/regen` driver mechanics per warp]; D12 owns the GATE that authorizes
landing — coordinate via the shared crate's disjoint sub-paths; D12's claim is the
gate decision surface, not the driver execution. D7 owns the verdict panel; D8 owns
the experiment-gate binding — D12 CONSUMES both.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D12 row, C4 regen drivers
  row for the seam) + §7 (C4→D12) + §8; `docs/whitepaper/hugit-v1.md` §6.3
  (regenerative rebase) + §13 risk 1 (regen trust, acceptance re-pass + verdict +
  every regen auditable); `docs/product/command-catalog.md` (Regenerative rebase
  OPT-IN forever; Derived-file regeneration); frozen `RegenGate`/`VerdictObject`/
  `AttestationChain`/`EventRecord`.
- Anchors: the gate is fail-CLOSED — both acceptance re-pass AND independent verdict
  required. Fixtures: opt-in vs non-opted repo, a failing re-pass, a missing
  verdict, a false "derived" declaration.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance suite
  committed BEFORE implementation; opus route (security/control); SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **Opt-in scope only (①):** regen runs ONLY on the opt-in scope from `RegenGate.optin_scope`; a non-opted repo NEVER regens — assert zero regen on the non-opted fixture.
- **Both preconditions, fail-CLOSED (②③):** regen lands ONLY if acceptance
  re-passes AND a FRESH INDEPENDENT adversarial verdict (a real D7 panel —
  distinct prompts + ≥2 models, served ground truth) APPROVES. Missing OR failing
  EITHER → BLOCKED + reported. No distance-threshold hand-wave; the independent
  verdict is mandatory per regen (circular-verification risk is structural).
- **Provenance closure (④):** every regen is its OWN auditable revision; its
  `AttestationChain` RECORDS the authorizing gate-verdict ref ("this regen was
  permitted because report-vX passed"). Assert the ref is present and resolves.
- **Anti-smuggling (⑤):** a file is "derived" ONLY if the regeneration command
  DETERMINISTICALLY produces it from sources. A file not provably so CANNOT be
  classified derived; a false "derived" declaration to bypass the gate is BLOCKED
  + AUDITED. Drive a false-derived fixture and assert the block + audit event.
- **Consumes, never owns:** D12 CONSUMES the D7 VerdictObject and the D8 binding;
  it owns the gate decision surface, not the panel or the experiment harness.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All five owned items green · zero writes outside `crates/hugit-checks/regen/` ·
evidence bundle (non-opted no-regen proof, both-preconditions land/block
transcripts, per-regen attestation with gate-verdict ref, false-derived
block+audit) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–⑤: green/red) · evidence refs (test ids + fixture paths +
attestation ref + anti-smuggling transcript) · claims-respected: yes ·
deviations: none | waiver-ref.
