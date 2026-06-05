# WP-X8 — self-release attestation
squad X · M · opus · 60k · branch: wp/X8 · scheduled: **sprint 2**

## Charter
Prove hugit attests its OWN releases: every App/CLI/runner-image release is
signed and published to a verifiable transparency log, the running App verifies
its own provenance at boot, and an unsigned/tampered self-build fails CLOSED.
The supply-chain invariant turned on hugit itself.

## Owned acceptance
① every hugit App/CLI/runner-image release signed + published to a verifiable transparency log
② the running App verifies its own provenance at boot
③ unsigned/tampered self-build fails CLOSED

## Contract deps
- `AttestationChain {tree, def, runner, model, principal, sig}` (frozen — the self-release attestation form; `sig` carries the release signature; never modified).
- The release/build surface (workspace scaffold + CI) and the App boot path (B1) — consumed as-built.
- Builds on X4 (DAG: X8⇠X4): X4 proves third-party images are pinned/verified; X8 proves hugit's OWN releases are signed/verified.

## Claims
- `crates/hugit-invariants/x8/` (test crate + red-team fixtures for X8 only).
- No production crate paths; consumes the release + boot surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X8 row) + §7 (DAG: X8⇠X4) · `docs/whitepaper/hugit-v1.md` §9 (Provenance; the five locks; fail-closed) · `docs/product/command-catalog.md` (attestation→model-level provenance) · frozen `AttestationChain`.
- Anchors: items ①–③ each a test module under `crates/hugit-invariants/x8/`; ① asserts signature + transparency-log publication, ② asserts boot-time self-provenance verification, ③ is the unsigned/tampered self-build fail-closed attack.
- Conventions: failing suite first; "verifiable transparency log" = the publication is independently checkable; ② asserts the check happens AT BOOT before serving.

## Implementation notes (every fork PRE-DECIDED)
- **Self-release attestation = `AttestationChain` from hugit-contracts**, with the release artifact as the attested object and `sig` as the release signature. Item ① asserts every App/CLI/runner-image release is signed AND published to a verifiable transparency log (the publication is independently checkable from the log alone).
- **Item ② (boot-time self-verification):** the running App verifies its OWN provenance at boot against its published attestation; the test asserts the verification gates serving — a boot with a verifiable attestation proceeds, the check is not skippable.
- **Item ③ (fail CLOSED):** present an unsigned self-build and a tampered self-build; assert boot fails CLOSED in both cases — the App does not serve, with an audit event. This is the self-turned form of X4③ (tampered third-party image fails closed); X8③ is hugit attesting itself.
- X8 builds ON X4's pinning floor (DAG: X8⇠X4) and consumes the release/boot surfaces as-built; it modifies neither. It does not re-prove third-party image pinning (that is X4).

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–③ red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x8/` · evidence bundle (release-signature + transparency-log publication proof, boot-time self-verification assertion, the unsigned/tampered fail-closed proof) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + transparency-log ref), deviations = none | waiver-ref.
