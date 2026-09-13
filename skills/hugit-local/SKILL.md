---
name: hugit-local
version: 0.1.0
description: Validate hugit as a local Git plugin from a real binary. Use when onboarding a repository, checking hook installation, exercising capture and local landing, or proving a release claim without remote services. Refuses remote assumptions, stale roadmap work, and success claims without parsed output and exit evidence.
---

# /hugit-local - Validate the local plugin

Use this skill for local hugit behavior only. A Git repository, Git, and the
`hugit` binary are sufficient. Do not invoke `ws`, `dispatch`, runner fabric,
remote AC, Clerk, tenancy, forge, mirror, or attestation paths: they are
permanently discontinued for hugit.

## Triggers

| Need | Run |
|---|---|
| Validate release baseline | `cargo build --release`, then the go-live script |
| Onboard a new repository | `hugit setup` or `hugit attach` |
| Inspect hook state | `hugit health --dir <repo>` |
| Verify silent capture | Git commit/checkout/merge/push attempt, then inspect the local log |
| Verify landing wedge | `hugit check`, `hugit verdict`, `hugit pr`, `hugit land queue` |
| Verify evidence reads | `hugit ledger`, `hugit watch`, `hugit why`, `hugit review` |
| Need remote execution or account state | **STOP**; outside hugit scope |

## Release Probe

Run from repository root:

```sh
cargo build --release
HUGIT_BIN="$PWD/target/release/hugit" ./scripts/validate-go-live.sh
```

The probe creates an isolated `HOME` and Git repository. It must prove hooks,
canonical log capture, campaign/intent flow, memoized check MISS to HIT, queue
landing, dock state, export, seal, and guards. Read its exit code and stdout;
do not infer success from partial output.

## Onboarding

1. Existing repository: run `hugit attach --repo <path>`.
2. New repositories: run `hugit setup`, then `git init`.
3. Inspect with `hugit health --dir <path>`.
4. Preserve foreign hooks. A conflict is a reported state, not permission to overwrite.
5. Use `hugit detach --dir <path>` only when removing managed hooks is intended.

`setup --repo` installs into an existing repository without changing global
`init.templateDir`. `setup --status` inspects global setup without mutation.

## Evidence Rules

- Local runtime state lives at `<git-common-dir>/hugit/event-log.json`.
- A push hook records a push attempt, never remote acceptance.
- Capture is best-effort and must not block Git.
- Hash-chain reads fail closed on malformed, tampered, or missing evidence.
- `check` HIT means declared key inputs matched; it does not prove undeclared
  environment, network, clock, or filesystem dependencies matched.
- `pr land` records local landing state; it does not update a Git destination ref.
- A cost is `null` or measured from explicit provider usage; never estimate or
  copy a number from another intent.

## Output Contract

Every normal command emits JSON on stdout:

| Exit | Meaning | Action |
|---|---|---|
| `0` | Result | Parse JSON; record one load-bearing fact |
| `2` | Structured user/domain error | Parse `error.kind`; act on `error.fix` once |
| `1` | Internal fault | Stop; report command, inputs, and JSON error |

Never substring-match prose. Never treat a non-zero exit as success. Never
call a `401`/`404` remote response proof that a route is absent; remote probes
are outside this skill anyway.

## Completion Card

Return:

```text
HUGIT-LOCAL CARD
  probe  : <command or script>
  exit   : <0 | 2 | 1 | stop>
  result : <one observed fact or error.kind>
  scope  : local-only
  honesty: <capture/remote/cost caveat, or clean>
```

Do not claim "live", "complete", or "release-ready" from a unit test alone.
State whether evidence is source-level, hermetic, real-binary local, or
verified end-to-end.

## Ground Truth

- `crates/hugit-cli/src/lib.rs`: live and namespace-reserved tokens.
- `crates/hugit-cli/src/main.rs`: actual dispatch.
- `crates/hugit-cli/src/porcelain.rs`: JSON and exit law.
- `docs/feature-ledger.md`: local feature behavior and evidence.
- `docs/manual-validation.md`: real-binary walkthrough.
- `CLAUDE.md`: permanent scope closure.
