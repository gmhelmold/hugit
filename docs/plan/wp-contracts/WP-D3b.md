# WP-D3b — push concurrency + total order + external-change + flag/negatives
squad D · size M · model route opus · context budget 70k · branch: wp/D3b

## Charter
Prove the write path under concurrency and at its boundaries: concurrent pushes
get a total order with correct stale rejection, a raw push is recorded as an
opaque external-change event with attribution (NEVER a fabricated intent), and
the write path is flag-gated off unless self-hosted-alpha. Carries the dedicated
D3 red-team pass — write path = source-of-truth bar. Builds on D3a's
receive-pack→CAS+log core.

## Owned acceptance (VERBATIM from decomposition v2.0 — D3)
> ② concurrent pushes: total order, correct stale rejection
> ③ raw push = external-change w/ attribution
> ④ flag off unless self-hosted-alpha
> ⑤(+) negative: NO synthetic intent fabricated for a raw push (intent log clean)

(Partition statement — D3 split is exhaustive + disjoint across D3a/D3b.
**D3b owns concurrency/total order + external-change + flag/negatives = D3
items ②③④⑤**; D3a owns receive-pack→CAS+log = ①. The spanning "prove the write
path" concern lands in this LATER half per the binding split rule. ① ∪ ②③④⑤ =
the full D3 set, no item shared.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- D3a's sealed receive-pack→CAS+log core — consumed as the in-crate substrate
  this WP exercises; not modified.
- D1 (`hugit-refstore`) append path + `EventRecord` (the external-change event
  is a D1 event; the single-writer serialization point provides total order).
  Consumed frozen.
- The intent corpus surface (B6/D4) for the ⑤ "intent log clean" negative —
  consumed read-only; this WP asserts ABSENCE of fabricated intents, writes none.

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-proto/src/write/order/` — concurrent-push total-order +
  stale-rejection.
- `crates/hugit-proto/src/write/external/` — raw-push → opaque external-change
  event with attribution.
- `crates/hugit-proto/src/write/flag/` — self-hosted-alpha flag gate.
- `crates/hugit-proto/tests/push_concurrency_negatives/` — owned acceptance
  suite + the D3 red-team write-path concurrency/negative fixtures.
- No writes under `crates/hugit-proto/src/write/{receive,store}/` (D3a),
  `crates/hugit-proto/src/read/` (D2), `crates/hugit-refstore/src/intent/`
  (D4), or any other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §4 (projection rule),
  §6.5; `docs/product/command-catalog.md` (raw `git push` = opaque change-events,
  never reverse-engineered into fake intents); `docs/plan/decomposition.md`
  §4 (D3 row) + §6/§8 (the no-fake-intents adjudication: D3⑤+D4④+E2⑤) + routing
  law (dedicated red-team on D3); `docs/plan/warp-10-days.md` D3 row + D10 SEAL.
- Anchors: concurrent total order via the D1 single-writer point; raw push =
  external-change with attribution; flag off unless self-hosted-alpha; zero
  synthetic intents fabricated for raw pushes.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; DCO + CHANGELOG `[Unreleased]`; cold-verify by a
  non-author agent; **dedicated red-team pass (write path = source-of-truth
  bar)**.
- Token estimate: ~66k (≤70k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **Total order + stale rejection (②):** concurrent pushes are serialized
  through D1's single-writer append point — that IS the total order. A push
  against a stale ref tip is correctly rejected (compare-and-append against the
  derived ref view); no lost update, no false accept.
- **Raw push = external-change WITH attribution (③):** a raw `git push` from a
  compatibility user is recorded as an OPAQUE external-change event carrying its
  attribution (who/when/ref). It is never interpreted as, or projected into, an
  intent — externals stay external.
- **NO synthetic intent (⑤, the negative):** the write path NEVER fabricates an
  intent for a raw push; after a raw-push fixture the intent log is provably
  clean (asserted absence). This is one leg of the on-record no-fake-intents
  adjudication (D3⑤ + D4④ + E2⑤) — do not synthesize, do not reverse-engineer.
- **Flag gate (④):** the write path is OFF by default and enabled only under the
  self-hosted-alpha flag; the gate is asserted (flag off ⇒ write path absent/
  refused).
- **Red-team (this WP):** the dedicated D3 red-team rides here — concurrent-push
  races, stale-tip forgery, attribution spoofing, and intent-fabrication
  attempts are part of the owned suite. Write path = source-of-truth bar.

## DoD (global)
fmt + clippy + test + audit green · owned items (②③④⑤) red→green · cold-verify
pass by a non-author agent · **dedicated red-team pass (write path =
source-of-truth bar)** · DCO + CHANGELOG discipline.

## Completeness
All owned items green · zero writes outside claims · evidence bundle
(concurrent total-order + stale-rejection proof; external-change-with-
attribution artifact; flag-gate assertion; clean-intent-log negative proof;
red-team report) attached to the SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned items red→green) · evidence refs (acceptance run, total-order
artifact, external-change artifact, flag-gate artifact, clean-intent-log proof,
red-team report) · claims-respected assertion · deviations = none | waiver-ref.
