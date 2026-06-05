# WP-B2b — runner execution + honesty (byte-identity, non-determinism, hit-rate)
squad B · M · opus · route opus · budget 80k · branch: wp/B2b

## Charter
Build the runner-side half of checks-as-code in `hugit-checks`: forge/runner
execution that is byte-identical to the local executor, non-determinism
detection, and honest partial-hit-rate measurement on a real npm fixture. This
half depends on B2a's frozen CheckDef + memo-key contract; it proves the
execution side of the same pure function.

## Owned acceptance
**This half owns items ③④⑤ of B2** (runner execution, byte-identity,
non-determinism, honest hit-rate — the integration side; item ③ spans
local↔runner and so lands in this later/integration half). The partition is
exhaustive and disjoint: B2a = ①②⑥⑦, B2b = ③④⑤, union = B2's seven items.
VERBATIM from decomposition v2.0 §2:

③ **🔧 local≡runner BYTE-IDENTICAL (artifact/digest compare, not result-equal)**
· ④ **🔧 non-determinism flagged after 3 divergent runs**, honest surface ·
**⑤(+) npm fixture: partial hit-rate measured & displayed as-is, no full-memo
claim**

## Contract deps
Consumes from `hugit-contracts` (frozen): **CheckDef** + **CheckResult** (the
memo record whose artifacts/digests are compared byte-for-byte) and the
**RunnerLease** type for runner-side execution context. Depends on B2a's
frozen CheckDef format + memo-key derivation (build against B2a's stub). No
contract type authored or changed here.

## Claims
`crates/hugit-checks/src/runner/` (runner-side execution driver, byte-identity
comparator, non-determinism tracker, hit-rate meter) and the runner-path
module wiring in `crates/hugit-checks/src/lib.rs` (the `runner` mod only). Does
NOT touch `crates/hugit-checks/src/client/` (B2a), `affected/` (B3), or
`regen/` (C4).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B2b.md`).
- `hugit-contracts` (CheckDef, CheckResult, RunnerLease) + B2a's frozen
  CheckDef/memo-key stub.
- whitepaper §6.2 ("local `hugit check` and forge execution are the same
  function — byte-identical by construction"), §5.1 (AC ride), §5.2/§11
  (honest hit-rate; npm partial-hit honesty).
- command-catalog (memoized-checks honest scope: hermetic full, npm/pip
  partial — measured, never promised).
- The failing acceptance suite at `tests/acceptance/wp-B2b/`.
- Conventions: CoreLink AC/CAS consumed as a CLIENT; runner is the Phase-C
  fabric consumed here only as an executor surface.
Estimated packet size: ~60k tokens (inside 80k).

## Implementation notes
Every fork pre-decided:
- **Byte-identity (③):** local and runner execution invoke the SAME pure check
  function (whitepaper §6.2). The proof is an ARTIFACT/DIGEST compare — each
  produced artifact's content digest from the local run equals the runner's,
  not merely an equal exit/result. Any digest mismatch on a deterministic
  fixture = FAIL.
- **Non-determinism flagging (④):** the tracker records produced-artifact
  digests per `memo_key` across runs; after **3 divergent runs** for the same
  key the check is flagged non-deterministic and surfaced HONESTLY (a
  `non_deterministic` state on the result, not a silent re-memo). A flagged
  check is never claimed as a clean memoized hit.
- **Honest hit-rate (⑤):** run the npm fixture, measure the actual AC hit-rate,
  and display it AS-IS (partial). NO full-memoization claim for non-hermetic
  ecosystems — the behavior is ecosystem-agnostic; npm is the representative
  partial case (decomposition adjudication: pip-vs-npm honesty = B2⑤).
- **Runner surface:** execution runs under a `RunnerLease` (the Phase-C fabric
  is consumed as an executor; B2b does not build the runner — C2 does — it
  drives execution through the lease surface).
- **CoreLink consumed as CLIENT only** — zero server changes.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ③④⑤ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
Items ③④⑤ green; zero writes outside `crates/hugit-checks/src/runner/`;
evidence bundle (byte-identity digest compare, 3-run divergence flag, npm
hit-rate measurement) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, digest-compare proof,
non-determinism flag trace, npm hit-rate figure), partition note (owns ③④⑤ of
B2), deviations = none | waiver-ref.
