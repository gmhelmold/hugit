# WP-B2a — checks client (CheckDef format + local executor + memo key)
squad B · M · opus · route opus · budget 80k · branch: wp/B2a

## Charter
Build the client side of checks-as-code in `hugit-checks`: the CheckDef format,
the local executor (`hugit check --local`), and the three-axis memo key against
the CoreLink Action Cache. This is the AC-client half of B2; B2b's runner-side
execution depends on B2a's frozen CheckDef + memo-key contract.

## Owned acceptance
**This half owns items ①②⑥⑦ of B2** (the memo-key / AC-client axes). The
partition is exhaustive and disjoint: B2a = ①②⑥⑦, B2b = ③④⑤, union = B2's
seven items. VERBATIM from decomposition v2.0 §2:

① repeat tree+def→AC hit, 0 exec, <500ms · ② glob sensitivity (in→rerun,
out→hit) · **⑥(R2) toolchain sensitivity: different toolchain → MISS, never
false hit** · **⑦(R3) def sensitivity: changed check definition, same
tree+toolchain → MISS + re-execute (3rd key axis)**

## Contract deps
Consumes from `hugit-contracts` (frozen): **CheckDef** (the def format and
`def_digest`), **CheckResult** (the memoized record + memo_key), and the
**QueueApi** types only insofar as affected-set sensitivity feeds them. No
contract type authored or changed here.

## Claims
`crates/hugit-checks/src/client/` (CheckDef parsing/validation, local executor,
memo-key derivation, AC client) and `crates/hugit-checks/src/lib.rs` module
wiring for the client path. Does NOT touch `crates/hugit-checks/src/runner/`
(B2b), `crates/hugit-checks/affected/` (B3), or `crates/hugit-checks/regen/`
(C4).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B2a.md`).
- `hugit-contracts` (CheckDef, CheckResult).
- whitepaper §6.2 (the memo key + check function — the source of truth for the
  three axes), §5.1 (AC ride), §5 substrate (AC `/v1/ac`, CAS `/v1/cas`).
- warp-10-days §Squad B (B2 deliverable; memo key `H(tree‖def‖toolchain)`).
- command-catalog (`hugit check --local`, memoized-checks honest scope).
- The failing acceptance suite at `tests/acceptance/wp-B2a/`.
- Conventions: CoreLink AC/CAS consumed as a CLIENT; `clw run` is the reference.
Estimated packet size: ~58k tokens (inside 80k).

## Implementation notes
Every fork pre-decided:
- **Memo key (load-bearing):** `key = H(tree_root ‖ check_def_digest ‖
  toolchain_digest)` (whitepaper §6.2), SHA-256 over length-prefixed
  concatenation; the three axes come straight from `hugit-contracts`. All three
  are necessary AND sufficient — any change to any axis → different key → MISS.
- **AC client (①):** lookup is `GET` against CoreLink AC `/v1/ac` keyed by
  `memo_key`; a hit returns the stored CheckResult with `0` local executions
  in `<500ms` (no runner spawn). `clw run` is the reference client — generalize
  it, do not reinvent the protocol.
- **CheckDef → def_digest:** canonical digest over the normalized definition
  body (command + inputs + toolchain_ref + glob_set), so ⑦ (changed definition,
  same tree+toolchain) yields a DIFFERENT `def_digest` → MISS + re-execute.
- **Glob sensitivity (②):** the input set is computed from CheckDef's
  `glob_set`; a tree edit INSIDE the glob changes `tree_root` over the
  check's input subtree → rerun; an edit OUTSIDE leaves it unchanged → hit.
  (The affected-set computation itself is B3's claim; B2a consumes its result
  to scope the tree hash.)
- **Toolchain sensitivity (⑥):** `toolchain_digest` is a real input axis; a
  different toolchain → different digest → MISS. NEVER a false hit (a false
  hit here is a correctness violation, fail-closed).
- **Local executor (`hugit check --local`):** the SAME pure function the runner
  uses — but B2a only owns the client/local path; the runner-side execution +
  byte-identity proof are B2b. Locally, on a MISS, the executor runs the check
  and the result is stored to the AC under `memo_key`.
- **CoreLink consumed as CLIENT only** — zero server changes.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①②⑥⑦ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
Items ①②⑥⑦ green; zero writes outside `crates/hugit-checks/src/client/`;
evidence bundle (AC hit/miss traces, memo-key derivations for each axis,
<500ms timing) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, per-axis MISS/HIT proofs,
timing), partition note (owns ①②⑥⑦ of B2), deviations = none | waiver-ref.
