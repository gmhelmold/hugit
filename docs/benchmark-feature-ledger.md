# Hugit Reproducible Evidence Report

This is semantic conformance report, not performance benchmark and not product
completeness score. It measures one local hook-to-land journey against retained
state. Research basis and publication rules:
`docs/research/2026-09-13-professional-evidence-report.md`.

## Run

```sh
./scripts/benchmark-feature-ledger.sh --run-id local-example
```

`HUGIT_BIN=/absolute/path/to/hugit` selects existing binary.
`HUGIT_EVIDENCE_DIR=/durable/report` selects a **new** output directory below
an existing parent. The output directory and `<report>.tar` must not already
exist, even as empty directories or dangling links; previous evidence is never
reset or overwritten. A retry needs a fresh destination.

Without that variable, the wrapper allocates a unique temporary parent and uses
its new `report` child. It prints that path before the run, so a failure remains
locatable. Completed or partial evidence is retained in that parent; only the
producer's own separate workspace is cleaned up after its identity is checked.
A replaced workspace or failed cleanup is reported, not silently removed.
The runner also creates deterministic `<report>.tar` serialization exclusively
and prints its SHA-256. Creation failures never authorize overwriting an artifact.

Verify directory independently:

```sh
python3 scripts/verify-evidence-report.py /durable/report
```

Pull requests changing evidence surface run `.github/workflows/evidence.yml`.
`produce` builds and packages on GitHub-hosted Ubuntu. Separate
`verify-on-fresh-host` job downloads same artifact onto fresh Ubuntu VM, verifies
archive checksum and semantic package, then uploads independent verification JSON.
Actions artifact retention is 90 days. Workflow logs provide build context;
package still labels source-to-binary cryptographic binding unestablished.

Verifier never executes benchmark. It reads package bytes, safely inspects
archives before extraction, runs stock Git, verifies event chain, recomputes
known journey-v1 semantics, and fails unknown selected claims.

## Scenario

1. Select, retain, and hash executable. Source-to-binary binding is explicitly
   unestablished.
2. Create isolated synthetic Git repository, HOME, XDG dirs, and Git config.
3. Install repository hooks with `hugit setup --repo`.
4. Make normal `git commit`; wait until receipt drain reaches canonical log.
5. Bind exact Git OID to explicit intent through local PR.
6. Run check miss, exact-key hit, and changed-tree miss. External sentinel must
   contain exactly two executions.
7. Record caller-supplied verdict and usage. Land PR and verify exact frozen
   price-card result while disclaiming provider authenticity.
8. Read ledger/watch, export separate clean corpus, and probe discontinued
   top-level command refusal without runtime-state change.
9. Retain full repository and export, build BagIt manifests, self-check package,
   then run separate verifier.

Scenario configures no network action. Network absence is not instrumented. No
performance number is produced.

## Selected Claims

| Class | Claim IDs |
|---|---|
| Semantic observations | `C-SETUP-REPO`, `C-CAPTURE-COMMIT`, `C-CAPTURE-RECEIPT-ID`, `C-COMMIT-INTENT-BIND`, `C-INTENT-NEW`, `C-PR-OPEN`, `C-CHECK-RUN`, `C-VERDICT-RECORD`, `C-CTX-USAGE`, `C-CTX-USAGE-PRICE`, `C-PR-LAND`, `C-LEDGER`, `C-WATCH`, `C-EXPORT-GIT` |
| Expected refusals | `C-SCOPE-WS`, `C-SCOPE-DISPATCH` |
| Not applicable | `C-SCOPE-RUNNER` |

Unselected ledger claims remain unmeasured by journey-v1. They do not disappear,
pass, fail, or enter denominator.

## Package

Package is BagIt 1.0 plus hugit v1 schemas:

```text
report/
├── bagit.txt
├── bag-info.txt
├── manifest-sha256.txt
├── tagmanifest-sha256.txt
├── README.md
└── data/
    ├── report.json
    ├── claims.json
    ├── source.json
    ├── binary.json
    ├── environment.json
    ├── methodology.md
    ├── limitations.md
    ├── limitations.json
    ├── repository.tar
    ├── export.tar
    ├── commands/
    └── assertions/
```

`repository.tar` preserves worktree plus `.git`: objects, refs, index, config,
hooks, canonical log, receipts state, intent sidecar, AC, and external check
sentinel. Command records preserve argv array, cwd label, allowlisted env,
stdout/stderr, exit, UTC start, and monotonic duration.

Manifest checks establish byte fixity, not author identity or fairness. Event
chain is unkeyed. Full-access writer can rewrite chain and package consistently.

## Independent Checks

Verifier names 14 directory checks; archive input adds outer-archive check:

- package and tag inventory/checksums;
- schema, claim coverage, evidence digests, taxonomy aggregates;
- repository/export archive safety and normalized metadata;
- stock `git fsck --full` over retained source and exported Git;
- independent event-chain verification;
- claim-specific semantic recomputation;
- explicit semantic-boundary disclosure.

Teeth suite has two positive controls plus 38 negative controls. It
mutates payload bytes, inventory, paths, claims, evidence hashes, hook modes,
tar metadata, env keys, Git objects, event fields, memo sentinel, projections,
scope taxonomy, and export objects. Unknown claim fails closed. Mutation probe
weakens checksum guard, observes wrong green, restores guard, then observes red.

## Reference Run

Local reference run on 2026-09-14 used `hugit 0.1.4` binary SHA-256:

```text
4951577ab86fbf0125b361317dc74ba956da5fd4a9b8296f8c93a8abeb7e0db4
```

Independent verifier result:

```text
overall_status: valid
conformant: 14
expected_refusal: 2
not_applicable: 1
nonconformant: 0
incomplete: 0
simulator: 0
unsupported: 0
```

Deterministic evidence archive SHA-256:

```text
489681fda4740a8f79ff8ddc378f6db4a6bc34a1bc33cd33b89f65a08c2680fe
```

Run artifact is retained locally, not published in repository. Hash records
local custody only; it is not public reproduction proof. Before public claim,
attach archive to immutable release/record and have independent machine rerun
verifier.

## Limits

- One synthetic local journey on macOS; no cross-platform result yet.
- Selected executable bytes/hash are retained, but build provenance does not bind
  them to source revision. Command attribution is structured evidence, not
  cryptographic proof of execution causality.
- Verdict and usage are caller supplied. Pricing proves arithmetic, not provider
  authenticity.
- Export uses separate clean corpus because primary hook corpus is refused as
  sensitive.
- Runner execution is discontinued and intentionally unexecuted. Legacy
  `pr land --dispatch` remains source-reachable; top-level `dispatch` refusal
  does not prove its behavior.
- Expected-refusal before/after digests are retained metadata; verifier cannot
  time-travel to independently reconstruct prior state.
- No network-denial instrument and no performance study in journey-v1.
- No aggregate “17/17,” percentage-complete, or forge-parity claim is valid.
