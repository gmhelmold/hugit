# WP-E1a — verified mirror: outbound sync + hash verify + ordering/queue

squad E · M · opus · 80k · branch: wp/E1a

## Charter
Build the one-way hugit→GitHub mirror **happy path**: every landed ref
replicates to the GitHub mirror, hash-verified to byte-identity within SLA,
and holds verified under sustained soak. The outbound writer is fed by a
durable, capacity-bounded queue whose ordering is preserved. This is the
soak instrument whose uptime IS the trust metric; failure modes (outage,
divergence, one-way enforcement) are E1b, bootstrap+DR are E1c.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E1; this WP owns items ① ③ ⑩.)*

- **①** landing on GitHub <60s hash-verified
- **③** 72h soak 100% verified
- **⑩ 🔧** the outage queue (④) has a stated capacity bound; overflow →
  backpressure + incident, never drop

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `EventRecord` — the landed-ref events this WP mirrors (the source of truth
  for what must replicate, in what order).
- `AppWebhooks` / GitHub App auth surface — the authenticated push channel to
  the GitHub mirror.
- Read path from **D2** (`hugit-proto/read`): pack/object materialization the
  mirror writer pushes from. Consumed as a frozen producer; not modified here.
- The durable-queue state-machine shape is **defined here** and consumed by
  E1b④ (which exercises overflow/outage against it); E1b owns the failure
  semantics, E1a owns the capacity bound (⑩) and ordering guarantee.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-mirror/src/outbound/` — the outbound sync writer, push driver,
  GitHub App auth client.
- `crates/hugit-mirror/src/verify/` — content-hash verification (per-push
  byte-identity check against the just-pushed ref).
- `crates/hugit-mirror/src/queue/` — the durable outage queue core (ordered,
  capacity-bounded, backpressure on overflow). *Failure-injection tests* over
  this queue are E1b's claim; the queue type + capacity bound are E1a's.
- `tests/mirror/outbound_*.rs`, `tests/mirror/soak_72h_*.rs`,
  `tests/mirror/queue_capacity_*.rs`.

## Dispatch packet
- This contract file.
- Frozen `EventRecord`, `AppWebhooks`, GitHub-App-auth anchors from
  `hugit-contracts`.
- D2 read-path producer signature (clone/fetch object materialization).
- Anchor: `crates/hugit-mirror/lib.rs` module barrel exporting `outbound`,
  `verify`, `queue`.
- Conventions: fail-CLOSED on any verify mismatch (raise divergence to E1b's
  alarm seam, never mark synced); all GitHub calls via the App auth client;
  every push carries the content-hash it expects to observe post-push.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Auth = GitHub App** (installation token, not PAT, not OAuth). Token minted
  per-push from the App's installation; rotation handled by the App layer.
- **Content-hash verification PER PUSH**: after each push, re-read the mirror
  ref and compare its object hash to the locally-computed expected hash; a
  mismatch is a hard fail-CLOSED → divergence signal (the alarm/repair is
  E1b's, the detection is here). Verification is per-push, not batched.
- **Durable outage queue with stated capacity bound**: bounded FIFO, persisted
  (survives writer restart), strict landing-order preservation. The capacity
  bound is a **stated constant** (config-pinned, documented in the SEAL). On
  overflow → **backpressure to the producer + incident event**, NEVER drop and
  NEVER reorder. The drain semantics under outage (bounded backoff, recovery
  drain) are E1b④'s to exercise; the queue's ordering + capacity invariant is
  proven here.
- **<60s SLA** measured land-event→verified-on-GitHub; the soak (③) asserts
  100% verified over 72h of continuous real landings (dogfood waves).
- One-way only: this writer never reads GitHub state as truth (E1b⑦ proves
  reverse writes are divergence).

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` all green on `wp/E1a`.
- Owned items ① ③ ⑩ go red→green with committed failing suites BEFORE
  implementation.
- Cold verification by a non-author agent; security review at SEAL.
- DCO `Signed-off-by:` + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (① ③ ⑩) green.
- Zero writes outside the Claims paths.
- Evidence bundle (72h soak log, per-push verify traces, queue capacity/
  overflow-backpressure proof) attached to the SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs (soak report, verify traces, queue
proof) · items ①③⑩ red→green · deviations = none | waiver-ref.
