# WP-E1b — verified mirror: failure modes (outage, partial divergence, one-way enforcement)

squad E · M · opus · 80k · branch: wp/E1b

## Charter
Prove the mirror's **failure modes**: GitHub outage (429/5xx) survived by the
durable queue with bounded backoff and no drop/reorder; force-push / branch-
delete / tag ops replicate correctly with no orphan-induced false divergence;
partial divergence repaired scoped to the broken ref with webhook-loss poll
fallback; and the one-way law enforced — a write made directly on the mirror
never becomes truth. Happy path is E1a; bootstrap+DR is E1c.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E1; this WP owns items ② ④ ⑤ ⑥ ⑦.)*

- **②** divergence→alarm+repair+incident
- **④(+)** GitHub 429/5xx for N hours: durable queue, bounded backoff, no
  drop/reorder; on recovery drains to verified sync + incident records gap
- **⑤(+)** force-push/branch-delete/tag ops replicate; deleted refs absent; no
  false divergence from orphans
- **⑥(+)** partial divergence: repair scoped to broken ref only; webhook loss →
  poll fallback still detects within SLA
- **⑦(R2)** ONE-WAY enforced: a write made directly on the GitHub mirror never
  propagates back/never becomes truth — treated as divergence (alarm →
  forge-authoritative repair → incident), zero reverse sync

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `EventRecord` — forge-authoritative truth used to compute divergence and
  scope repair.
- `AppWebhooks` — webhook delivery channel; webhook-loss poll fallback (⑥)
  consumes its absence.
- The **durable queue** type + capacity bound from **E1a** (`hugit-mirror/
  queue`): E1b exercises outage/backoff/drain/overflow against it; never
  modifies its public shape.
- The **outbound writer + verify** seam from **E1a** (`hugit-mirror/outbound`,
  `hugit-mirror/verify`): the divergence signal this WP's alarm/repair consumes.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-mirror/src/divergence/` — divergence detection, scoped repair,
  incident emission, one-way enforcement (reverse-write → divergence).
- `crates/hugit-mirror/src/outage/` — outage handling: bounded backoff, drain-
  to-verified on recovery, gap-incident recording (drives E1a's queue).
- `crates/hugit-mirror/src/refops/` — force-push/branch-delete/tag replication +
  orphan-aware no-false-divergence logic.
- `crates/hugit-mirror/src/poll/` — webhook-loss poll fallback detector.
- `tests/mirror/divergence_*.rs`, `tests/mirror/outage_*.rs`,
  `tests/mirror/refops_*.rs`, `tests/mirror/oneway_*.rs`.

## Dispatch packet
- This contract file.
- Frozen `EventRecord`, `AppWebhooks` anchors.
- E1a queue/outbound/verify seam signatures (queue capacity bound, divergence
  signal, push-verify result).
- Anchor: `crates/hugit-mirror/lib.rs` barrel exports `divergence`, `outage`,
  `refops`, `poll`.
- Conventions: every divergence → `{alarm, scoped repair, incident}` triple;
  repair is **forge-authoritative** (hugit/forge state wins, mirror is
  overwritten); fail-CLOSED — never mark a divergent ref synced.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Outage (④)**: on 429/5xx, items hold in E1a's durable queue; retry with
  **bounded exponential backoff** (stated max). No drop, no reorder. On
  recovery, **drain to verified sync** and emit a **gap incident** covering the
  outage window. Capacity-overflow path is E1a⑩'s backpressure; E1b drives it.
- **Ref ops (⑤)**: force-push, branch-delete, tag create/delete replicate as
  first-class ops; a deleted ref is **absent** on the mirror after sync;
  orphaned objects left by force-push do **not** register as divergence (orphan-
  aware diff — compare ref tips, not loose-object sets).
- **Partial divergence (⑥)**: repair is **scoped to the broken ref only** —
  never a full re-seed. If webhooks are lost, a **poll fallback** still detects
  divergence within the stated SLA.
- **One-way (⑦)**: a direct write on the GitHub mirror is detected as
  divergence and resolved **forge-authoritative** (hugit state overwrites the
  mirror write) → alarm + incident. **Zero reverse sync** exists; assert the
  reverse-propagation codepath is structurally absent (this is the warp-scope
  guard that E6 is gate-bound behind).
- All divergence handling is **fail-CLOSED**: an undecidable/degraded detector
  state treats the ref as divergent, never as synced.

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` green on `wp/E1b`.
- Owned items ② ④ ⑤ ⑥ ⑦ red→green; failing suites committed first.
- Cold verification by non-author; security review at SEAL (one-way
  enforcement is a trust-boundary control).
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (② ④ ⑤ ⑥ ⑦) green.
- Zero writes outside Claims.
- Evidence bundle (outage drain trace, scoped-repair proof, orphan no-false-
  divergence proof, reverse-write→divergence proof, poll-fallback SLA proof)
  attached to SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs · items ②④⑤⑥⑦ red→green ·
deviations = none | waiver-ref.
