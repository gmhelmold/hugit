# WP-X11 — degradation composition
squad X · M · opus · 80k · branch: wp/X11 · scheduled: **both SEALs**

## Charter
Prove the invariants COMPOSE under partial degradation injected MID-operation:
the secrets broker fails CLOSED so no credential reaches any workspace during
the degradation window; objects written during degradation are marked
provenance-ABSENT with zero fabricated synthetic intent/attestation; and the
CoreLink non-interference baseline (X10) holds WHILE hugit is degraded, not only
when healthy. Runs at both SEALs.

## Owned acceptance
① smart-layer failure injected MID-operation (partial degradation window, not just steady-state outage): secrets broker fails CLOSED — no credential reaches any workspace during degradation
② objects written during degradation are marked provenance-ABSENT; no synthetic intent/attestation ever fabricated by a fallback path
③ CoreLink non-interference (X10 baseline) holds WHILE hugit is degraded, not only when healthy

## Contract deps
- `AttestationChain` (frozen — item ② asserts no synthetic attestation is fabricated by a fallback path; never modified).
- `RunnerLease`, `FenceManifest` (frozen — the workspace/broker boundary item ① attacks during the degradation window).
- Surfaces consumed as-built: C5 (fence + secrets broker), D3/D4 (write path + intents), X10 (the non-interference baseline item ③ re-asserts under degradation).
- Tenant boundary = HMAC-derived prefixes (CoreLink model).

## Claims
- `crates/hugit-invariants/x11/` (test crate + red-team fixtures for X11 only).
- No production crate paths; consumes the C5/D3/D4/X10 surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X11 row) + the v1.x degradation-honesty notes (D8⑦, C3③, D2⑤) for the composition context · `docs/whitepaper/hugit-v1.md` §9 (lock 2 secrets-never-enter; lock 5 degradation invariant) · frozen types above.
- Anchors: items ①–③ each a test module under `crates/hugit-invariants/x11/`; ① injects the broker failure MID-operation; ② scans degradation-window writes for provenance-ABSENT marking + zero fabricated intent/attestation; ③ re-runs the X10 baseline WHILE degraded.
- Conventions: failing suite first; "mid-operation" = injected during an in-flight op, not a pre-op outage; ② scans for ABSENCE of fabricated objects (not flag state).

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (broker fail-closed mid-op):** inject a smart-layer failure MID-operation (a partial degradation WINDOW, not a steady-state outage); assert the secrets broker fails CLOSED — NO credential reaches any workspace during the window. This is C5④'s broker-down property, composed under a mid-flight injection; X11 consumes C5's broker surface, never modifies it.
- **Item ② (provenance-ABSENT, no fabrication):** objects written during the degradation window are marked provenance-ABSENT; assert NO synthetic intent (cf. D3⑤/D4④) and NO synthetic `AttestationChain` is ever fabricated by a fallback path. The scan asserts the absence of any fabricated provenance object, not merely a flag.
- **Item ③ (non-interference under degradation):** re-assert the X10 CoreLink non-interference baseline WHILE hugit is degraded — the boundary must hold under degradation, not only when healthy. This consumes X10's baseline + caps; any CoreLink-facing load obeys the X10⑤ rate caps FIRST and the coordinated-window abort thresholds (the test obeys the non-interference it verifies).
- **attestation = `AttestationChain` from hugit-contracts; tenant boundary = HMAC-derived prefixes.** Consumes C5/D3/D4/X10 as-built; modifies none.
- Runs at **both SEALs**; each SEAL re-injects the mid-op degradation and re-asserts ①–③.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–③ red→green · cold-verify pass by a non-author · zero writes outside claims · security review at **both** SEALs (X11 runs at both).

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x11/` · evidence bundle (the mid-op broker fail-closed proof, the provenance-ABSENT + no-fabrication scan, the under-degradation non-interference re-assertion with caps+abort thresholds) attached to **both** SEALs.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + per-SEAL degradation-window refs), deviations = none | waiver-ref.
