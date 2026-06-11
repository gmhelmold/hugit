# hugit

> **hug it** — the git-compatible, LLM-native forge. Embrace the community,
> fix the workflow.

**Status (2026-06-11): build complete; adversarial hardening ongoing (Wave H
complete, Wave I in progress, Round 6 pending).** A **17-package** Rust
workspace (hugit-app + {ui,exit,sidecar} sub-crates = 4 app crates + 13
feature crates) implements all 67 work-packages of decomposition v2.0. The
codebase has been through SOTA-audit waves A/B/C/D/E/F/G/H + the
memoized-CI wedge wave, 2 component migrations (hugit-web → githugr;
hugit-runner → corelink-runners), and schema 1.2.0 (money as integer
micro-USD). Five adversarial rounds (fresh 7-agent fleets) each returned
7/7 DO-NOT-SHIP; Waves E+F+wedge+G+H remediated Rounds 1–4; the spine held
through all five rounds. Wave I is remediating Round 5 findings (honesty
gap: event-log hash chain tamper-EVIDENT not tamper-PROOF — PS-8 tracks
log-auth as P2 seam; plus forge state-machine coherence and
identifier-redaction coupling). `main` is green by local gate (fmt + clippy
`--workspace --all-targets --locked -D warnings` + test `--workspace
--locked` + deny + audit); remote CI gate passes when it runs to completion,
but the single self-hosted runner is contention-flaky (~35% of recent runs
fail — includes both infra failures and a code fmt failure at HEAD before
hotfix eec3eab). What remains is owner-gated infra (P2 CoreLink tenant
provisioning). See **[CLAUDE.md](CLAUDE.md)** for the live source of truth.

## What hugit is

A version-control + merge + CI platform designed for the way software is built
in 2026: **orchestrated fleets of AI agents with a human in command.** Git's
data model is kept (it is a content-addressable store with refs — exactly the
primitive CoreLink already runs in production); git's *workflow* is rebuilt:

- **Worktrees = workspace snapshots** — N agents = N disposable cursors over one
  durable, content-addressed object.
- **Conflicts are first-class objects** (the jj model, server-side) — stored,
  never blocking.
- **Continuous speculative merge** — conflicts surface at write time, not merge
  time; the orchestrator's DAG is a forge primitive.
- **Memoized checks** — a CI check is a function of the tree hash; merging N
  green branches whose union was already tested re-runs nothing.
- **Semantic merge** — lockfile-aware ("regenerate, don't text-merge"),
  AST-aware, LLM-arbitrated behind a confidence gate.
- **PRs are structured machine verdicts**, not prose threads; policy-as-code
  gates are native.

## What hugit is not

- Not a frontal attack on GitHub's social network. The compat ladder: git wire
  protocol → ride on GitHub (the agent landing layer) → bounded bidirectional
  mirror → authoritative forge.
- Not a new paradigm to learn. The naming principle is the product principle:
  **don't deviate from git** — every deviation costs human adoption and LLM
  affinity (models are trained deeply on git).

## Founding documents

- [`docs/strategy/campaign-3-llm-native-forge.md`](docs/strategy/campaign-3-llm-native-forge.md)
  — the founding brief (thesis, pillars, competitive map, economics, caveats).
- [`docs/research/`](docs/research/) — the 2026-06-05 four-lane evidence sweep
  (git core pains, GitHub platform pains, multi-agent workflow pains,
  competitive landscape).

## Relationship to CoreLink

hugit is expansion campaign #3 of the CoreLink product family — the namespace,
merge, and policy layer over the same content-addressed primitive stack:
cache (launch) → compute (campaign #1, CI runners) → workspace (campaign #2,
snapshots) → **forge (campaign #3, this repo)**.
