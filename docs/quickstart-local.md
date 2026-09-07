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

## 2. `hugit setup --repo` — attach hooks to existing repo

Create repo with Git, then attach hugit hooks without changing Git data:

```sh
mkdir ~/my-project && cd ~/my-project
git init
hugit setup --repo "$PWD"
# ✓ installs hooks; first commit creates .hugit/log.json (canonical log)
```

Existing repository:

```sh
 hugit setup --repo /path/to/existing-repo
```

`hugit setup --repo` never runs `git init`, never overwrites non-hugit hooks,
and reports installed, no-op, and conflict hook names in JSON.

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

Dump a usable synthetic Git snapshot + canonical JSON envelope. `export.json`
keeps event records byte-identical; restore verifies their hash chain. Output
repo is usable with zero hugit tooling, but is not source Git history:

```sh
hugit export --log .hugit/log.json --out ./backup
# creates ./backup/repo.git  (usable synthetic Git snapshot)
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
