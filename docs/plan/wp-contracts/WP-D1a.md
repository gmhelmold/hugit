# WP-D1a — event-log core (append, hash chain, replay, tamper)
squad D · size M · model route opus · context budget 80k · branch: wp/D1a

## Charter
Build the append-only, hash-chained `EventRecord` log that is the source of
truth for one repository — one Durable Object per repo owns it; refs are a
derived view computed by replaying the log. This WP delivers the append path,
the per-record hash chain, deterministic replay to a ref state, and tamper
detection. Compaction/cold-tier, recovery, undo (D1b) and concurrency/perf
(D1c) build on the primitives sealed here.

## Owned acceptance (VERBATIM from decomposition v2.0 — D1)
> ① 10k-event replay identical
> ② tamper detected

(Partition statement — D1 split is exhaustive + disjoint across D1a/D1b/D1c
along three proof families: **D1a owns append/hash-chain/replay/tamper = D1
items ①②**; D1b owns compaction/cold-tier + recovery + undo = D1 items ③④⑥;
D1c owns concurrency/perf = D1 item ⑤. ① ∪ ② ∪ ③④⑥ ∪ ⑤ = {①②③④⑤⑥}, the
full D1 set, with no item shared.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` (the frozen Day-0 type — the append-only log element).
- CoreLink CAS client surface (object put/get) for the bytes EventRecords
  reference. Consumed as a frozen external API; this WP is a CoreLink tenant,
  zero server-side changes.
- No other frozen type is mutated; D1b/D1c/D2a/D3a/D4/D5/D14/E5 consume the
  log primitives sealed here against stubs.

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-refstore/src/log/` — EventRecord append path + hash chain.
- `crates/hugit-refstore/src/replay/` — deterministic log→ref-state projection.
- `crates/hugit-refstore/src/tamper/` — chain verification + tamper detection.
- `crates/hugit-refstore/tests/log_core/` — the owned acceptance suite.
- No writes under `crates/hugit-refstore/src/{compaction,recovery,undo,concurrency}/`
  (D1b/D1c) or any other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §4 (object model), §6.5
  (event log & universal undo); `docs/plan/decomposition.md` §4 (D1 row);
  `docs/plan/warp-10-days.md` Sprint-2 D1 row.
- Anchors: the DO-per-repo append-only hash-chained log (§6.5); refs = derived
  view; `EventRecord` frozen schema from hugit-contracts.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; `Signed-off-by:` (DCO) + CHANGELOG `[Unreleased]`
  entry on feat/fix commits; cold-verify by a non-author agent.
- Token estimate: ~74k (≤80k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **One Durable Object per repo** owns the log; there is exactly one append
  point. Append is the only mutating primitive in this WP.
- **The log is append-only and hash-chained:** each `EventRecord` carries the
  hash of its predecessor; the chain is the integrity spine. Nothing is ever
  rewritten — this is fixed, not a choice.
- **Refs are a DERIVED VIEW**, never stored as primary state: ref state is the
  fold of `replay` over the log. Replay is pure and deterministic — same log →
  same ref state, byte-for-byte (this is what ① asserts at 10k events).
- **Tamper detection (②)** = chain re-verification: any altered/inserted/
  dropped record breaks the predecessor-hash chain and is detected; detection
  fails CLOSED (refuse to serve a derived view from a broken chain), never
  silently repairs.
- Event payload bytes that are large are CAS-referenced (the record holds the
  CAS key); the record itself stays small. Compaction/cold-tier of the log is
  D1b — but the record format here is designed so D1b can offload cold ranges
  without changing the chain semantics (forward-compatible by construction).
- No concurrency/throughput claims here — single-writer append correctness only;
  the 100-concurrent / p99 proof is D1c.

## DoD (global)
fmt + clippy + test + audit green · owned items (①②) red→green · cold-verify
pass by a non-author agent · DCO + CHANGELOG discipline.

## Completeness
All owned items green · zero writes outside claims · evidence bundle (replay
determinism proof at 10k events; tamper-detection proof) attached to the SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned items red→green) · evidence refs (acceptance run, replay-identity
artifact, tamper-detection artifact) · claims-respected assertion ·
deviations = none | waiver-ref.
