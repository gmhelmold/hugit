# WP-X10 — the focus gate itself (incl. API-tenancy channel)
squad X · M · sonnet→opus · 80k · branch: wp/X10 · scheduled: **both SEALs**

## Charter
Prove the reformed focus gate: under hugit's heaviest sustained load, CoreLink's
launch route/sessions/CI show ZERO measurable degradation vs a hugit-idle
baseline; the dogfood set provably excludes corelink-server (enrollment fails
the build); X6 is rescoped to intra-hugit isolation while X10 owns the
adjacent-product boundary; the SHARED API-TENANCY channel is driven and proven
non-interfering; and hugit's own CoreLink-tenant consumption is policy-capped so
storms are bounded before they ever test fairness. Runs at both SEALs.

## Owned acceptance
① under hugit's heaviest sustained load (runner fleet + union queue + dogfood soak): CoreLink's launch route/sessions/CI capacity show ZERO measurable degradation vs a hugit-idle baseline
② the dogfood target set provably excludes corelink-server — enrollment during the launch window FAILS the build
③ **🔧 X6 rescoped: X6 = intra-hugit tenant isolation; X10 = the adjacent-product boundary**
④(R11) the SHARED API-TENANCY channel: drive hugit's full fleet-scale CAS/AC/R2 customer workload against CoreLink prod (write storms, cold-tier bursts, AC floods) and assert CoreLink's OTHER tenants' latency/availability are unaffected through CoreLink's own fairness layer — infra isolation does not reach this channel; the tenancy is shared by design
⑤(R11) preventive bound: hugit's own CoreLink-tenant consumption is rate/budget-capped by policy, so a hugit-side storm is structurally bounded before it ever tests CoreLink's fairness

## Contract deps
- `RunnerLease` (frozen — the fleet unit under load; consumed read-only).
- The dogfood harness (B8) target set, the union queue (B4), and hugit's CoreLink-tenant API client (CAS/AC/R2) — consumed as-built.
- Tenant boundary = HMAC-derived prefixes (CoreLink model): hugit IS a CoreLink tenant; item ④ drives that shared tenancy. CoreLink's own fairness layer is the surface item ④ tests THROUGH.

## Claims
- `crates/hugit-invariants/x10/` (test crate + red-team fixtures + the rate-cap policy fixture for X10 only).
- No production crate paths; consumes the dogfood/queue/fleet/CoreLink-tenant surfaces, never modifies them. **Read-only/load-only against CoreLink prod.**

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X10 row, items ①–⑤) + the v1.11 note (the shared API-tenancy channel) + §7 · `docs/plan/warp-10-days.md` (the reformed focus gate, "um não interfere — AT ALL"; dogfood excludes corelink-server) · `docs/product/command-catalog.md` (the focus gate) · `docs/whitepaper/hugit-v1.md` §12 (CoreLink launch route untouchable) · frozen `RunnerLease`.
- Anchors: items ①–⑤ each a test module under `crates/hugit-invariants/x10/`; ② is a build-failing enrollment guard; ④ is the fleet-scale CoreLink-prod load test; ⑤ is the policy rate-cap fixture that must be in place FIRST.
- Conventions: failing suite first; the "zero measurable degradation" tolerance and the abort thresholds are STATED before any load; ④ runs in a coordinated window with abort-on-any-CoreLink-latency-movement.

## Implementation notes (every fork PRE-DECIDED)
- **X10 = the adjacent-product boundary; X6 = intra-hugit isolation** (item ③, the rescope). X10 owns the CoreLink-facing channel; X6 owns hugit's own box/quota isolation. They do not overlap.
- **Item ① (no degradation under heaviest load):** drive hugit's heaviest sustained load (runner fleet + union queue + dogfood soak) and assert CoreLink's launch route/sessions/CI show ZERO measurable degradation vs a hugit-idle baseline (within the stated tolerance).
- **Item ② (corelink-server excluded, build-failing):** assert the dogfood target set provably excludes corelink-server; an enrollment of corelink-server during the launch window FAILS THE BUILD — a build-time guard, not a runtime warning.
- **Item ④ — the SHARED API-TENANCY channel (the load test that drives hugit's REAL workload against CoreLink prod):** hugit runs its substrate INSIDE CoreLink prod as a paying tenant. This item drives hugit's full FLEET-SCALE CAS/AC/R2 customer workload (write storms, cold-tier bursts, AC floods) **against CoreLink prod as a tenant — read-only/load-only against CoreLink**, and asserts CoreLink's OTHER tenants' latency/availability are unaffected THROUGH CoreLink's own fairness layer. Infra isolation (X6) does not reach this channel; the tenancy is shared by design.
  - **The test itself must obey the non-interference it verifies:** the X10⑤ rate/budget caps must be in place FIRST; the load is **ramped**, with **abort thresholds on ANY CoreLink latency movement**, run in a **coordinated window**. The test is read-only/load-only against CoreLink — zero CoreLink server changes.
- **Item ⑤ (preventive bound, in place FIRST):** hugit's OWN CoreLink-tenant consumption is rate/budget-capped BY POLICY (the committed policy fixture), so a hugit-side storm is structurally bounded BEFORE it ever tests CoreLink's fairness. ⑤ is the precondition for ④ — it lands and is asserted first.
- **Route note:** sonnet→opus — the build is contract-determined (sonnet) but item ④'s adversarial against-prod judgment escalates to opus for the coordinated-window load.
- Runs at **both SEALs**; each SEAL re-measures ① and re-runs ④ under the ⑤ caps.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–⑤ red→green · cold-verify pass by a non-author · zero writes outside claims · security review at **both** SEALs (X10 runs at both) · item ④ executed only with ⑤'s caps in place and abort thresholds armed.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x10/` · zero CoreLink server changes (read-only/load-only) · evidence bundle (the stated-tolerance + abort-threshold doc, idle-vs-load measurement at each SEAL, the build-failing enrollment guard, the fleet-scale shared-tenancy load result through CoreLink's fairness layer, the rate-cap policy fixture) attached to **both** SEALs.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + per-SEAL load-result + policy-fixture refs), deviations = none | waiver-ref.
