# WP-C5a — sparse fence materialization + path enforcement
squad C · M · opus · 80k · branch: wp/C5a

## Charter
Claim-fenced workspaces (SECURITY), fence half: sparse hydrate by path-set so a
workspace physically materializes only its claimed paths, and a write or read
outside the path_set returns ENOENT — the file is not there. Sparse
materialization IS the fence. The secrets broker + escape red-team harness are
C5b, which rides on this frozen fence.

## Owned acceptance
**Partition of the original C5 set (6 items) — this contract owns the sparse
fence materialization + path enforcement half; the secrets broker + escape
red-team harness go to C5b (the later half; the spanning red-team item rides
there). Union(C5a, C5b) = C5; intersection = ∅.**

① outside path_set→ENOENT

## Contract deps
- `FenceManifest` (from WP-00 — frozen — THE path_set contract C5a materializes
  and enforces; never modified here).
- `RunnerLease` (frozen — the lease that carries the `FenceManifest`; consumed,
  not modified).

## Claims
- `crates/hugit-fence/` — sparse-hydrate-by-path-set + path enforcement (ENOENT
  outside path_set) + fence teardown. C5a owns
  `crates/hugit-fence/{materialize,enforce}`; C5b owns the broker submodule —
  disjoint by construction.

## Dispatch packet
- Files received: this contract · decomposition §3 (C5 row) · warp-10-days
  Squad C (C5: "sparse hydrate by path-set … claims as fences") ·
  command-catalog Phase C ("Claim-fenced workspaces (SECURITY): sparse
  materialization = physical reach limits") · whitepaper §6.1 (claims runtime:
  "writes outside are physically impossible — the file isn't there") · §9
  (lock 1: claims as physical fences) · `hugit-contracts`
  (`FenceManifest`, `RunnerLease`).
- Anchors: `clw hydrate` filtered by the `FenceManifest` path_set; the ENOENT
  guarantee on any access outside the set.
- Conventions: failing acceptance suite committed first; security review at SEAL.

## Implementation notes (every fork PRE-DECIDED)
- **`clw` is THE snapshot/hydrate/run reference client** — the fence is sparse
  hydrate by path-set via `clw`; C5a does not re-implement materialization, it
  filters it by the `FenceManifest`.
- **The fence = sparse materialization:** outside the path_set the file is
  absent, so an access returns ENOENT — not a permission denial layered on top,
  but physical absence (§6.1, §9 lock 1).
- The `FenceManifest` is the frozen WP-00 contract; C5a is its enforcement
  surface. The broker (CoreLink **write-only secret model generalized**,
  credentials never on the runner) is C5b's claim — C5a holds no credentials.
- Runs on the C2 container-per-job runtime (Hetzner box; Firecracker path
  documented, not built). Fence teardown coordinates with C2a teardown / C2b
  crash-recovery cleanup.

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-fence/{materialize,
enforce}` · evidence bundle (ENOENT-outside-path_set test transcript) attached
to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
