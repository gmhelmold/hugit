# WP-D3a — receive-pack → CAS + log (push core)
squad D · size M · model route opus · context budget 80k · branch: wp/D3a

## Charter
Build the git wire-protocol WRITE path core: receive-pack ingests a push,
stores objects in the CoreLink CAS, and records the ref update as an
append-only event on the D1 log. A push then clones back identical. This is
self-hosted-flagged write — the source-of-truth bar — so it carries the
dedicated D3 red-team pass. Concurrency/total-order, external-change attribution
and the flag/negatives are D3b.

## Owned acceptance (VERBATIM from decomposition v2.0 — D3)
> ① push→clone round-trip identical

(Partition statement — D3 split is exhaustive + disjoint across D3a/D3b.
**D3a owns receive-pack→CAS+log = D3 item ①**; D3b owns concurrency/total order
+ external-change + flag/negatives = D3 items ②③④⑤. ① ∪ ②③④⑤ = the full D3
set, no item shared.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- D1 (`hugit-refstore`) append path + `EventRecord` — ref updates are recorded
  as events on the D1 log. Consumed frozen; not modified.
- D2a read-path serve — used to clone back for the round-trip assertion.
  Consumed frozen.
- CoreLink CAS client surface (object put) for pushed object bytes. Frozen
  external API; CoreLink tenant only, zero server-side changes.

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-proto/src/write/receive/` — receive-pack ingest.
- `crates/hugit-proto/src/write/store/` — object → CAS persistence + ref-update
  → D1 event append.
- `crates/hugit-proto/tests/push_core/` — owned acceptance suite + the D3
  red-team write-path fixtures.
- No writes under `crates/hugit-proto/src/write/{order,external,flag}/` (D3b),
  `crates/hugit-proto/src/read/` (D2), or any other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §4 (object model), §6.5
  (event log); `docs/plan/decomposition.md` §4 (D3 row) + §8 (routing law —
  dedicated red-team on D3); `docs/plan/warp-10-days.md` D3 row + D10 final-SEAL
  ("security review including D3's write path — the source-of-truth bar").
- Anchors: receive-pack stores to CAS; every ref update is an append-only D1
  event; libgit2-class implementation (NOT a from-scratch rewrite); a push
  clones back byte-identical.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; DCO + CHANGELOG `[Unreleased]`; cold-verify by a
  non-author agent; **dedicated red-team pass (write path = source-of-truth
  bar)**.
- Token estimate: ~76k (≤80k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **Receive-pack → CAS + log:** the push write path ingests objects into the
  CoreLink CAS and records the ref move as an APPEND-ONLY event on the D1 log.
  There is no primary ref state outside the log — the ref update IS an event;
  refs stay a derived view.
- **Use a libgit2-class implementation — NOT a from-scratch protocol rewrite**
  (symmetric to D2a). receive-pack negotiation/unpack rides a mature git
  library; this WP wires it to CAS persistence + D1 append.
- **push→clone round-trip identical (①):** objects pushed are clonable back
  byte-identical via the D2a read path — git is never broken; the write path
  produces a valid git repository.
- The write path is **self-hosted-flagged** (the flag gate itself is asserted in
  D3b ④); D3a builds the ingest+store core so the round-trip holds under the
  flag.
- External-change attribution for raw pushes, the no-synthetic-intent negative,
  concurrent total-order/stale-rejection, and the flag-off default are OUT of
  this WP — they are D3b.
- **Red-team (this WP):** the write-path red-team fixtures (malformed/oversized
  pack, object-injection, ref-update tampering) are part of the owned suite —
  the source-of-truth bar applies here, not only at D3b.

## DoD (global)
fmt + clippy + test + audit green · owned item (①) red→green · cold-verify pass
by a non-author agent · **dedicated red-team pass (write path = source-of-truth
bar)** · DCO + CHANGELOG discipline.

## Completeness
Owned item green · zero writes outside claims · evidence bundle (push→clone
round-trip identity proof; write-path red-team report) attached to the SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned item red→green) · evidence refs (acceptance run, round-trip
artifact, red-team report) · claims-respected assertion · deviations = none |
waiver-ref.
