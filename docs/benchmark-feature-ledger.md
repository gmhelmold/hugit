# Feature Ledger Benchmark

`scripts/benchmark-feature-ledger.sh` exercises hugit's local CLI ledger with
the release binary and a real Git checkout. It does not contact hugit services,
CoreLink, a forge, or a runner. The only network operation is cloning the small
public fixture repository.

## Method

1. Build `hugit-cli` in release mode when `HUGIT_BIN` is absent.
2. Clone `HUGIT_BENCHMARK_REPO`, defaulting to
   `https://github.com/octocat/Hello-World.git`.
3. Set temporary `HOME`, XDG directories, and isolated Git config. No host
   Git configuration or repository is used.
4. Run setup, attach/detach, health, every capture kind, all live ledger
   families, local and committed symbol modes, dock modes, and boundary probes.
5. Invoke existing `validate-go-live.sh` first and preserve its output. Its
   result is a prerequisite: failure makes the benchmark fail. Both validator
   and matrix use Git-common-dir runtime state.
6. Preserve command output and write one JSON object per case to `cases.ndjson`.
   Write `summary.json` and `manifest.json` beside it. Report directory is
   printed and never removed.

Run:

```sh
HUGIT_BENCHMARK_REPO=https://github.com/octocat/Hello-World.git \
  ./scripts/benchmark-feature-ledger.sh
```

Use `HUGIT_BIN=/absolute/path/to/hugit` to select an existing release binary,
or `HUGIT_BENCHMARK_REPORT_DIR=/path/to/report` to choose durable output.

## Evidence Schema

Each `cases.ndjson` row contains:

| Field | Meaning |
|---|---|
| `case` | Stable benchmark case name. |
| `command` | Shell-escaped binary invocation. |
| `attempts` | Number of attempts; transient `log_busy` retries are explicit in the record. |
| `expected_class` | `live-success`, `silent-success`, `fail-closed`, or `rejected`. |
| `exit` | Observed process exit code. |
| `stdout` | Complete captured standard output. |
| `stderr` | Complete captured standard error. |
| `status` | Harness judgment: `pass` or `fail`; success cases also require valid JSON unless silent. |

`summary.json` records source URL and commit, binary version, Git version,
platform, count, failure count, prerequisite result, status, and required case
list. `manifest.json` binds report files to the schema name.

## Pass Criteria

Pass requires every declared case to exist and match its class:

- Live commands exit zero.
- Hook-only `capture` commands exit zero, including all six supported kinds.
- `pr land --dispatch` must reject/fail closed. It must never count as live
  runner execution or fabricate a successful landing.
- `ws` and `dispatch` must reject. They are permanently discontinued namespace
  probes, not live successes.
- Any missing case, malformed success output, unexpected exit class, clone failure, or release build
  failure makes the script exit nonzero.

## Scope Notes

The benchmark measures CLI-local behavior only. Remote hosting, identity,
tenancy, remote AC, runner execution, and mirror deployment are not applicable
to this product surface. `hugit serve` and `/v1` are historical/out-of-scope.
The discontinued probes remain in the report to prove rejection, never to pad
the live ledger. A successful command proves local execution against captured
fixtures, not deployment or remote delivery.

Prerequisite validator coverage remains available separately:

```sh
HUGIT_BIN=target/release/hugit ./scripts/validate-go-live.sh
```

This benchmark is intentionally broader: it records every ledger family and
alternative mode feasible without remote services.

## Verified Run

Run date: 2026-09-13. Hugit source baseline: `30aeed1`. Fixture:
`https://github.com/octocat/Hello-World.git` at commit
`7fd1a60b01f91b314f59955a4e4d4e80d8edf11d`. Binary: `hugit 0.1.4`. Host:
macOS x86_64, Git `2.51.0`.

Result: **62/62 cases passed, 0 failures**. This includes 46 live-success
cases, 11 silent-success cases, 3 fail-closed cases, and 2 discontinued
namespace rejection cases, plus alternative-mode and fixture setup cases. The
prerequisite `validate-go-live.sh` also passed.

Raw report was retained at `/tmp/hugit-benchmark-publishable` during this run:

| Artifact | SHA-256 |
|---|---|
| `cases.ndjson` | `8cc2b4468df4e187b78bf10d0f1e6fb1b0f067cdd932c35d2bcbc6c155db4649` |
| `summary.json` | `75141e76401339e59c1642e5148f5af30e7b8e02551ecf3eca1d464ae0d0ebef` |
| `manifest.json` | `607a509378f1015cfce48ab131faa22d92415b11163b6b2d364c711b97b4f933` |

The report is machine evidence for this run, not a permanent repository path.
Re-run the documented command to produce a new dated report and new hashes.
