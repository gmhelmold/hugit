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
   result is informational because that older validator still asserts the
   legacy `.hugit/log.json` path while current hooks use Git-common-dir runtime
   state; this matrix is the gating benchmark.
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
| `expected_class` | `live-success`, `silent-success`, `fail-closed`, or `rejected`. |
| `exit` | Observed process exit code. |
| `stdout` | Complete captured standard output. |
| `stderr` | Complete captured standard error. |
| `status` | Harness judgment: `pass` or `fail`. |

`summary.json` records source URL, binary, count, failure count, status, and
the required case list. `manifest.json` binds report files to the schema name.

## Pass Criteria

Pass requires every declared case to exist and match its class:

- Live commands exit zero.
- Hook-only `capture` commands exit zero, including all four supported kinds.
- `pr land --dispatch` must reject/fail closed. It must never count as live
  runner execution or fabricate a successful landing.
- `ws` and `dispatch` must reject. They are permanently discontinued namespace
  probes, not live successes.
- Any missing case, unexpected exit class, clone failure, or release build
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
