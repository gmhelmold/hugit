# Feature Ledger: hugit CLI-local v1

This ledger describes shipped local behavior only. hugit is a free, open-source
Git plugin: a repository, Git, and the `hugit` binary are enough. Remote hosting,
identity, tenancy, and runner execution are outside this plugin scope.

## Status And Evidence

| Mark | Meaning |
|---|---|
| **LIVE** | Dispatched in `crates/hugit-cli/src/main.rs`, registered in `HUGIT_VERBS`, and backed by implementation. |
| **T** | Covered by unit or acceptance tests. |
| **U** | The command or command family appears in the real-binary walkthrough in `docs/manual-validation.md`; this does not claim every option or subcommand path was manually exercised. |
| **HISTORICAL** | Optional `hugit-serve` material, not CLI-v1 product status. Hermetic tests do not prove deployment. |
| **RESERVED** | Kept out of CLI v1; not a hidden implementation backlog. |

Unless shown otherwise, `--log` means explicit path, then `$HUGIT_LOG`, then
`.hugit/log.json`. Commands emit stable JSON. User/domain errors exit 2;
internal faults exit 1. `hugit capture` is the exception: hook-only, silent,
best-effort, always exit 0 so Git is never blocked.

Evidence anchors: `scripts/validate-go-live.sh` and
`docs/manual-validation.md` cover the real-binary path; `crates/hugit-cli/tests/`
contains the acceptance suite; `acceptance_rcli.rs` checks binary/registry
equality; `acceptance_w0.rs` checks live/reserved separation.

## Live Top-Level Surface

Every row below is dispatched. Subcommands are exact current names.

| Command | Behavior | Evidence |
|---|---|---|
| `hugit setup` | Installs hugit Git hooks in global `init.templateDir`; `--repo <path>` installs into existing repo; `--status` inspects; `--dir <path>` selects template; `--replace-global-template` replaces another global template only when requested. | T `acceptance_setup`; U |
| `hugit attach` | Safely adopts hugit-owned hooks in existing repo, preserving foreign hooks. `--repo <path>` selects repo; `--preview` is read-only; `--adopt-managed-dispatcher <preview-token>` adopts exact preview bytes; `--detach` restores only byte-matching managed hooks. | T `acceptance_attach`, `acceptance_capture`; U |
| `hugit detach` | Removes hugit-owned hooks from an existing repo while retaining captured evidence. `--dir <path>` selects repo; default is current directory. | T `acceptance_capture`; U via attach/detach walkthrough |
| `hugit health` | Reports hook, log, capture, coverage, and local fact state; distinguishes observed locally, attempted push, explicit declaration, and unsupported. Never claims remote push success. | T `acceptance_gitlocal_journey`, `acceptance_setup`; U |
| `hugit capture` | Internal hook/worker input for `commit`, `checkout`, `push-attempt`, and `merge`; optional bounded rewrite/reference-transaction receipt fields preserve raw Git facts. Never onboarding; never blocks Git. | T `acceptance_capture`, `acceptance_capture_jj_checkout_merge`; U indirectly through Git |
| `hugit campaign open` | Appends campaign charter and human owner. | T `acceptance_pc1`, `acceptance_wbcamp`; U |
| `hugit campaign close` | Seals campaign with proof and rollup; `--allow-rejected` explicitly permits rejected intents; repeat close is idempotent. | T `acceptance_wj_close`, `acceptance_wbcamp`; U |
| `hugit campaign show` | Projects landed, in-flight, blocked, verdict, and cost state for `--campaign`. | T `acceptance_pc1`; U |
| `hugit campaign list` | Lists campaigns from canonical log. | T `acceptance_pc1`; U |
| `hugit campaign abandon` | Appends idempotent abandonment with required `--reason`; closed campaigns cannot be abandoned. | T `acceptance_wbcamp`; U |
| `hugit intent new` | Records charter, repeatable `--acceptance`, campaign, optional `--id`, `--agent`, `--context-ref`, `--store`, and shared `--log`; sealed campaigns reject new intents. | T `acceptance_pc2`, `acceptance_wave_m_intent_atomicity`; U |
| `hugit intent show` | Projects intent, sidecar, context ref, and verdicts from `--store`. | T `acceptance_pc2`; U |
| `hugit intent list` | Lists intent store globally or filtered by `--log` and `--campaign`; resolves each intent's owning log. | T `acceptance_pc2`; U |
| `hugit issue transition` | Appends issue state transition to `backlog`, `open`, `closed`, or `dispatch`, with issue number and optional priority. | T `acceptance_pc3`; U |
| `hugit pr open` | Opens PR from repeatable `--intent`, captured `--commit`, or captured `--commit-ref`; requires `--author-kind orchestrator|human`; rejects subagent authors; supports `--run-id`, `--principal`, and `--recorded-at`. | T `acceptance_pc3`, `acceptance_pr_commit`; U |
| `hugit pr queue` | Appends PR to union landing queue. | T `acceptance_pc3`, `acceptance_wbpr`; U |
| `hugit pr land` | Settles queued PR as landed and captures metrics/envelopes. Supports `--dispatch` as an explicit external-runner seam that fails closed when unwired; manual metrics use `--tokens`, `--cost-usd-micros`, `--tool-calls`, `--active-ms`, `--model-turns`, `--model`, `--context-cas`, transcript refs, and `--verdicts-ref`. | T `acceptance_wprlanded`, `acceptance_pc3`; U |
| `hugit pr show` | Shows PR, bundled intents, queue state, and cost rollup. | T `acceptance_pc3`; U |
| `hugit pr list` | Lists PRs, optionally filtered by `--campaign` and `--state` (`proposed`, `queued`, `abandoned`, `landed`). | T `acceptance_pc3`; U |
| `hugit pr abandon` | Idempotently abandons PR with required `--reason`; removes it from queue projection. | T `acceptance_pc3`; U |
| `hugit land queue` | Runs local union test over queued PRs; memoizes checks, lands green set, and bisects a red batch to minimal failing pair. Optional `--campaign`, `--ac`, `--recorded-at`. | T `acceptance_land_queue`, `acceptance_wb2`; U |
| `hugit queue show` | Shows queued entries, batch composition, campaign scope, and recorded union failure. | T `acceptance_wb2`; U |
| `hugit check run` | Runs local memoized check. Built-ins: `fmt`, `clippy`, `test`; custom definitions require `--cmd`. Supports `--store`, `--root`, `--toolchain`, `--pr`, `--principal`, `--ac`, `--timeout-secs`, repeatable `--env-axis`. | T `acceptance_wcheck`, `acceptance_q_ancestor_manifest`, `acceptance_n1_modebit`; U MISS→HIT |
| `hugit check show` | Projects recorded checks and hit rate; optional `--pr`. | T `acceptance_wcheck`; U |
| `hugit check key` | Computes memo key from `--tree`, `--def`, and `--toolchain` without execution. | T `acceptance_wcheck`; U |
| `hugit verdict record` | Records or dry-runs multi-lens panel: repeatable paired `--lens` and `--result` (`approve`, `fix_first`, `reject`), optional `--store`, `--tree-hash`, `--recorded-at`; approves only when every lens approves. | T `acceptance_wverdict`, `acceptance_d7`; U dry-run + stored verdict |
| `hugit verdict approve` | Records single-lens human approval for `--intent`. | T `acceptance_k_verdict`, `acceptance_wverdict`; U |
| `hugit verdict reject` | Records single-lens human rejection for `--intent`. | T `acceptance_k_verdict`, `acceptance_wverdict`; U |
| `hugit why` | Resolves path provenance from `--log`; `--walk` returns full chain; `--line` and `--symbol` require `--repo` and optional `--commit`, using committed-tree blame. Fails closed when evidence is absent. | T `acceptance_d10`, `acceptance_round8_readpath`; U |
| `hugit impact` | Computes build-graph blast radius from `--graph` and repeatable `--path`; accepts Cargo, pnpm, turbo, or unknown ecosystem labels. | T `acceptance_d10`; U |
| `hugit tournament` | Generates policy-capped candidate fan-out with `-n|--candidates`, `--intent`, optional `--log`; checks intent existence when log supplied. | T `acceptance_d13`; U |
| `hugit export` | Exports chain-verified canonical log into synthetic Git artifact, envelope JSON, and redaction manifest. Requires `--log <path> --out <dir>`; no source-history/topology guarantee. | T `acceptance_e5`, `acceptance_wkchain`; U |
| `hugit undo` | Appends human-only compensating event for `--seq`; never rewrites history; honest `nothing_to_compensate` when no inverse exists. | T `acceptance_d13`; U |
| `hugit policy test` | Evaluates house gates against required `--context <path>` locally; fail-closed on missing or malformed context. | T `acceptance_pc4_cycle`; U |
| `hugit policy edit` | Appends human-only policy change over house baseline; supports gate `--enable` or `--disable` and optional `--reason`. | T `acceptance_pc4_cycle`; U through policy walkthrough |
| `hugit note` | Appends scrubbed `journal.note`; supports `--note`, optional `--workspace`, `--intent`, and `--principal`. | T `acceptance_ctx`, `acceptance_wi_scrub`; U |
| `hugit diag` | Bisects log-backed red check history by `--def-digest`, optional `--toolchain`, and reports structured diagnosis. Read-only. | T `acceptance_d10`, `acceptance_fleet_journey`; U |
| `hugit ledger` | Projects asked → done → proven history from `--log`, optionally `--campaign`. | T `acceptance_ledger`; U |
| `hugit fleet` | Projects versioned machine-readable workspace/agent state from `--log`. | T `acceptance_fleet`; U |
| `hugit watch` | Replays classified, redacted event stream from `--log`; optional `--class`. | T `acceptance_watch`; U |
| `hugit symbol` | Emits tree-sitter symbol outline for local `--file`; supports TypeScript, JavaScript, Python, Go, Java, C, C++, and Ruby. | T `acceptance_symbol`; U |
| `hugit ctx resume` | Reconstructs short-horizon session from matching `journal.note` records using `--workspace`, `--intent`, optional `--tenant`, `--now-ms`. Refuses when evidence/horizon is insufficient. | T `acceptance_ctx`, `acceptance_review`; U |
| `hugit ctx usage` | Appends provider `/usage` token counts verbatim to `ctx.usage`; exactly one of `--intent` or `--pr`; requires `--model`, `--input`, `--output`, `--cache-read`, `--cache-write`; optional `--model-digest`, `--recorded-at`. Computes only checked token total, never prices or calls network. | T `acceptance_ctx`, unit tests in `ctx/usage.rs`; U |
| `hugit review` | Answers questions only from logged check/verdict evidence; refuses unsupported claims. Requires `--question`, optional `--intent` and `--log`. | T `acceptance_review`; U |
| `hugit dock coin` | Coins physical worktree/repo binding; hook-born, idempotent, non-blocking. | T `acceptance_dock_coinage`, `acceptance_dock_resolver`; U |
| `hugit dock ls` | Lists dock ids, branches, origin, and `open`/`ghost` state. | T `acceptance_dock_resolver`; U |
| `hugit dock show <id>` | Shows one dock record. | T `acceptance_dock_resolver`; U |
| `hugit dock close` | Finalizes and reconciles one dock; idempotent. | T `acceptance_dock_reconcile`; U |
| `hugit dock reconcile` | Closes ghost docks whose Git directory disappeared. | T `acceptance_dock_reconcile`; U |
| `hugit dock insight` | Projects per-branch cost and residual buckets; absent cost stays honest zero. | T `acceptance_dock_insights`; U |
| `hugit dock land` | Requires worktree SHA byte identity and green acceptance before landing; fail-closed otherwise. | T `acceptance_dock_land`; U |
| `hugit meta set` | Appends local `repo.meta` visibility/owner metadata with `--visibility public|private`, optional `--owner-tenant`, `--by`, `--recorded-at`. This records policy; it is not a hosting service. | T `acceptance_wj_matrix`; U |

## Capture And Cost Rules

- Runtime state lives at `<git-common-dir>/hugit/event-log.json` (legacy
  `.hugit/log.json` is migrated/used by the local resolver); it is outside the
  worktree and shared by linked worktrees.
- Events are append-only and hash-chained. Reads verify the chain; tampering,
  malformed payloads, and missing evidence fail closed.
- Hooks observe local commit, checkout, push attempt, merge, rewrite, and
  reference-transaction facts. A push attempt is not remote confirmation.
- `ctx usage` records provider token counts; `hugit` does not infer tokens from
  Git activity and does not call a provider.
- `hugit pr land` prices matching `ctx.usage` records only when every record has
  a known exact model price. Current frozen card: `pc-2026-07`, exact Anthropic
  ids `claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5-20251001`,
  `claude-haiku-4-5`, and `claude-fable-5`. Unknown model, malformed record, or
  overflow yields honest zero, never fallback pricing.
- Manual land metrics are explicit operator input. Omitted metrics stay zero;
  transcript blobs are not created by local land. `--dispatch` is not local
  execution and fails closed without external runner wiring.
- `hugit export` is a portable proof/snapshot, not a promise to preserve source
  Git commit topology, refs, or hosting-provider state.

## Reserved Or Out Of Scope

| Surface | Status | Boundary |
|---|---|---|
| `hugit ws` | **RESERVED** | No CLI v1 workspace lifecycle; workspace execution belongs elsewhere. |
| `hugit dispatch` | **RESERVED** | No CLI v1 off-box agent execution; remote runner execution is deferred. |
| `hugit ctx snap` | **ABSENT** | No second context store in CLI v1; use canonical log, `hugit note`, `hugit ctx usage`, and `hugit export`. |
| `hugit init` | **NOT A CLI VERB** | Library-only bootstrap helper. It is not dispatched because `init` would shadow `git init`; use `hugit setup` for global templates or `hugit attach` for an existing repository. Evidence: T `acceptance_gitlocal_journey` library path; no binary evidence. |
| `hugit serve` / `/v1` / Git smart HTTP | **HISTORICAL/OPTIONAL** | Separate backend product surface. Not required by local CLI and not evidence of CLI behavior. |
| Remote hosting, identity, and tenancy | **DEFERRED** | Git remote remains hosting boundary; local plugin requires none. |
| GitHub App activation and live mirror deployment | **EXTERNAL** | Code may exist in optional crates, but activation/deployment is not CLI-v1 delivery. |
| Runner execution and remote AC | **EXTERNAL** | Local file-backed memoization is shipped; external fabric is not local CLI behavior. |
| Automatic jj observation after `jj git export` | **NOT IMPLEMENTED** | jj capture requires explicit MCP `capture`; no automatic v1 hook claim. |

## Validation

```sh
cargo test -p hugit-cli --locked
cargo test -p hugit-contracts --locked
cargo fmt --check
HUGIT_BIN=target/release/hugit ./scripts/validate-go-live.sh
```

Manual runbook: `docs/manual-validation.md`. Full source of truth for top-level
dispatch/reserved status: `crates/hugit-cli/src/lib.rs` (`HUGIT_VERBS` and
`HUGIT_RESERVED_VERBS`) plus `crates/hugit-cli/src/main.rs`.
