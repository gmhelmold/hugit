# WP-D1b — compaction/cold-tier + recovery + undo
squad D · size M · model route opus · context budget 80k · branch: wp/D1b

## Charter
Bound the hot event log and make "nothing is ever lost" hold at the CAS level:
compaction with cold-tier offload to R2 (designed in from day 1), full
recovery of ref state after hot-DO loss by replaying the cold tier, and
universal `undo` implemented as a compensating event that preserves history.
Builds on the append/hash-chain/replay primitives sealed in D1a.

## Owned acceptance (VERBATIM from decomposition v2.0 — D1)
> ③ compaction replay-equivalent, hot log bounded
> ④ undo restores + preserves history
> ⑥(+) recovery: hot-DO loss → full ref state rebuilt from cold tier (and/or mirror) replay-identical

(Partition statement — D1 split is exhaustive + disjoint across D1a/D1b/D1c.
**D1b owns compaction/cold-tier + recovery + undo = D1 items ③④⑥**; D1a owns
append/hash-chain/replay/tamper = ①②; D1c owns concurrency/perf = ⑤. The
spanning concern "history preservation under compaction+undo+recovery" lands in
this LATER half per the binding split rule. ①② ∪ ③④⑥ ∪ ⑤ = the full D1 set,
no item shared.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` (frozen Day-0 type).
- D1a's sealed primitives (append, hash chain, deterministic replay, tamper
  detection) — consumed as the in-crate substrate this WP extends; not modified.
- CoreLink CAS + R2 cold-tier surface (object put/get/list) consumed as a
  frozen external API; this WP is a CoreLink tenant, zero server-side changes.

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-refstore/src/compaction/` — hot-log bound + cold-range offload.
- `crates/hugit-refstore/src/coldtier/` — R2 cold-tier read/write of cold ranges.
- `crates/hugit-refstore/src/recovery/` — rebuild ref state after hot-DO loss.
- `crates/hugit-refstore/src/undo/` — compensating-event emission.
- `crates/hugit-refstore/tests/compaction_recovery_undo/` — owned acceptance suite.
- No writes under `crates/hugit-refstore/src/{log,replay,tamper}/` (D1a) or
  `crates/hugit-refstore/src/concurrency/` (D1c) or any other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §4, §6.5; §13 risk-3
  (single-vendor / DR / mirror as live replica); `docs/plan/decomposition.md`
  §4 (D1 row); `docs/plan/warp-10-days.md` D1 row.
- Anchors: compaction/cold-tier to R2 designed in from day 1; `undo` =
  compensating event; recovery from cold tier (and/or mirror) replay-identical.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; DCO + CHANGELOG `[Unreleased]`; cold-verify by a
  non-author agent.
- Token estimate: ~76k (≤80k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **Compaction/cold-tier to R2 is designed in from day 1** (DO storage caps are
  real): the hot log is bounded; cold ranges are sealed and offloaded to R2 as
  CAS objects. Compaction is **replay-equivalent** — the ref state derived from
  (hot log + cold tier) is byte-identical to replaying the full uncompacted log
  (③). Compaction never rewrites or drops history; it relocates it.
- **`undo` is a COMPENSATING EVENT** appended to the log — never a rewrite,
  never a deletion. Undoing an operation restores the prior ref state AND
  preserves the full history (the original event and its compensator both
  remain addressable). Force-push data loss is unexpressible by construction (④).
- **Recovery (⑥):** on hot-DO loss, ref state is rebuilt by replaying the cold
  tier (and/or the mirror as a fallback recovery source), replay-identical to
  the pre-loss state. Recovery-source precedence: cold tier first; the mirror is
  the secondary source (cf. E1's substrate-loss DR). "Nothing lost" holds at the
  CAS level; the hot log stays bounded.
- The mirror is consumed as a recovery source only — this WP does not build the
  mirror (that is E1); it asserts recoverability against the cold tier with the
  mirror path stubbed/contracted.
- No concurrency/throughput claims here (that is D1c); recovery and undo are
  asserted under single-writer semantics.

## DoD (global)
fmt + clippy + test + audit green · owned items (③④⑥) red→green · cold-verify
pass by a non-author agent · DCO + CHANGELOG discipline.

## Completeness
All owned items green · zero writes outside claims · evidence bundle
(compaction replay-equivalence proof + hot-log bound; undo restore+history-
preservation proof; hot-DO-loss recovery replay-identity proof) attached to the
SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned items red→green) · evidence refs (acceptance run, compaction
artifact, undo artifact, recovery artifact) · claims-respected assertion ·
deviations = none | waiver-ref.
