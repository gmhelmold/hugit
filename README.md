# hugit

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

> **hug it** — the git-compatible, LLM-native forge.

---

## The problem it solves

**Your agent fleet ships branches that are green alone and red together.**
hugit lands them on a `main` that is always green and re-runs zero CI it has
already paid for — on your existing GitHub repos, migrating nothing.

---

## First: try it now (no server needed)

`hugit symbol` works standalone against any source file. Build the CLI and
run it on itself:

```sh
git clone https://github.com/gmhelmold/hugit.git
cd hugit
cargo build --release
./target/release/hugit symbol --file crates/hugit-symbols/src/lib.rs
```

You get a structured symbol outline — functions, types, impls — extracted by
tree-sitter. Supported languages: TypeScript, JavaScript, Python, Go, Java, C,
C++, Ruby.

---

## hugit is a git-local CLI (CI lives elsewhere)

`hugit` is designed to work **where git works** — in a repository, on a laptop,
no server, no account. The runtime is local-only by default:

- **`hugit init`** runs the same ceremony as `git init` (initializes `.git/` when
  absent) and adds `.hugit/` + the versioned intent/context log.
- Every verb reads/writes the **canonical local log** (`.hugit/log.json`) with a
  verifiable hash chain — `why`, `impact`, `intent`, `pr`, `verdict`, `undo`,
  `policy`, `ledger`, `watch`, `symbol`, `ctx`, `review`, `export`, and more all
  work with zero CoreLink credentials.
- **`hugit check`** memoizes locally by default (file-backed AC); a CoreLink
  shared cache is an explicit opt-in via env, never a silent network call.
- **CI / compute execution is a separate project** (`corelink-runners`). hugit
  records the *demand*; the runner fabric is not rebuilt here.

`hugit serve` is the **optional remote / forge host** (smart-HTTP clone/fetch/push
+ the `/v1` API) — a separate binary, not part of the git-local CLI flow.

---

## Where hugit is today

The integrity spine (Ed25519/SHA-256 crypto, policy engine, platform safety
invariants) is hermetic and hardened. The `/v1` read+write API is live for
the hugit repo itself. The rest is honest about where it stands:

| Capability | Status | What you get today |
|---|---|---|
| `hugit symbol` — symbol outline | **LIVE** | `hugit symbol --file <path>` against any TS/JS/Python/Go/Java/C/C++/Ruby file |
| `hugit export` — exit guarantee | **LIVE** | full git + JSON snapshot; requires `--log <path> --out <dir>`; zero dependencies |
| `hugit import` — bring your repo | **ROADMAP** | reserved verb (`hugit import` is not in the CLI surface yet); tracked for the next wave. |
| `/v1` read+write API | **LIVE (2 repos)** | multi-repo (`hugit` + `githugr`); 11/20 reads serve chain-verified data; 9 POST verbs CAS-persisted + authz-gated; SSE replay |
| `hugit check` / `hugit verdict` | **LIVE** | real policy-engine EXECUTE paths; `hugit policy test` runs local≡forge |
| `hugit verdict approve` / `hugit verdict reject` | **LIVE** | single-lens wrappers over the canonical verdict record |
| `hugit undo` | **LIVE** | event-sourced compensating undo; force-push data-loss is unexpressible |
| `hugit note` | **LIVE** | appends a signed record to the canonical log |
| `hugit fleet` / `hugit ledger` / `hugit watch` | **LIVE** | real log-backed commands |
| `hugit diag` | **LIVE** | log-backed bisect |
| `hugit policy edit` | **LIVE** | append-only policy changes over the house baseline |
| `git clone` / `git fetch` wire protocol | **LIVE (anonymous-gated)** | the smart-HTTP wire is deployed (git-from-CAS, `/readyz git_serving:true`); anonymous clone is gated on the per-repo public-flag (deferred), not `HUGIT_SERVE_GIT_DIR` |
| File-content reads (`blob` / `edit`) | **LIVE** | path→blob traversal, secret-scrubbed on read; serving in prod via the CAS read path |
| `git push` (receive-pack) | **LIVE\*** | real push to the prod engine succeeds (#198, git-free gix-pack unpack on the distroless engine); a pushed ref is advertised **immediately** (live ref hot-swap, #201, no reboot); _\*v0 = self-contained packs (an incremental push on server-side history is rejected fail-closed — thin-pack/CAS-base reachability is a tracked follow-up); clone-back needs the per-repo public-flag (deferred)_ |
| Union-tested landing queue | **ROADMAP** | the core landing algorithm is designed + hermetically tested; EXECUTE needs the runner fabric |
| Memoized checks (CI dedup) | **ROADMAP** | algorithm built; requires the live runner + AC substrate |
| Runner fabric | **ROADMAP** | spec'd and in flight in a sibling repo; hugit is anchor tenant |
| GitHub App mirror | **ROADMAP** | bidirectional mirror design done; needs the App provisioned |
| Multi-tenant | **ROADMAP** | single-tenant today (the hugit repo); tenancy machinery built, not provisioned |

---

## What hugit does (once fully live)

For the **fleet operator** — an engineer or orchestrator running 10–50 agents
in parallel — today's pain is: every agent ships a branch that passes its own
CI, and they collide on merge. You spend evenings reconciling work that
machines produced in minutes.

hugit's answer is three properties working together:

1. **Union-tested landing queue.** Branches enter a queue and are tested as a
   batch *before* touching `main`. If the batch is green, they all land
   atomically. If it's red, a log₂-depth bisect over memoized checks finds the
   minimal failing pair in seconds; the rest of the batch lands anyway.
   `main` is always green — by construction, not by convention.

2. **Memoized checks.** A CI check is a pure function of
   `(tree-hash, check-def, toolchain)`. The second time that exact tree is
   checked — regardless of which branch or which agent produced it — the result
   is a cache lookup. Zero re-execution. The structural economics: GitHub bills
   the waste; hugit deletes it.

3. **Intent + context versioning.** Every commit carries the charter that
    produced it, the model and cost that executed it, and the claims that
    bounded its blast radius. `git log` shows you the diff; `hugit ledger` shows
    you the intent. Same store, two altitudes, always consistent.

---

## Safety and provenance (built, under-sold)

These are real today, not roadmap:

- **Event-sourced refs** — every ref mutation is an append-only log event.
  `hugit undo --seq <N>` walks it backwards with a compensating event. Refs
  cannot be force-pushed to oblivion. Data loss is structurally unexpressible.
- **Claim fences** — a workspace materializes only the paths the intent
  declared. Writing outside the claim is physically impossible (the file is not
  present), not merely forbidden by policy.
- **Model-level attestation** — every commit records which model authored it,
  under whose instruction, at what cost. SLSA-class provenance including the
  model layer; no forge today can express this.
- **Policy as code** — `hugit policy edit` (append-only) + `hugit policy test`
  (local = forge). The event log is the gate-set store; no external DB.
- **Adversarial hardened** — 13 rounds of adversarial security review on the
  integrity spine (Ed25519/SHA-256 crypto). Redaction at the read boundary.
  Fail-closed boot.

---

## The exit guarantee

```sh
hugit export --log .hugit/log.json --out <dir>
```

Produces a full git bundle + JSON proof of every intent, verdict, and claim.
Restore to a bare git repo on any hosting provider. No proprietary lock-in —
the exit proof is also the disaster-recovery plan.

---

## Bring your existing repo

```sh
# hugit import is reserved/ROADMAP (PR-7): see status table above
# hugit import <github-org>/<repo>
```

Your GitHub repo stays where it is. hugit attaches without migration. The
compat ladder: git wire protocol → landing layer riding on GitHub → bounded
bidirectional mirror → authoritative forge. You climb it at your pace; a
broken bridge kills trust, so every rung is reversible.

---

## How it works / Why it's cheap

hugit is not a greenfield stack. It is the forge layer of **CoreLink** — a
content-addressed storage and computation platform already in production:

| Layer | What it is | Status |
|---|---|---|
| **CAS** | R2-backed global object store; tenant-isolated; Merkle-verified | in production |
| **Action Cache** | memoized check results; surfaces: Bazel REAPI v2, Turborepo, sccache | in production |
| **Workspaces** | snapshot / hydrate / run (AC-memoized) against the live API | phase 1 shipped |
| **Runners** | ephemeral Firecracker-class compute; cache-warm boot | in flight |
| **hugit** | intent store, landing engine, policy engine, event-log refs | this repo — see status table above |

The cost physics fall out of the substrate: **zero egress (R2), global
content-addressed dedup, memoized verification.** GitHub's revenue model bills
the waste (per-minute CI, usage-billed AI). CoreLink's margin model deletes it.
For GitHub to match hugit's economics it must destroy its own P&L.

---

## Quick CLI reference

```sh
# Inspect any file's symbol structure (works right now, no server)
hugit symbol --file src/main.rs

# Export the repo as a portable proof bundle
hugit export --log .hugit/log.json --out <dir>

# Show the intent log (forge-connected; `hugit log` is not a verb — use
# `hugit ledger --log .hugit/log.json`)
hugit ledger --log .hugit/log.json

# Run local policy check (forge-identical)
hugit policy test

# Undo the last event-sourced operation (requires --seq <N>)
hugit undo --seq <N>
```

See `hugit --help` and [`docs/product/command-catalog.md`](docs/product/command-catalog.md)
for the full verb surface.

---

## Founding documents

- [`docs/whitepaper/hugit-v1.md`](docs/whitepaper/hugit-v1.md) — full design:
  thesis, object model, algorithms, architecture, economics, risks, phased route.
- [`docs/product/product.md`](docs/product/product.md) — ICP, positioning,
  pricing posture.
- [`docs/adr/`](docs/adr/) — architecture decision records (context envelope,
  identity model).

---

## Gate

`main` is gated by `cargo fmt --check` + `cargo clippy --workspace
--all-targets --locked -D warnings` + `cargo test --workspace --locked` +
`cargo deny`. CI runs on GitHub-hosted ubuntu. Docs-only pushes skip CI.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
