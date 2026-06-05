# WP-D1c — concurrency/perf (serialization, p99)
squad D · size S · model route opus · context budget 60k · branch: wp/D1c

## Charter
Prove the event-log core serializes concurrent operations with zero loss and
meets its latency budget under load: 100 concurrent operations are serialized
through the single-writer Durable Object, no event is lost, and p99 stays under
500ms. This is the load/perf proof family over the correctness primitives of
D1a and the durability of D1b.

## Owned acceptance (VERBATIM from decomposition v2.0 — D1)
> ⑤ 100 concurrent ops: serialized, 0 loss, p99<500ms

(Partition statement — D1 split is exhaustive + disjoint across D1a/D1b/D1c.
**D1c owns concurrency/perf = D1 item ⑤**; D1a owns append/hash-chain/replay/
tamper = ①②; D1b owns compaction/cold-tier + recovery + undo = ③④⑥. ①② ∪ ③④⑥
∪ ⑤ = the full D1 set, no item shared.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` (frozen Day-0 type).
- D1a's sealed append/hash-chain/replay primitives and D1b's durability paths —
  consumed as the in-crate substrate this WP loads; not modified.
- CoreLink CAS surface consumed as a frozen external API; CoreLink tenant only,
  zero server-side changes.

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-refstore/src/concurrency/` — serialization + back-pressure
  around the single-writer append point.
- `crates/hugit-refstore/tests/concurrency_perf/` — the owned load/perf
  acceptance suite (100-concurrent fixture, p99 harness).
- No writes under `crates/hugit-refstore/src/{log,replay,tamper}/` (D1a) or
  `crates/hugit-refstore/src/{compaction,coldtier,recovery,undo}/` (D1b) or any
  other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §6.5 (event log);
  `docs/plan/decomposition.md` §4 (D1 row); `docs/plan/warp-10-days.md` D1 row.
- Anchors: one Durable Object per repo = the single serialization point; refs =
  derived view; the p99<500ms budget at 100 concurrent ops.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; DCO + CHANGELOG `[Unreleased]`; cold-verify by a
  non-author agent.
- Token estimate: ~54k (≤60k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **The single Durable Object per repo IS the serialization point.** Concurrent
  operations are ordered through that one append point — there is no second
  writer to reconcile. Serialization is structural, not lock-juggled.
- **Zero loss under contention:** every accepted operation produces exactly one
  EventRecord on the chain; back-pressure (not drop) when the writer is
  saturated — an operation is queued/rejected explicitly, never silently lost.
- **p99 < 500ms at 100 concurrent ops** is the measured budget; the harness
  drives 100 concurrent operations and asserts the p99 latency bound plus the
  no-loss invariant (record count == accepted-op count, chain intact).
- This WP adds no new event semantics — it loads and measures the primitives
  D1a/D1b sealed; any correctness regression surfaced here is a defect in those,
  fixed at root (no bypass).

## DoD (global)
fmt + clippy + test + audit green · owned item (⑤) red→green · cold-verify pass
by a non-author agent · DCO + CHANGELOG discipline.

## Completeness
Owned item green · zero writes outside claims · evidence bundle (100-concurrent
serialization + zero-loss proof; p99<500ms latency report) attached to the SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned item red→green) · evidence refs (acceptance run, p99 latency
report, zero-loss artifact) · claims-respected assertion · deviations = none |
waiver-ref.
