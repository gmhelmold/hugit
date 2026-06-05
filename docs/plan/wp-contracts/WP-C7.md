# WP-C7 — budgets + queue fairness
squad C · S · sonnet · 60k · branch: wp/C7

## Charter
Per-tenant check budgets + queue fairness: an exhausted budget queues work (never
drops it), surfaced as a defined status field + event; under contention every
tenant's p95 queue wait is ≤ a defined bound and throughput share ≥ a defined
floor; and metering is accurate within ±5% of actual on the fixture workload.
Depends on B4's queue.

## Owned acceptance
① **🔧 exhausted→queued not dropped; surfaced as defined status field + event** · ② **🔧 fairness bound: under contention, every tenant's p95 queue wait ≤ defined bound and throughput share ≥ defined floor (interleave fixture)** · **③ 🔧 metering accuracy: accounted ≈ actual within ±5% on the fixture workload**

## Contract deps
- `QueueApi` (frozen — C7 adds budget + fairness over the B4 queue; consumed,
  never modified).
- The B4 union-queue core (consumed as the queue C7 budgets/fairness-schedules;
  B4 owns ordering + landing, C7 owns budgets + fairness).

## Claims
- `crates/hugit-queue/budget/` — per-tenant budgets, the exhausted→queued status
  field + event, the fairness scheduler (p95-bound + throughput-floor), and the
  metering accounting. Disjoint from the rest of `hugit-queue` (B4 core).

## Dispatch packet
- Files received: this contract · decomposition §3 (C7 row) · warp-10-days
  Squad C (C7: "per-tenant check budgets + queue fairness") · whitepaper §11
  (pricing doctrine: flat, never meter the customer's own compute) ·
  `hugit-contracts` (`QueueApi`).
- Anchors: the exhausted→queued status field + event schema; the defined p95
  wait bound + throughput-share floor; the ±5% metering tolerance.
- Conventions: failing acceptance suite committed first; interleave fixture for
  ② + a fixture workload for the ±5% ③ assertion.

## Implementation notes (every fork PRE-DECIDED)
- **Exhausted → queued, not dropped (①):** budget exhaustion enqueues, surfaced
  as a defined status field + event — never a silent drop (the pricing doctrine
  is flat; surfacing is honest backpressure, not an overage charge).
- **Fairness (②):** under contention the scheduler holds every tenant's p95
  queue wait ≤ the defined bound and throughput share ≥ the defined floor,
  proven on an interleave fixture. The bound + floor are stated constants in the
  module, not left open.
- **Metering accuracy (③):** accounted ≈ actual within ±5% on the fixture
  workload — the accounting is reconcilable, not approximate hand-waving.
- C7 budgets feed C8 (shadow checks decrement budget), C10 (pricing no-shock),
  and D13 (tournament fan-out caps) — but C7's claim is the budget + fairness
  engine only; those consumers own their own wiring.
- Runs over the B4 queue; the runner is C2 (container-per-job on the Hetzner
  box). C7 does not own runner or queue-core code.

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-queue/budget/` ·
evidence bundle (exhausted-queued status+event trace, interleave-fixture p95 +
throughput-share measurement, ±5% metering reconciliation) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
