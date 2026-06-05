# WP-C3 — cache-warm boot
squad C · M · sonnet · 60k · branch: wp/C3

## Charter
Make runner boot cache-warm: `clw hydrate` on lease so a warm boot is ≤10s vs a
cold boot ≥60s, with toolchain layers shared from the CAS across jobs. When
CAS/AC is down mid-job, fail CLOSED — zero poisoned writes, no hang, no false
green. Depends on C2a's lease + C2b's runtime.

## Owned acceptance
① warm ≤10s vs cold ≥60s · ② toolchain layers shared · **③(+) CAS/AC down mid-job → fail CLOSED, zero poisoned writes, no hang, no false green**

## Contract deps
- `RunnerLease` (frozen — the lease C3 hydrates against; never modified).
- `FenceManifest` (from WP-00 — the path_set the hydrate materializes; fence
  enforcement is C5a).
- C2a/C2b runner API (consumed: boot hooks into the SEALed lease lifecycle).

## Claims
- `crates/hugit-runner/boot/` — hydrate-on-lease, toolchain-layer sharing from
  CAS, CAS/AC-down fail-closed path. Disjoint from C2a/C2b runner modules,
  C5a/C5b, and E4's shim.

## Dispatch packet
- Files received: this contract · decomposition §3 (C3 row) · warp-10-days
  Squad C (C3: "`clw hydrate` on lease + toolchain layers from CAS") ·
  whitepaper §5 (L0 CAS, L2 `clw`) · §5.2 (warm-CAS economics) · §9 (lock 5) ·
  C2a/C2b SEALed runner API · `hugit-contracts` (`RunnerLease`, `FenceManifest`).
- Anchors: warm ≤10s / cold ≥60s timing fixture; shared toolchain layers;
  CAS/AC-down fail-closed kill-test.
- Conventions: failing acceptance suite committed first.

## Implementation notes (every fork PRE-DECIDED)
- **Container-per-job on the Hetzner box** (from C2); Firecracker path
  **documented, not built**.
- `clw` is THE snapshot/hydrate/run reference client — boot calls `clw hydrate`
  on lease; do not re-implement hydration. Toolchain layers are CAS objects
  shared across jobs (one physical copy per content hash).
- **Fail-closed on substrate loss:** if CAS or AC is unreachable mid-job, the
  job fails CLOSED (defined error status, no partial/poisoned write to CAS/AC,
  no indefinite hang, never a false green) — §9 lock 5 degradation invariant.
- The broker (C5b, write-only secret model generalized) is untouched here;
  fence path enforcement is C5a. C3 consumes the `FenceManifest` only to scope
  the hydrate.

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-runner/boot/` ·
evidence bundle (warm/cold timing run, shared-layer proof, CAS/AC-down
fail-closed kill-test) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
