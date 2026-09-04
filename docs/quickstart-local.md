# hugit quickstart — local, git-native, no server, no account

> hugit is a **git-local CLI**: it works where git works — in a repository, on
> a laptop, with no server, no account, and **zero CoreLink in the default
> path**. This is the exact first-user flow, proven by
> `crates/hugit-cli/tests/acceptance_gitlocal_journey.rs`.

---

## 1. Install + build

```sh
git clone https://github.com/gmhelmold/hugit.git
cd hugit
cargo build --release
# the binary:
./target/release/hugit --version
```

## 2. `hugit init` — the git-proximate bootstrap

In a **new** directory, `hugit init` runs the same ceremony as `git init` (it
creates a real git repository) and then adds the hugit layer:

```sh
mkdir ~/my-project && cd ~/my-project
hugit init
# ✓ creates .git/ (git init) + .hugit/ + .hugit/log.json (the canonical log)
```

In an **existing** git repository it only adds the hugit layer — your `.git`
is never touched:

```sh
cd /path/to/existing-repo   # already a git repo
hugit init                  # adds .hugit/ + log; leaves .git alone
```

> **Note:** `hugit init` is exercised through the library entry point; it is
> not a top-level binary verb because the X5 namespace law forbids a hugit verb
> that shadows a git command (`git init` exists). The behavior is identical.

## 3. `hugit check run` — memoized local verification

Run a command, memoized locally. Cold run executes; a warm re-run is a HIT with
**zero execution time** — all on your machine, no network, no CoreLink:

```sh
hugit check run --def ad-hoc --cmd "echo hello" --store
# cold: {"cache_hit": false, ...}
hugit check run --def ad-hoc --cmd "echo hello"
# warm: {"cache_hit": true, "duration_ms": 0, ...}
```

`check` is local-only by default (file-backed AC). A shared cache is an
explicit opt-in via env, never a silent network call.

## 4. `hugit symbol` — instant code outline

Works on any source file, standalone, no server:

```sh
hugit symbol --file src/lib.rs
```

Supported: TypeScript, JavaScript, Python, Go, Java, C, C++, Ruby.

## 5. `hugit export` — the zero-lock-in exit proof

Dump your repo's git artifact + the full JSON envelope. The output is a **real
git repository**, usable with zero hugit tooling:

```sh
hugit export --log .hugit/log.json --out ./backup
# creates ./backup/repo.git  (a real git repo)
#          ./backup/export.json
#          ./backup/redaction-manifest.json
```

## 6. The PR landing journey (all local)

The full intent → PR → land cycle runs entirely on the local log:

```sh
# 1. record an intent
hugit intent new --charter "my feature" --campaign my-campaign --log .hugit/log.json
#   → captures the returned intent id (e.g. intent-3f5b…)

# 2. open a PR bundling it
hugit pr open --pr PR-1 --campaign my-campaign \
  --author-kind orchestrator --run-id run-1 \
  --intent intent-3f5b… --log .hugit/log.json

# 3. queue it for the union-testing landing queue
hugit pr queue --pr PR-1 --log .hugit/log.json

# 4. batch-land the queue (green → lands)
hugit land queue --campaign my-campaign --log .hugit/log.json
#   → {"verdict": "green", "landed": ["PR-1"], ...}

# 5. show the PR's terminal state
hugit pr show --pr PR-1 --log .hugit/log.json
```

## 7. Explore the rest

Everything you can do with the local log:

| Verb | What it does |
|---|---|
| `why` / `impact` | resolve a line/symbol → originating intent; blast radius |
| `verdict approve/reject` | record an adversarial review verdict |
| `undo` | compensating undo (human-only, event-sourced) |
| `policy` | declarative gate management |
| `ledger` / `watch` | the forge history view; replayed redacted event stream |
| `fleet` | machine-readable fleet/workspace/agent state |
| `diag` | bisect a red check history (log-backed) |
| `ctx resume` | short-horizon session resume |
| `review` | grounded Q&A over the log — cite-or-refuse |

## 8. What is NOT in hugit (by design)

- **CI / compute execution** lives in the separate `corelink-runners` project.
  hugit records the *demand*; it does not run a runner fabric.
- **`hugit serve`** (the smart-HTTP forge host) is a separate binary and an
  optional remote — not part of the git-local CLI flow.
- No account, no server, no CoreLink credentials are required for any of the
  above.

---

*Verified on `main` — the journey suite (`acceptance_gitlocal_journey.rs`) runs
these exact steps and asserts the outcomes.*
---

## See also

- **[`docs/quickstart-golive.md`](quickstart-golive.md) —the go-live runbook:

  the complete capture → why → PR → land loop, inclusive jj / MCP `capture`
  tool e2e, `why --walk` provenance, land of commits-only PRs, exit proof..
- [`docs/quickstart-hooks.md`](quickstart-hooks.md) —the silent-hooks flow:



  git captures in the background when you use git normally (+ undo a
  mistaken capture).
