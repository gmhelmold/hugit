# WP-X6 — intra-fabric isolation (intra-hugit tenant isolation)
squad X · M · sonnet · 60k · branch: wp/X6 · scheduled: **both SEALs**

## Charter
Prove hugit at full load does not degrade CoreLink (latency/availability within
stated tolerance) and that hugit infra is resource-isolated from CoreLink
runners/sessions (separate boxes/quotas, config-asserted). Per X10③'s rescope,
X6 is the INTRA-hugit tenant-isolation / infra-isolation leg; X10 owns the
adjacent-product boundary. Runs at both SEALs.

## Owned acceptance
① hugit at full load concurrently with CoreLink workloads → CoreLink latency/availability unaffected within stated tolerance (measured, both SEALs)
② hugit infra is resource-isolated from CoreLink runners/sessions (separate boxes/quotas, asserted by config test)

## Contract deps
- `RunnerLease` (frozen — the unit whose box/quota isolation item ② asserts; consumed read-only).
- The runner/fabric infra config (C2 boxes/quotas) — consumed as-built.
- Tenant boundary = HMAC-derived prefixes (CoreLink model) — the intra-hugit partition this WP's isolation leg concerns.

## Claims
- `crates/hugit-invariants/x6/` (test crate + red-team fixtures for X6 only).
- No production crate paths; consumes the infra config + load surfaces, never modifies them. **Read-only/load-only against CoreLink** (per the X10⑤ caps; see notes).

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X6 row + X10③ rescope note) · `docs/whitepaper/hugit-v1.md` §9 + §12 (CoreLink launch route untouchable) · `docs/plan/warp-10-days.md` (the reformed focus gate, zero interference) · frozen `RunnerLease`.
- Anchors: item ① a measured load comparison (hugit-idle baseline vs hugit-full-load) asserting CoreLink latency/availability within stated tolerance; item ② a config test asserting separate boxes/quotas.
- Conventions: failing suite first; the tolerance is STATED (committed under the test crate) before measurement; item ② asserts config, not runtime.

## Implementation notes (every fork PRE-DECIDED)
- **X6 = intra-hugit tenant isolation + infra isolation** (the X10③ rescope: X6 = intra-hugit; X10 = adjacent-product boundary). X6 asserts the box/quota separation and the latency-tolerance under hugit's own load; X10 owns the shared-API-tenancy channel against CoreLink prod.
- **Item ① (no degradation):** drive hugit to full fabric load (runner fleet) and measure CoreLink latency/availability against a hugit-idle baseline; assert within the stated tolerance. This is the infra-isolation reading of non-interference; the SHARED-API-TENANCY channel is X10④'s job, not X6's.
- **Item ② (config isolation):** a config test asserts hugit runners/sessions run on **separate boxes/quotas** from CoreLink's — structural isolation, asserted statically.
- **CoreLink interaction is read-only/load-only**, and any load that reaches CoreLink obeys the **X10⑤ rate/budget caps FIRST** and the abort thresholds (any CoreLink latency movement → abort), run in a coordinated window. X6's item ① measures hugit's OWN load impact via infra isolation; it does not itself drive the fleet-scale tenant workload (that is X10④).
- Runs at **both SEALs** (sprint-1 mid-SEAL and sprint-2 final SEAL); each SEAL re-measures item ① and re-asserts item ②.
- Consumes C2 infra config as-built; modifies nothing.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–② red→green · cold-verify pass by a non-author · zero writes outside claims · security review at **both** SEALs (X6 runs at both).

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x6/` · evidence bundle (the stated-tolerance doc, the idle-vs-load measurement at each SEAL, the box/quota config-test report) attached to **both** SEALs.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + per-SEAL measurement refs), deviations = none | waiver-ref.
