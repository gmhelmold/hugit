# Professional Evidence Report Research

Date: 2026-09-13. Input: approved CLI-local feature ledger at baseline
`6f9bbfa`. Purpose: decide how to measure, retain, document, and present hugit
without hype or unsupported inference.

## Decision

Publish **Hugit Reproducible Evidence Report**, not one blended “feature
benchmark.” Keep three result classes separate:

1. semantic conformance: did exact state transition happen;
2. robustness: did negative, corruption, interruption, and mutation cases fail
   correctly;
3. performance: distribution for narrowly named operations under disclosed
   cache/environment state.

Command exit is captured diagnostic data, never feature evidence. Every semantic
result needs independent oracle over resulting bytes/state. Every published run
retains full test repository, raw command streams, oracle outputs, environment,
and complete package manifest.

## Research Basis

- ACM SIGPLAN empirical guidance: evaluation form must fit claim; checklist is
  judgment support, not box ticking.
  <https://www.sigplan.org/Resources/EmpiricalEvaluation/>
- ACM SIGSOFT empirical standards: define quality, metric, method, workload, and
  context; retain raw results; assess repetition/stability and construct validity.
  <https://www2.sigsoft.org/EmpiricalStandards/docs/standards?standard=GeneralStandard>
- SPEC CPU run rules: validate outputs, repeat measurements, disclose all
  performance-relevant conditions, and keep result scoped to observed system.
  SPEC repetition formula is not copied as universal law.
  <https://www.spec.org/cpu2017/Docs/runrules.html>
- MLPerf policies: correctness/accuracy qualification and performance reporting
  are separate; submission includes system metadata, implementation, scripts,
  logs, and reproduction instructions. ML workload rules are not copied blindly.
  <https://github.com/mlcommons/policies/tree/45b8b625425295cdde53c4b969149ea6c4c4d084>
- Hyperfine: warmup, per-run preparation, cache-state disclosure, raw JSON/CSV,
  outlier visibility, and shell-free mode for short commands.
  <https://github.com/sharkdp/hyperfine/tree/f12f3d9f86f3643b3b7deace5e160b1f0f44d2b7>
- Criterion.rs: warmup, sampled measurement, bootstrap confidence intervals,
  outlier classification, and explicit noise thresholds. Appropriate for
  in-process kernels, not full Git/hook journeys.
  <https://bheisler.github.io/criterion.rs/book/analysis.html>
- Reproducible Builds: reproducibility requires same source, relevant build
  environment, instructions, and bit-identical specified artifacts. Semantic run
  outputs may vary; deterministic archive serialization is separate claim.
  <https://reproducible-builds.org/docs/definition/>
- BagIt RFC 8493: payload manifests enumerate every payload file exactly once;
  valid bag is complete and every listed checksum verifies. Fixity protects
  against corruption, not active attacker.
  <https://www.rfc-editor.org/rfc/rfc8493.html>
- W3C PROV supplies useful entity/activity/agent vocabulary, but full PROV
  conformance would duplicate hugit schema without current need.
  <https://www.w3.org/TR/prov-dm/>
- in-toto statements can bind typed predicate to artifact digest. Optional CI
  attestation proves publisher/build origin, not benchmark truth.
  <https://github.com/in-toto/attestation/blob/2dcd055e9f72e746687c306e35f4e59720ff45be/spec/v1/statement.md>
- RO-Crate describes research objects and provenance but explicitly need not be
  complete file inventory. Optional discovery metadata, not core fixity layer.
  <https://www.researchobject.org/ro-crate/specification/1.3/>
- ripgrep benchmark publication models expert honesty: user task first,
  equivalent visible work, environment/version disclosure, raw results, explicit
  bias, intentionally unfair cases labelled, and anti-pitch/limitations.
  <https://blog.burntsushi.net/ripgrep/>
- Git bundle transports reachable objects/refs but omits worktree, index, hooks,
  config, and other local state. Supplemental only.
  <https://git-scm.com/docs/git-bundle>

Primary-source URLs above were independently spot-checked during synthesis.
Mutable repository URLs must be commit-pinned when publishing final report.

## Measurement Architecture

Four independent layers:

1. Fixture builder uses stock Git and deterministic inputs.
2. Black-box driver invokes selected release binary and real Git hooks; records
   argv array, cwd, allowed environment, stdin identity, stdout/stderr bytes,
   status, signal, and monotonic duration.
3. Independent semantic oracle parses files without calling hugit implementation
   modules. Stock Git validates commits, refs, changed paths, blame, worktrees,
   and exported repository usability.
4. Performance runner starts only after semantic qualification. It retains every
   raw sample and cannot change semantic verdict.

Required controls:

- known valid artifact proves oracle can see;
- mutated artifact proves oracle rejects;
- missing artifact and empty selected-claim set hard fail;
- check subprocess writes external sentinel: cold miss increments, warm hit does
  not, changed axis increments again;
- known connector calibrates network detector; hugit local scenario must produce
  zero observed attempts under enforced denial;
- stock Git/no hook and stock Git/no-op hook separate Git baseline from process
  launch floor;
- async hook measurement reports foreground Git latency separately from
  receipt-to-canonical-event quiescence.

No fixed magic sample count. Pilot measures variance; run count follows declared
precision/effect target. Report median, enough-sample p95, bootstrap 95% interval,
sample count, min/max, failures, and all observations. Keep outliers in primary
result; annotate interference and optionally publish sensitivity view.

## Evidence Package

Use BagIt 1.0 as core package completeness/fixity layer plus hugit-specific
schemas. Deterministic tar preserves full repository filesystem. Optional
RO-Crate metadata improves discovery. Optional signed in-toto statement binds CI
publisher to final archive digest. SPDX, CycloneDX, OCI, and Merkle trees are not
core requirements.

```text
hugit-evidence-<suite>-<run-id>/
├── bagit.txt
├── bag-info.txt
├── manifest-sha256.txt
├── tagmanifest-sha256.txt
├── README.md
├── schema/
└── data/
    ├── report.json
    ├── methodology.md
    ├── limitations.md
    ├── source.json
    ├── binary.json
    ├── environment.json
    ├── claims.json
    ├── commands/
    ├── assertions/
    ├── performance/
    └── repository.tar
```

`repository.tar` includes full working tree and `.git`: config, HEAD, index,
hooks, objects, refs, reflogs, and `.git/hugit` runtime state. Supplemental Git
bundle may be included but cannot replace archive.

Bag rules:

- every regular payload file appears exactly once in `manifest-sha256.txt`;
- tag manifest covers tag files and payload manifest, never itself;
- paths are relative, slash-separated, traversal-safe, unique, and sorted under
  stable byte ordering;
- archive records modes and symlink targets without following links; reject
  devices, sockets, FIFOs, escaping links, external Git alternates, duplicate
  names, and normalization/case collisions;
- final archive gets external SHA-256; signature/attestation optional;
- hashes prove byte fixity, not author, execution truth, or fair selection.

Secret policy: synthetic secret-free run. Record environment allowlist only. Scan
before packaging; phrase result “no configured detector matched,” never “no
secrets exist.” Do not redact raw evidence after run.

## Verifier

`verify inspect` performs no benchmark execution:

1. safe extraction;
2. BagIt completeness and checksum verification;
3. schema-major validation;
4. selected-claim/assertion exact-set equality;
5. evidence-path existence and digest verification;
6. repository archive inventory, modes, links, and regular-file hashes;
7. `git fsck --full`, refs, hook bytes/modes, runtime-log integrity chain,
   sidecar identity, and semantic oracle recomputation;
8. report aggregate recomputation from assertions.

Unknown schema major, missing/duplicate claim, skipped required assertion,
missing/unmanifested file, hash mismatch, unsafe archive node, or empty selected
set is failure.

Verifier teeth suite mutates payload byte, removes/adds file, changes hook mode,
changes link target, replaces Git object, flips assertion, removes/duplicates
claim, and adds traversal member. Each mutation must fail named check.

## Presentation

Truthful umbrella headline:

> Hugit records a local Git provenance journey and exposes resulting evidence
> through reproducible CLI observations.

Release headline only after verified run:

> Hugit vX.Y.Z: reproducible local journey from Git commit hook to
> integrity-checked provenance.

README/public page order:

1. one falsifiable sentence;
2. annotated journey diagram;
3. strongest claim table linking directly to artifact/oracle;
4. “what this does not prove” beside results;
5. one reproduction command;
6. methodology, raw package, verifier, and review links;
7. performance distributions only after correctness qualification.

Visual legend:

- blue: Git-derived fact;
- green: independently observed resulting state;
- amber: caller-supplied assertion;
- hatched purple: simulator;
- gray: unsupported/discontinued scope.

Never use green for exit 0. Never publish “62/62,” “100% complete,” blended
score, screenshot-only evidence, giant “Nx faster” label, hidden failures, or
timing from different machines as direct comparison.

Reddit posture:

> I built a reproducible evidence report for a Git-local provenance CLI. I’d
> value criticism of the oracles and limitations.

Lead with what bytes changed and how reader can inspect them. State unkeyed chain,
caller-supplied usage/verdict, direct-capture trust boundary, and queue simulator
in body. Ask: which oracle is weak, which workload unrepresentative, which
negative case missing. Do not ask for stars/jobs or predict applause.

Professional signal comes from claim discipline, oracle design, failure handling,
reproducibility, corrections, and readable raw evidence. Social response cannot
be guaranteed or benchmarked.

## Minimum Credible Versus Gold Standard

Minimum credible:

- all currently binary-observable claims mapped to claim-specific independent
  oracles;
- real release binary, stock Git, retained repository, BagIt package, verifier;
- core negatives and mutation probes;
- macOS arm64 and Linux x86_64 semantic runs where available;
- performance limited to hook foreground, drain quiescence, check miss/hit, and
  projection sizes;
- raw samples and explicit local/simulator/caller-supplied limits.

Gold standard adds every non-discontinued `C-*`, state-machine generated
sequences, targeted mutation testing, all supported platforms, multiple stable
hosts, preregistered metrics/stopping rules, randomized paired baseline order,
independent rerun, blank-environment reproduction, immutable archival mirror,
and public correction policy.

## Result Taxonomy

- `conformant`: independent state oracle passed;
- `nonconformant`: oracle observed wrong state;
- `incomplete`: required evidence missing or run interrupted;
- `expected_refusal`: negative case refused and state stayed unchanged;
- `simulator`: exact local mechanics observed, production semantic explicitly
  not claimed;
- `unsupported`: outside selected supported surface, never removed from report;
- `not_applicable`: discontinued scope, excluded from success denominator;
- `performance_sample`: qualified timing observation, never feature verdict.

No aggregate score. Summary reports counts by taxonomy and lists every claim id.
