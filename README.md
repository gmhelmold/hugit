# hugit

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

> **hug it** — the git-compatible, LLM-native forge. Embrace the community,
> fix the workflow.

## Quick start

```sh
git clone https://github.com/HumanGuardrail/hugit.git
cd hugit
cargo build --release
./target/release/hugit --help
```

**Status (2026-06-20):** A **19-package** Rust workspace. The integrity spine
(Ed25519/SHA-256 crypto, policy engine, platform safety invariants) is
hermetic, hardened across 13 rounds of adversarial security review, and
genuinely solid. The engine `/v1` read+write API is **LIVE** for the hugit
repo: 11/20 reads serve chain-verified R2 data; all 9 POST verbs are
CAS-persisted and `authz`-gated; `git clone`/fetch logic is built and
CI-proven (lazy git-from-CAS, boots ~5 s). **Not yet live:** anonymous git
clone (repo is auth-gated on the deployed surface), CoreLink's live CAS+AC
(hot-path tenant), runner fabric, GitHub App mirror, multi-tenant, `git push`.
`main` is gated by fmt + clippy `--workspace --all-targets --locked -D
warnings` + test + deny. See the [whitepaper](docs/whitepaper/hugit-v1.md)
for design detail and the honest live-vs-hermetic-vs-absent status.

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

- [`docs/whitepaper/hugit-v1.md`](docs/whitepaper/hugit-v1.md) — full product
  design: thesis, object model, algorithms, architecture, economics, risks, and
  the phased route (§12).
- [`docs/adr/`](docs/adr/) — architecture decision records (context envelope,
  identity model).

## Relationship to CoreLink

hugit is built on the CoreLink product family — the namespace, merge, and
policy layer over the same content-addressed primitive stack: CoreLink's CAS
(cache), runners (CI compute), and workspaces (snapshots) → **hugit (forge,
this repo)**.
