# CLASS 6 — ERROR-LAW — Round 9 re-audit

Round 9 · fresh-context convergence re-audit on the integrated state
(`integ/wave-l` @ HEAD `95353f9`, Wave L / L-C).
Auditor: cold-context, default-refuse (uncertain = hole until proven).
Baseline: `docs/review/round8/class-6-errorlaw.md` (Round-8 found 2 real holes:
F-1 string-sniffed AC retryability, F-2 clap arg errors bypass the envelope).

## 1. Scope & method

Class 6 = (A) error law — every error-emitting CLI path emits the canonical
`{"error":{"kind","message","fix",…}}` envelope on **stdout** + exit **2**
(exit 1 only for `internal`); (B) concurrency — retryable taxonomy preserved
as a TYPE + every multi-write atomic; (C) determinism — byte-identical claims hold.

Method: (1) `cargo build -p hugit-cli --locked` → green (0.20s, no warnings).
(2) Re-enumerated EVERY error emission in `crates/hugit-cli/src/`
(`eprintln!`/`bail!`/`process::exit`/`panic!`/`unwrap`/`expect`/`Err(`-print)
and clap parse. (3) LIVE-repro'd the two Round-8 holes against the built binary.
(4) Re-read `map_exec_error` + `AcError` + the HTTP/lock-lock boundary for the
typed-retryability fix. (5) STRESS-ran the Round-7 flaky test ×10 (bare exit).
(6) Audited atomicity of log-append / `.ac`-write / store-commit. (7) Grepped
determinism hazards (HashMap iteration in non-test code). Temp work outside repo.

## 2. Error-path matrix re-verification (clap envelope + typed taxonomy closed?)

**Hole F-2 (clap → envelope) — CLOSED.** `main.rs:453` is now `Cli::try_parse()`
with an `Err(e)` arm at the choke-point: `DisplayHelp`/`DisplayVersion`/
`DisplayHelpOnMissingArgumentOrSubcommand` → print verbatim to **stdout**, exit 0;
every genuine arg/subcommand/value-parse error → `PorcelainError::new(
"invalid_arguments", <clap msg, newlines flattened>, "run `hugit --help` …")`
on **stdout**, exit 2. Live repros (built binary):

| Invocation | stdout | exit | stderr |
|---|---|---|---|
| `hugit frobnicate` | `{"error":{…"kind":"invalid_arguments"…}}` | 2 | **0 bytes** |
| `hugit verdict --bogus` | `…"kind":"invalid_arguments"…` | 2 | empty |
| `hugit why` (missing required) | `…"kind":"invalid_arguments"…` | 2 | empty |
| `hugit check --timeout-secs notanum …` (value-parse) | `…"invalid_arguments"…` | 2 | empty |
| `hugit --help` | help text | 0 | — |
| `hugit --version` | `hugit 0.1.0` (stdout-only) | 0 | — |
| `hugit` (bare, no subcommand) | help text | 0 | empty |

The whole sub-class (arg / subcommand / value-parse, every verb) now obeys the law.

**Hole F-1 (string-sniffed AC retryability) — CLOSED, cured not patched.**
- `AcError::Busy { detail }` is now a TYPED variant (`ac.rs:42`), Display'd as
  "AC busy (retryable)" (`ac.rs:84`).
- Produced at all three retryable sources: lock-exhaustion `acquire_ac_lock`
  (`run.rs:988`), HTTP `lookup` 429|503 (`ac.rs:499`), HTTP `store` 429|503
  (`ac.rs:521`).
- `map_exec_error` (`run.rs:1338`) matches `ExecError::Ac(AcError::Busy{..}) =>
  "ac_busy"` as a TYPED arm; all other AcError variants are an EXHAUSTIVE
  explicit list (`NotWired|Transport|NotConfigured|Status|Decode|
  DigestMismatch|InvalidKey`) → terminal `ac_error`. No wildcard ⇒ a future
  variant fails to compile until consciously classified.
- The `starts_with("ac_busy:")` string-sniff is DELETED from all production code
  (grep: only doc-comment mentions remain, `run.rs:1330/1342/1495`).
- Tests assert the full matrix: `Busy→ac_busy`, `429/503→ac_busy`,
  `Status(401)→ac_error`, `Transport→ac_error` (`run.rs:1491–1527`) — all pass.

Other paths re-confirmed unchanged-good: legacy verbs `main.rs:511–522`
`println!{to_json}`; campaign/intent/pr/checks/verdict/queue all emit envelope
on stdout exit 2; `map_lock_error` typed `LockError::Busy→log_busy` (`run.rs:1308`).
NO `eprintln!`/`bail!`/`process::exit(1)`/`panic!` in non-test CLI code; every
`unwrap`/`expect` hit is inside a `#[cfg(test)]` module (verified).

## 3. Break attempts + 10/10 stress + atomicity

**Bare-error / panic / mis-taxonomy hunt — no NEW terminal hole found:**
- clap value-parse error (not just arg/subcommand) → still routes through
  `try_parse` Err arm → `invalid_arguments` (repro'd, T6). No leak.
- `--version` confirmed stdout-only exit 0 (not stderr). `--help` exit 0.
- Bare `hugit` (no subcommand) → help on stdout, exit **0**, empty stderr. This
  is clap's `DisplayHelpOnMissingArgumentOrSubcommand`, treated as success by
  design — debatable (an orchestrator firing `hugit` with no verb gets exit 0,
  no envelope) but it is clap's standard contract and the Round-8 remediation
  explicitly scoped this kind to the exit-0 stdout path. Not a regression, not a
  terminal-mis-taxonomy. Noted as residual R-1 (honesty, not a defect).
- AC store-write IO fault → `AcError::Transport` → terminal `ac_error`
  (`run.rs:952`): correct — a write IO fault is terminal, not retryable.
- Retryable-collapse hunt: every retryable source (lock-exhaustion + HTTP
  429/503) is typed `Busy`; the ONE remaining terminal-classified-but-arguably-
  retryable case is a ureq network-level error (connect/read **timeout**, DNS,
  refused) → `AcError::Transport` → `ac_error` (`ac.rs:555/568`). See R-2 below —
  this is the disclosed P2 HTTP seam, out of the Round-8 F-1 fix scope (429/503
  only), hermetic-unreachable locally; LOW/disclosed, not a convergence blocker.

**10/10 stress (the Round-7 flaky test):**
`cargo test -p hugit-cli --locked concurrent_checks_do_not_double_record_even_if_both_execute -- --exact`
×10, bare exit each: **RUN1..RUN10 all = 0 → PASS=10/10.**

**Atomicity — all multi-writes atomic-by-construction (re-verified):**
- log append (`record_on_log` run.rs:1208): holds `FileLock` across the FULL
  load→dedup-scan→append→persist; dedup-under-lock provably kills the
  double-record; `persist_log` uses `atomic_write`.
- `.ac` store (`run.rs:942`): holds `acquire_ac_lock` across read-map→
  insert→serialize→`atomic_write` (the whole read-modify-write).
- `atomic_write` (`filelock.rs:187`): `File::create(.{name}.tmp-{pid}-{nanos})`
  → `write_all` → `sync_all` → same-dir `rename` → cleanup-on-fail. Crash-safe,
  collision-safe. No torn-write / TOCTOU window found.

**Determinism:** NO `HashMap` in non-test CLI code (grep clean). Canonical/sorted
serialization claims (`canonical_json`, `BTreeMap` tree snapshot, sorted env-axis
manifest) unchanged from Round-8 and hold. No float-fmt / locale hazard surfaced.

## 4. Residual findings (sev · repro · TYPE)

- **R-1 · LOW · honesty · TYPE: disclosed-design.** Bare `hugit` (and any verb
  whose only failure is missing-subcommand) exits 0 with help on stdout, no
  envelope. An orchestrator invoking with no verb gets a non-error exit. This is
  clap's `DisplayHelpOnMissingArgumentOrSubcommand` contract, accepted by the
  Round-8 remediation. Not a regression; flagged for honesty only.
- **R-2 · LOW (latent at P2) · disclosed seam · TYPE: scope.** ureq network-level
  errors (connect/read timeout, DNS, refused) → `AcError::Transport` → terminal
  `ac_error` (`ac.rs:555/568`). A transient timeout under fleet contention is
  arguably retryable, yet classifies terminal. OUT of the Round-8 F-1 fix scope
  (which mandated only HTTP 429/503 + lock-exhaustion as `Busy`), and
  hermetic-unreachable until the live AC seam lights up at P2. Tracked under the
  disclosed PS-8/PS-12 live-AC seam. Recommend (P2, non-blocking): classify
  ureq `Transport`/`Io` timeout kinds as `AcError::Busy` at the ureq boundary.

No HIGH/MEDIUM finding. Both Round-8 holes (F-1, F-2) are independently closed,
live-verified. F-3 (duplicate PorcelainError render paths) remains a LOW
maintainability note, unchanged, not in scope for this class verdict.

## 5. CONVERGENCE VERDICT

**CONVERGED.** Every CLI error path emits the structured envelope on stdout with
exit 2 (exit 1 only for `internal`); the two Round-8 holes are closed and
live-repro'd shut — clap arg/subcommand/value-parse errors now render the
`invalid_arguments` envelope exit 2, and retryability is a TYPE
(`AcError::Busy`) matched in an EXHAUSTIVE compiler-enforced `map_exec_error`
with the string-sniff deleted. 10/10 stress on the Round-7 flaky test. All
multi-writes atomic-by-construction (lock-held read-modify-write +
temp/fsync/rename). Determinism claims hold (no HashMap iteration, canonical
serialization). The ONLY residuals are the disclosed P2 HTTP-busy seam
(R-2: ureq network timeout → terminal, untested locally, out of F-1 scope) and
the accepted clap missing-subcommand→exit-0 contract (R-1) — neither a
bare-error, mis-taxonomy, or torn-write reproducible on the live local path.
