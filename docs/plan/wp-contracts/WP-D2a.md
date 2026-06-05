# WP-D2a — pack assembly + clone/fetch core
squad D · size L→M · model route opus · context budget 90k · branch: wp/D2a

## Charter
Build the git wire-protocol READ path over the CoreLink CAS: smart-HTTP
protocol v2 negotiation and pack assembly from CAS, serving clone and delta-only
fetch of a hugit repo. A clone is byte-identical to the mirror; a fetch
transfers only the delta. This is the projection layer — refs come from the
D1 event-log derived view; the bytes come from CAS. Client matrix, jj, CPU/
chunked fallback, degradation kill-test and scale ceilings are D2b.

## Owned acceptance (VERBATIM from decomposition v2.0 — D2)
> ① clone byte-identical to mirror
> ② delta-only fetch

(Partition statement — D2 split is exhaustive + disjoint across D2a/D2b.
**D2a owns pack assembly + clone/fetch core = D2 items ①②**; D2b owns client
matrix + jj stacks + CPU/chunked fallback + degradation kill-test + scale
ceilings = D2 items ③④⑤⑥⑦. ①② ∪ ③④⑤⑥⑦ = the full D2 set, no item shared.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- D1 (`hugit-refstore`) derived-view ref state — the refs a clone/fetch
  negotiates against. Consumed as a frozen in-workspace dependency; not modified.
- CoreLink CAS client surface (object get; chunked SplitBlob/SpliceBlob Merkle
  manifests per whitepaper §5) for pack bytes. Frozen external API; CoreLink
  tenant only, zero server-side changes.
- No other frozen type mutated; E1 (mirror) and D2b consume the read path
  sealed here.

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-proto/src/read/negotiate/` — protocol v2 ref-advertisement +
  want/have negotiation.
- `crates/hugit-proto/src/read/pack/` — pack assembly from CAS objects.
- `crates/hugit-proto/src/read/serve/` — clone + delta-only fetch entrypoints.
- `crates/hugit-proto/tests/clone_fetch_core/` — the owned acceptance suite.
- No writes under `crates/hugit-proto/src/read/{clients,fallback,limits}/` or
  `crates/hugit-proto/src/write/` (D2b / D3) or any other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §2 (git never broken),
  §4 (projection rule), §5 (CoreLink CAS substrate); `docs/plan/decomposition.md`
  §4 (D2 row); `docs/plan/warp-10-days.md` D2 row.
- Anchors: smart-HTTP protocol v2; pack assembly from CoreLink CAS via a
  libgit2-class implementation (NOT a from-scratch protocol rewrite); refs =
  D1 derived view; clone byte-identical to the mirror.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; DCO + CHANGELOG `[Unreleased]`; cold-verify by a
  non-author agent.
- Token estimate: ~84k (≤90k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **Smart-HTTP protocol v2** is the wire protocol: capability advertisement +
  ref advertisement + want/have negotiation. This is fixed, not a choice.
- **Pack assembly is from the CoreLink CAS** — objects are fetched from CAS
  (chunked Merkle manifests) and assembled into the pack the client negotiated.
  Refs are read from the D1 event-log derived view; the read path never holds
  primary ref state of its own.
- **Use a libgit2-class implementation — NOT a from-scratch protocol rewrite.**
  Negotiation and packfile encoding ride a mature git library; this WP wires it
  to CAS-backed object access, it does not reimplement git's pack format.
- **Clone byte-identical to the mirror (①):** the served repo is a valid git
  repository — git is never broken (§2); a clone produces byte-identical object
  bytes to what the GitHub mirror holds for the same refs.
- **Delta-only fetch (②):** want/have negotiation transfers only the objects the
  client lacks — no full re-send on incremental fetch.
- Client-matrix conformance (git/jj/libgit2), CPU/chunked fallback at scale,
  the smart-layer-disabled degradation kill-test, and per-dimension scale
  ceilings are explicitly OUT of this WP — they are D2b, building on this core.

## DoD (global)
fmt + clippy + test + audit green · owned items (①②) red→green · cold-verify
pass by a non-author agent · DCO + CHANGELOG discipline.

## Completeness
All owned items green · zero writes outside claims · evidence bundle
(byte-identical-clone proof vs mirror fixture; delta-only-fetch proof) attached
to the SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned items red→green) · evidence refs (acceptance run, clone-identity
artifact, delta-fetch artifact) · claims-respected assertion · deviations =
none | waiver-ref.
