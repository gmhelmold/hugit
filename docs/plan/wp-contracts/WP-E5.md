# WP-E5 — export + exit proofs (the anti-lock-in guarantee)

squad E · M · opus · 80k · branch: wp/E5

## Charter
The executable anti-lock-in promise: one command dumps git + a documented JSON
that validates against a versioned `ExportSchema`, round-trips on restore,
applies redaction, streams multi-GB without OOM, and — THE EXIT PROOF — the
exported artifact (and the mirror) is fully usable with ZERO hugit/forge
tooling present. Export holds on terminating accounts and under live concurrent
mutation.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E5; this WP owns items ① ② ③ ④ ⑤ ⑥ ⑦ ⑧ ⑨.)*

- **①** one-command dump git + documented JSON
- **②** restore round-trip reproduces refs+intents+events
- **③(🔧 was doc-presence)** export validates against versioned ExportSchema
  (machine check)
- **④(+)** export applies context/journal redaction policy (no secret material
  emitted); multi-GB export streams without OOM
- **⑤(R2)** THE EXIT PROOF: the exported git artifact (and the mirror) is fully
  usable with ZERO hugit/forge dependency — clone/log/branch/push-elsewhere all
  work with no hugit tooling present
- **⑥(R3)** completeness: export+restore reproduces ALL first-class object
  classes (refs, intents, events, ledger, verdicts, journals, policy,
  provenance links) object-for-object; any out-of-scope class explicitly
  enumerated in the ExportSchema
- **⑦(R3)** redaction red-team: seeded secrets appear NOWHERE in the exported
  git/JSON; AND the redacted artifact still passes the exit proof, removals
  manifested
- **⑧(R3)** exit under exit conditions: export succeeds, complete and valid, on
  a suspended / past-due / offboarding account (read-only terminating path)
- **⑨(R4)** "any moment" consistency: export on a LIVE account under concurrent
  mutation (landings + mirror sync + event append) yields ONE point-in-time-
  consistent cut — no dangling provenance link, no event referencing an absent
  object; restore is self-consistent

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `ExportSchema` (versioned, machine-validatable) — the export's contract;
  validation (③) and the out-of-scope enumeration (⑥) ride it.
- `EventRecord`, intent shape, `VerdictObject`, ledger/journal/policy/provenance
  object shapes — the first-class classes reproduced object-for-object (⑥).
- **D1** event-log/refstore (`hugit-refstore`): the point-in-time-consistent cut
  (⑨) reads a snapshot of the log. Consumed; not modified.
- **E1** mirror — the exported mirror is part of the exit proof (⑤). Consumed.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-cli/src/export/` — the export command, streaming dumper,
  `ExportSchema` validator, redaction-at-export, point-in-time-cut reader,
  restore round-trip, terminating-account path.
- `tests/export/roundtrip_*.rs`, `tests/export/schema_validate_*.rs`,
  `tests/export/redaction_redteam_*.rs`, `tests/export/exit_proof_*.rs`,
  `tests/export/terminating_account_*.rs`, `tests/export/live_consistency_*.rs`.

## Dispatch packet
- This contract file.
- Frozen `ExportSchema` + all first-class object-class shapes.
- D1 snapshot/cut signature; E1 mirror handle (for the exit proof).
- Anchor: `crates/hugit-cli/lib.rs` barrel exports `export`.
- Conventions: export is **streaming** (no full-corpus-in-RAM); the JSON
  **validates against the versioned `ExportSchema`** by machine check; the exit
  proof runs the artifact with **NO hugit tooling on PATH**.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Versioned `ExportSchema` (③⑥)**: the JSON dump declares its schema version
  and **validates by machine check**; ALL first-class classes (refs, intents,
  events, ledger, verdicts, journals, policy, provenance links) are reproduced
  **object-for-object**; any out-of-scope class is **explicitly enumerated in
  the schema** — no silent omission.
- **Streaming (④)**: multi-GB export streams to disk **without OOM** (bounded
  memory, chunked writer).
- **Redaction at export (④⑦)**: the context/journal redaction policy is applied
  **at export time**; seeded secrets appear **nowhere** in git or JSON; the
  redacted artifact **still passes the exit proof**; redactions are
  **manifested** (the removal is recorded, not silent).
- **THE EXIT PROOF (⑤)** — the load-bearing guarantee: the exported git artifact
  AND the mirror are fully usable with **ZERO hugit/forge dependency**. The
  proof runs in an environment with **no hugit tooling present** and asserts
  `clone` / `log` / `branch` / `push-elsewhere` all work. This is the exit
  promise made executable.
- **Terminating accounts (⑧)**: export succeeds, complete and valid, on a
  **suspended / past-due / offboarding** account via a **read-only terminating
  path** — exit is never blocked by account state.
- **Live consistency (⑨)**: export on a LIVE account under concurrent mutation
  (landings + mirror sync + event append) reads **ONE point-in-time-consistent
  cut** from D1's snapshot — **no dangling provenance link, no event
  referencing an absent object**; restore is self-consistent (cf. X14 deep-link
  integrity).

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` green on `wp/E5`.
- Owned items ① ② ③ ④ ⑤ ⑥ ⑦ ⑧ ⑨ red→green; failing suites committed first.
- Cold verification by non-author; **redaction red-team** at SEAL.
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (①–⑨) green.
- Zero writes outside Claims.
- Evidence bundle (one-command dump + schema-validate proof, object-for-object
  round-trip, redaction red-team trace, **zero-hugit-tooling exit proof**,
  terminating-account proof, live point-in-time-cut consistency proof) attached
  to SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs · items ①②③④⑤⑥⑦⑧⑨ red→green ·
deviations = none | waiver-ref.
