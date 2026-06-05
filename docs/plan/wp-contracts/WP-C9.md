# WP-C9 — workspace lifecycle (spawn/attach/resume)
squad C · M · sonnet · 70k · branch: wp/C9

## Charter
Workspace lifecycle: attach joins a live workspace (same fence/materialization)
without respawn; resume restores state + fence and cannot exceed the original
path_set; spawn is <1s with identical concurrent spawns deduped to one
materialization; and local vs remote execution yields identical observable
results. Depends on C2 + the C5a fence.

## Owned acceptance
① attach joins live workspace (same fence/materialization) without respawn · ② resume restores state+fence; resumed ws cannot exceed original path_set · ③ spawn <1s; identical concurrent spawns dedup to one materialization · ④ local vs remote execution: identical observable results

## Contract deps
- `RunnerLease` (frozen — the lease a workspace runs under; consumed, not
  modified).
- `FenceManifest` (from WP-00 — the path_set resume/attach must not exceed;
  consumed, not modified).
- C2a/C2b runner API + C5a fence API (consumed: lifecycle joins/restores the
  SEALed lease + fence; C9 never re-implements either).

## Claims
- `crates/hugit-runner/ws/` — spawn/attach/resume lifecycle (dedup,
  state+fence restore, the path_set ceiling on resume, local/remote
  result-identity). Disjoint from C2a/C2b runner modules, `hugit-runner/boot`
  (C3), and `hugit-runner/shim` (E4).

## Dispatch packet
- Files received: this contract · decomposition §3 (C9 row) · warp-10-days
  Squad C (C9 maps to "Workspace spawn/attach/resume" in command-catalog) ·
  command-catalog Phase C ("`clw` + fencing; <1s, deduped, local/remote
  transparent") + worker-agent surface (`hugit ws spawn/attach/snap/gc`) ·
  whitepaper §5.1 (workspaces born <1s, fenced) · C2a/C2b + C5a SEALed APIs ·
  `hugit-contracts` (`RunnerLease`, `FenceManifest`).
- Anchors: attach-without-respawn; resume-restores-state+fence with path_set
  ceiling; <1s spawn + concurrent-spawn dedup; local≡remote observable results.
- Conventions: failing acceptance suite committed first.

## Implementation notes (every fork PRE-DECIDED)
- **`clw` is THE snapshot/hydrate/run reference client** — spawn/attach/resume
  drive `clw`; C9 does not re-implement materialization, it orchestrates the
  lifecycle over the C2 runtime + C5a fence.
- **Attach (①):** joins a live workspace sharing the same fence/materialization —
  no respawn, no re-hydrate.
- **Resume (②):** restores state + fence; the resumed workspace is bounded by the
  original `FenceManifest` path_set and **cannot exceed it** (security: resume is
  not a fence-widening hole).
- **Spawn + dedup (③):** spawn <1s; identical concurrent spawns dedup to ONE
  materialization (warm-CAS economics).
- **Local≡remote (④):** local and remote execution produce identical observable
  results — the same pure function either way (consistent with B2's local≡runner
  byte-identity philosophy; C9 asserts it at the workspace-lifecycle level).
- Runtime is container-per-job on the Hetzner box; Firecracker path documented,
  not built. The fence (C5a) is the ceiling; the broker (C5b) is untouched here.

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-runner/ws/` ·
evidence bundle (attach-no-respawn proof, resume state+fence restore with
path_set-ceiling assertion, <1s spawn + dedup measurement, local≡remote
result-identity) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
