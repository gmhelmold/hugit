# WP-C2a — ephemeral runner: lease lifecycle + isolation
squad C · M · opus · 80k · branch: wp/C2a

## Charter
Build the ephemeral runner v0 on the Hetzner box as container-per-job: acquire a
lease, run one job in an isolated container, and tear down leaving nothing behind.
This is the correctness half of C2 (lifecycle + isolation); load, expiry, and
crash recovery are C2b, which depends on the lease state machine frozen here.

## Owned acceptance
**Partition of the original C2 set (5 items) — this contract owns the lifecycle +
isolation half; concurrency/throughput + crash recovery + expiry go to C2b
(the later half). Union(C2a, C2b) = C2; intersection = ∅.**

① destroy leaves nothing (forensic re-scan) · ② lease isolation (tmp/net)

## Contract deps
- `RunnerLease` (frozen — consumed as the lease object; lease id, path_set ref,
  expiry, principal chain; never modified here).
- `FenceManifest` (from WP-00 — referenced as the materialization contract the
  lease carries; fence enforcement itself is C5a).

## Claims
- `crates/hugit-runner/` — lease lifecycle + isolation modules (lease acquire,
  container spawn, job run, teardown/forensic-clean). Disjoint from C2b's
  load/crash/expiry modules and from `hugit-runner/boot` (C3) and
  `hugit-runner/shim` (E4).
- C2a owns `crates/hugit-runner/{lease,isolation,teardown}` (the lifecycle path).

## Dispatch packet
- Files received: this contract · decomposition §3 (C2 row) ·
  warp-10-days Squad C (C2 deliverable: "container-per-job now, Firecracker
  upgrade path documented") · whitepaper §5 (L3) · §9 (lock 1, lock 5) ·
  WP-C1's inventory (reuse verdicts) · `hugit-contracts` (`RunnerLease`,
  `FenceManifest`).
- Anchors: container-per-job spawn; teardown that a forensic re-scan proves
  left nothing; tmp + net isolation between leases.
- Conventions: failing acceptance suite committed before implementation.

## Implementation notes (every fork PRE-DECIDED)
- **Container-per-job on the single Hetzner-class box** is the runtime. The
  **Firecracker upgrade path is documented, not built** — write the upgrade note
  in the module docs; do not implement Firecracker.
- `clw` is the snapshot/hydrate/run reference client — the runner drives `clw`
  for materialization; it does not re-implement snapshot/hydrate.
- **Isolation mechanism:** per-job container with private tmp and an isolated
  network namespace; "leaves nothing" = forensic re-scan of the box (disk + mounts
  + process table + network) finds zero residue after teardown.
- The broker (CoreLink **write-only secret model generalized**, credentials
  never on the runner) is C5b; C2a's lease carries no raw credentials by
  construction.
- Fence path enforcement is C5a; C2a consumes the `FenceManifest` only to scope
  the container's materialized view, not to enforce ENOENT (that is C5a's claim).

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-runner/{lease,isolation,
teardown}` · evidence bundle (forensic re-scan transcript, isolation test output)
attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
