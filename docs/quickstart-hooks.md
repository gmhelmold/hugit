# hugit quickstart — silent git hooks, zero-friction agent capture

> The complete first-user flow for git-local hugit: install, hook setup, and
> the silent hooks), and WATCHING what your agent fleet does to the git graph —
> with the agents using `git` normally, never learning a hugit command.

## 1. Install + hook setup

```sh
git clone https://github.com/gmhelmold/hugit.git
cd hugit
cargo build --release

# In an existing Git repo:
hugit setup --repo /path/to/repo
# installs post-commit / post-checkout / pre-push / post-merge;
# first Git operation creates .hugit/log.json
```

`hugit setup --repo` never initializes Git or overwrites non-hugit hooks.

## 2. Use git NORMALLY — hugit captures silently

**Your agents (or you) run git as usual.** Every command is captured in the
background by a silent hook — no wrapper, no new tool, no friction:

```sh
git commit -m "feat: x"                # → ref.update {branch, oid} on the log
git checkout -b feat/agent            # → ref.update {checkout:true}
git push origin feat/agent            # → ref.update {attempt:true}
git merge feat/agent                  # → ref.update {merged_from}
```

**Hard guarantees (the silent contract):**
- a hugit hook **never fails or blocks git** — even if hugit is missing/broken, the commit/push proceeds (exit 0 always);
- the hooks are **asynchronous** (a detached background child) — git never waits;
- nothing is printed — the capture is silent;
- nothing is invented — the events are the raw `ref.update` kind with honest
  payload qualifiers (`checkout`/`attempt`/`merged_from`), never a forged intent.

## 3. See what the fleet did

```sh
# The raw agent trace, classified:
hugit watch --log .hugit/log.json --class git-activity

# The machine-readable fleet state, now including git_activity per branch:
hugit fleet --log .hugit/log.json

# The forge history (asked → done → proven):
hugit ledger --log .hugit/log.json
```

## 4. The PR bundle rides on the log

When an orchestrator opens a PR with the intents + the captured commits:

```sh
hugit pr open --pr PR-1 --campaign my-campaign \
  --author-kind orchestrator --run-id run-1 \
  --intent <intent-id> \
  --commit <captured-commit-oid> \
  --log .hugit/log.json
```

A captured commit oid is accepted as an **external member** of the PR
(`commit_ids` in the payload) — never forged into an intent.

## 5. Undo a mistaken capture (human-only)

```sh
hugit undo --seq <n> --actor user:you   # compensates a captured ref.update
```

## 6. What is NOT in hugit (by design)

- **CI / compute execution** lives in corelink-runners (separate project). hugit
  records the demand; it does not run a runner fabric.
- **`hugit serve`** (the smart-HTTP forge host) is a separate binary and an
  optional remote — not part of the git-local CLI flow.
- No account, no server, no CoreLink credentials are required.

---

*Verified on `main` — the capture journeys (`acceptance_capture.rs`,
`acceptance_fleet_journey.rs`) run these exact steps with REAL git + the REAL
binary and assert the outcomes (6 + 1 tests, stable).*

---

## See also

- **[`docs/quickstart-golive.md`](quickstart-golive.md)** —the go-live runbook:
  the complete capture → why → PR → land loop, inclusive jj / MCP `capture`
  tool, `why --walk` provenance, land of commits-only PRs, exit proof..
- [`docs/quickstart-local.md`](quickstart-local.md) —the intent-first PR
  journey (intent new → pr open → queue → land → show).
