# WP-X2 — attestation end-to-end
squad X · M · opus · 70k · branch: wp/X2 · scheduled: **sprint 2**

## Charter
Prove the full attestation chain resolves cryptographically end-to-end, that
tampered/unsigned attestations are rejected at promotion, that verification is
a public documented procedure, and that a cross-tenant-shared hit attests to an
anonymized PLATFORM identity — never leaking the producing tenant.

## Owned acceptance
① artifact attestation resolves full chain (tree+def+runner+model+principal) cryptographically
② tampered/unsigned attestation rejected at promotion
③ verification is a public, documented procedure
④(R7) cross-tenant-shared hit honesty: tenant B's `why`/attestation on a shared public-deterministic artifact resolves to an anonymized PLATFORM attestation — never leaks tenant A's principal/runner identity, never mis-attributes B as producer

## Contract deps
- `AttestationChain {tree, def, runner, model, principal, sig}` (frozen — THE object under test; resolved, verified, tamper-checked; never modified).
- `CheckResult`, `CheckDef` (frozen — the attested artifacts).
- `RegenGate {optin_scope, repass, indep_verdict}` (frozen — item ② attacks the promotion boundary that rejects tampered attestations).
- Tenant boundary = HMAC-derived prefixes (CoreLink model) — the partition that item ④'s anonymized platform attestation must respect.

## Claims
- `crates/hugit-invariants/x2/` (test crate + red-team fixtures for X2 only).
- No production crate paths; consumes the attestation surface as-built, never modifies it.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X2 row) + §7 (DAG context) · `docs/whitepaper/hugit-v1.md` §9 (Provenance: `{tree_hash, check_def, runner, model, prompt, principal chain}`) · `docs/product/command-catalog.md` (`hugit why` provenance, adversarial verdict panels) · frozen types above.
- Anchors: items ①–④ each a test module under `crates/hugit-invariants/x2/`; ① resolves a full chain, ② is the tamper/unsigned attack, ④ is the cross-tenant `why` honesty attack.
- Conventions: failing suite first; cryptographic assertions use the frozen `sig` field; the public verification procedure (③) is emitted as a committed doc the test executes against.

## Implementation notes (every fork PRE-DECIDED)
- **Attestation = `AttestationChain` from hugit-contracts.** Item ① resolves all five links (tree+def+runner+model+principal) and verifies the signature cryptographically; no link may be unresolved.
- **Item ② (promotion gate):** present a tampered chain (mutated `tree`/`principal`) and an unsigned chain at the promotion boundary (the `RegenGate`/attestation-verify surface); assert both rejected fail-closed, with an audit event. This is the same tamper-evidence X12①/D8⑨ rely on; X2 owns the e2e crypto proof.
- **Item ③:** the verification procedure is PUBLIC and documented — committed as a runnable doc under the test crate; the test executes it against a known-good and a known-bad attestation to prove the doc is sufficient, not aspirational.
- **Item ④ (shared-hit honesty):** tenant B does `why`/attestation on a public-deterministic shared artifact; assert the resolved attestation is the **anonymized PLATFORM** form — zero tenant-A `principal`/`runner` bytes (cf. X1③ no-leak side), AND B is NOT mis-attributed as producer. The X1 side proves no leak; X2④ proves the honesty/non-mis-attribution of the surfaced attestation.
- Consumes B2/D7 attestation surfaces and the `why` surface as-built; modifies neither.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–④ red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x2/` · evidence bundle (chain-resolution proof, tamper/unsigned rejection + audit assertions, the public verification doc + its executing test, the anonymized-platform-attestation proof) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + public-procedure doc ref), deviations = none | waiver-ref.
