# WP-C2b — ephemeral runner: concurrency/throughput + crash recovery + expiry
squad C · M · opus · 70k · branch: wp/C2b

## Charter
On top of C2a's frozen lease lifecycle, prove the runner under load: ≥8 concurrent
jobs per box, hard-kill on expiry, and crash recovery — a mid-job box crash is
detected as lost, requeued/surfaced, never silently dropped or falsely green, with
lease/fence cleaned up. This is the load + recovery half of C2.

## Owned acceptance
**Partition of the original C2 set (5 items) — this contract owns the
concurrency/throughput + crash recovery + expiry half (the later half; the
spanning recovery item rides here). Union(C2a, C2b) = C2; intersection = ∅.**

③ expiry hard-kill · ④ ≥8 concurrent/box · **⑤(R2) box crash mid-job (not expiry): job detected lost → requeued/surfaced, no silent drop, no false green, lease/fence cleaned up**

## Contract deps
- `RunnerLease` (frozen — consumed for expiry field + lease id; never modified).
- `FenceManifest` (from WP-00 — referenced for the fence-cleanup obligation on
  crash; fence materialization/enforcement is C5a/C5b).
- C2a's lease lifecycle + isolation API (consumed as the frozen substrate this
  WP loads — acquire/spawn/teardown are C2a's; C2b adds concurrency, expiry-kill,
  crash detection).

## Claims
- `crates/hugit-runner/{concurrency,expiry,recovery}` — concurrency scheduler,
  expiry hard-kill path, crash-detection + requeue/surface path. Disjoint from
  C2a's `{lease,isolation,teardown}`.

## Dispatch packet
- Files received: this contract · decomposition §3 (C2 row) · warp-10-days
  Squad C (C2) · whitepaper §5 (L3) · §9 (lock 5 degradation invariant) ·
  the SEALed C2a API surface · `hugit-contracts` (`RunnerLease`, `FenceManifest`).
- Anchors: ≥8 concurrent containers on the box; expiry → hard-kill; crash
  mid-job → lost-detection → requeue/surface + lease/fence clean.
- Conventions: failing acceptance suite committed first; kill-test for ⑤.

## Implementation notes (every fork PRE-DECIDED)
- **Container-per-job on the single Hetzner-class box**; concurrency = N
  containers in parallel, target ≥8. Firecracker path **documented, not built**.
- `clw` is the reference snapshot/hydrate/run client; recovery re-uses C2a's
  teardown for fence/lease cleanup — do not re-implement teardown.
- **Crash semantics:** a box crash mid-job is distinct from expiry — the job is
  detected lost (heartbeat/lease-liveness absent), then requeued or surfaced as a
  defined lost-job status; never a silent drop, never a false green. Fail toward
  surfacing (degradation invariant, §9 lock 5).
- **Expiry semantics:** expiry → hard-kill of the container and lease, distinct
  from crash; expiry is deterministic from the `RunnerLease` expiry field.
- Lease/fence cleanup on crash reuses C5a's fence teardown contract; the broker
  (C5b) holds no state that survives a crash because credentials never land on
  the runner (write-only secret model generalized).

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims. The ⑤ crash-recovery item
carries a kill-test in its acceptance suite.

## Completeness
All owned items green · zero writes outside `crates/hugit-runner/{concurrency,
expiry,recovery}` · evidence bundle (concurrency run at ≥8, expiry-kill trace,
crash kill-test transcript) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
