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

Download the **hugit** binary for your platform from
[GitHub Releases](https://github.com/gmhelmold/hugit/releases) (see
[docs/installation.md](docs/installation.md) for the exact commands per OS),
or build it:

```sh
git clone https://github.com/gmhelmold/hugit.git
cd hugit
cargo build --release
./target/release/hugit symbol --file crates/hugit-symbols/src/lib.rs
```

`hugit symbol` works standalone against any source file. You get a structured
symbol outline — functions, types, impls — extracted by tree-sitter. Supported
languages: TypeScript, JavaScript, Python, Go, Java, C, C++, Ruby.

**Quickstart, new repositories:** run `hugit setup` once, then use normal Git:

```sh
hugit setup
git init my-project
cd my-project
git add . && git commit -m "initial commit"
git checkout -b feature/example
git push -u origin feature/example
hugit health
```

`setup` installs a Git template for future repositories. `commit`, `checkout`,
and `push` remain Git commands; hugit observes locally available facts without
changing their outcome. `health` labels facts as **observed locally**, **attempted push**, **explicit declaration**, or **unsupported**; it reports partial coverage instead of inventing remote confirmation.

**Existing repository:** run `hugit attach`, then normal Git (`commit`,
`checkout`, `push`), then `hugit health`. Foreign hooks stay preserved; health
reports partial coverage rather than replacing or guessing about them.

Before attaching a repository with existing tooling, inspect exact hook
ownership without writes:

```sh
hugit attach --preview
```

---

## hugit is a git-local CLI

`hugit` is designed to work **where git works** — in a repository, on a laptop,
no server, no account. The runtime is local-only:

- **`hugit setup`** (one-time) configures Git's global `init.templateDir` so every
  future `git init` ships hugit hooks. Runtime state lives at
  `<git-common-dir>/hugit/event-log.json`, outside tracked worktree state and
  shared by linked worktrees. For an existing repo, run `hugit attach`; it
  installs only missing hugit-owned hooks.
- Every verb reads/writes the **canonical local log**
  (`<git-common-dir>/hugit/event-log.json`) with a
  verifiable hash chain — `why`, `impact`, `intent`, `pr`, `verdict`, `undo`,
  `policy`, `ledger`, `watch`, `symbol`, `ctx`, `review`, `export`, and more all
  work with no service and no account.
- **`hugit check`** memoizes locally (file-backed cache): the same check on the
  same tree+def+toolchain is a cache HIT with zero re-execution.
- **CI / compute execution is not rebuilt here** — hugit records the *demand*
  and leaves execution to whatever runs your checks.
- `ws`, `dispatch`, and `ctx snap` are deliberately outside CLI v1. hugit keeps
  the tokens reserved or the surface absent rather than pretending remote
  execution or a second context store is local.

---

## Where hugit is today

Honest status — everything here is the CLI, local, testable now:

| Capability | Status | What you get today |
|---|---|---|
| `hugit setup` — boot ceremony | **LIVE** | `git init` in any fresh repo auto-ships the hooks; first git op lazy-boots the log |
| `hugit symbol` — symbol outline | **LIVE** | `hugit symbol --file <path>` against any TS/JS/Python/Go/Java/C/C++/Ruby file |
| `hugit export` — exit guarantee | **LIVE** | full git + JSON snapshot; requires `--log <path> --out <dir>`; zero dependencies |
| `hugit check` / `hugit verdict` | **LIVE** | real policy-engine EXECUTE paths, memoized; `hugit policy test` runs the house gate set |
| `hugit campaign / intent / pr / land` | **LIVE** | the agent-fleet loop: milestones → tasks → PRs → union landing |
| `hugit dock` (worktree binding) | **LIVE** | cost per worktree; byte-identity + acceptance verified landing |
| `hugit verdict approve` / `reject` | **LIVE** | single-lens human decision over the canonical verdict record |
| `hugit undo` | **LIVE** | event-sourced compensating undo; never rewrites history |
| `hugit note` | **LIVE** | appends a record to the canonical log |
| Existing repo attachment | **LIVE** | run `hugit attach`; missing hooks install, foreign hooks stay untouched |
| `hugit fleet` / `hugit ledger` / `hugit watch` | **LIVE** | real log-backed commands |
| `hugit diag` | **LIVE** | log-backed bisect |
| `hugit policy edit` | **LIVE** | append-only policy changes over the house baseline |
| Union-tested landing queue | **LIVE (local)** | the union engine runs locally over the queue — green set lands, red pair bisects |
| Memoized checks (CI dedup) | **LIVE (local)** | file-backed memo cache: same tree+def+toolchain = HIT, zero re-execution |
| GitHub App mirror | **CODE DONE / EXTERNAL** | real push+verify lane exists; App registration and test-repo activation are outside the local CLI |
| Multi-tenant hosting | **OUT OF SCOPE** | hugit is a CLI in your repo; hosting is your git remote |

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
- jj capture is explicit through the MCP `capture` tool. Automatic observation
  after `jj git export` is not a v1 feature. `hugit capture` is internal,
  hook-only; normal onboarding never invokes it. Intent is one explicit
  declaration or a separately contracted trusted adapter.

---

## The exit guarantee

```sh
hugit export --log <git-common-dir>/hugit/event-log.json --out <dir>
```

Produces a full git bundle + JSON proof of every intent, verdict, and claim.
Restore to a bare git repo on any hosting provider. No proprietary lock-in —
the exit proof is also the disaster-recovery plan.

---

## Bring your existing repo

```sh
# hugit attaches to an existing local repo; Git remains the remote/host
cd /path/to/your/repo
hugit attach
hugit health
```

Your GitHub repo stays where it is. hugit attaches without migration. The
compat ladder: git wire protocol → landing layer riding on GitHub → bounded
bidirectional mirror → authoritative forge. You climb it at your pace; a
broken bridge kills trust, so every rung is reversible.

---

## How it works / Why it's cheap

hugit is git + a small local log. The claimed economics come from structure,
not a paid substrate:

- The **canonical log** (`<git-common-dir>/hugit/event-log.json`) is a
  hash-chained, append-only local record, outside tracked worktree state and
  shared by linked worktrees. A push attempt is not remote confirmation.
- The **memo cache** (file-backed) makes a repeated check a lookup: the same
  tree+def+toolchain is never executed twice.
- The **union landing engine** tests queued PRs as a batch locally; a red pair
  is bisected in log-depth over cached results.

hugit never charges for your compute. It runs where git runs, on your machine
or your CI, and deletes the waste git alone cannot see — the second run of the
same work.

---

## Quick CLI reference

```sh
# Inspect any file's symbol structure (works right now, no server)
hugit symbol --file src/main.rs

# Export the repo as a portable proof bundle. Get canonical log path from health.
hugit health
hugit export --log <git-common-dir>/hugit/event-log.json --out <dir>

# Show the local intent log (`hugit log` is not a verb)
hugit ledger --log <git-common-dir>/hugit/event-log.json

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
