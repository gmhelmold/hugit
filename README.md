# hugit

> **hug it** — the git-compatible, LLM-native forge. Embrace the community,
> fix the workflow.

**Status (2026-06-08): build complete.** A 17-package Rust workspace (15 crates
+ `hugit-app/{ui,exit}`; `hugit-app-sidecar` auto-included) implements all 67
work-packages of decomposition v2.0. `main` is green by local gate (fmt +
clippy `--workspace --all-targets --locked -D warnings` + test `--workspace
--locked` + audit); remote CI green on HEAD, flaky under single-runner
contention. What remains is owner-gated infra (P2 CoreLink tenant
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
