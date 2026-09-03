# HANDOFF — hugit as a git-local CLI: validated catalog + scope correction (2026-09-02)

> For the **next session**. This repo was cloned fresh from
> `https://github.com/gmhelmold/hugit` (`main 15f34b7`). The owner's direction
> (2026-09-02): **hugit is a forge like git — a CLI that works where git works.**
> CI is NOT part of hugit; CI/runner execution belongs to the separate
> `corelink-runners` project. Do not rebuild the CI/compute layer here.

## Owner decision (verbatim intent)

- "eu quero que seja igual o git. CI e outra coisa, e corelinkrunners, e
  projeto separado. O hugit e pra trabalhar onde o git trabalha"
- Preceding: the earlier 10-WP "go-live v2" plan (local AC/CAS/docker
  runner/identity/migrate/cost/boot-reconcile — `docs/plan/2026-09-02-go-live-v2-parallel.md`)
  is **superseded as wrong-scope**. Those WPs assumed a decoupling / self-hosted
  forge-hosting angle. The owner rejected it: hugit = git-local CLI.

## What this repo actually IS (validated this session, `main 15f34b7`)

19-crate Rust workspace. Real code, zero `todo!()`/`FIXME`/`unimplemented!()`.
The CLI (`crates/hugit-cli`) is a real dispatched verb surface, NOT stubs.

### CLI verb catalog (validated in `crates/hugit-cli/src/main.rs:67-130`, all real)

| Verb | Class | What it does |
|---|---|---|
| `why` | D10 | resolve line/symbol → originating intent + provenance |
| `impact` | D10 | build-graph blast radius of changed paths |
| `tournament` | D13 | fan intent out to N candidates (budget-bounded) |
| `export` | E5 | dump git artifact + JSON envelope (zero-dep exit proof) |
| `campaign` | D5 | open / close (seal) / show |
| `intent` | D4 | new / show / list |
| `issue` | D | issue lifecycle transition |
| `pr` | D | open / queue / land / show / list / abandon |
| `land` | B4 | batch land: union-test + bisect + memoize over the queue |
| `meta` | D | repo metadata: visibility + owning tenant |
| `queue` | B4 | landing-queue state |
| `check` | B2 | memoized CI check (see caveat: CI compute is corelink) |
| `verdict` | D7 | adversarial review panel / approve / reject |
| `undo` | D14 | compensating undo event (human-only) |
| `policy` | D6 | declarative gate management |
| `note` | D11 | append session note to canonical log |
| `diag` | B5 | bisect red check history (log-backed) |
| `ledger` | D5 | default forge history: asked → done → proven |
| `fleet` | D5 | machine-readable fleet state (workspaces + agents) |
| `watch` | D5 | replay classified, redacted event stream |
| `symbol` | W6 | outline local source file symbols (tree-sitter) |
| `ctx` | D11 | short-horizon session resume |
| `review` | D7 | grounded Q&A over log, cite-or-refuse |
| `init` | — | create `.hugit/` + canonical event log |

### What is NOT part of hugit (owner-decided)

- **docker/podman runner** (WP-13): not in runtime. Validated zero docker use
  in CLI runtime (only `hugit-fence/src/broker` + `seam.rs` reference it, and
  that is the runner-transfer leftover). CI computes belong to corelink-runners.
- **LocalAc / LocalCAS / providers.toml / ProviderKind** (WP-05,10,11,12,15):
  decoupling-for-hosting scope, rejected.
- **cost-honesty / multi-tenant boot-reconcile** (WP-16,17): githugr/CoreLink
  forge-hosting scope, not git-local CLI scope.

## What already works standalone (no server, no CoreLink)

- `hugit init` → `.hugit/` + canonical log (in `crates/hugit-cli/src/init/mod.rs`)
- All verbs above dispatch genuinely on `main` (`Command::*` match,
  `main.rs:549-569`)
- `hugit export` is the documented zero-lock-in exit proof
- `hugit serve` exists (`crates/hugit-serve`) but note: it is a **forge host**
  (smart-HTTP clone/fetch/push over CAS). Owner said hugit is a CLI "where git
  works" — whether `hugit serve` is in scope needs re-confirmation; the owner
  statement points at the CLI, not a hosting server.

## What is genuinely missing for "git-like CLI"

This needs a fresh session to validate against the owner's exact UX, but a
first-pass gap list from this session:

1. **`hugit init` currently only makes `.hugit/` + log** — it does NOT do the
   git-proximate ceremony (does not wrap `git init`, no `.hugit/` config
   scaffold, no first checkout). Confirm whether `hugit init` should either
   just register into an existing `.git`, or do nothing about `.git` at all.
2. **`hugit check` depends on the CoreLink AC/http-compute seam for the memoized
   path** — the CLI's AC defaults to `HttpAcClient` requiring credentials. For a
   git-local tool, `hugit check` running purely local (file-backed AC) may be
   wanted, OR check may be declared out of scope (owner: "CI é outra coisa").
   **Decide and remove/determinize the `check` verb accordingly.**
3. Any verb that joins "CI" or "hosted forge" concepts should be flagged for
   removal or re-scoped to local-only. `land --dispatch`, `fleet` (agent state),
   `check` are the candidates.

## Machine state for next session

- Git repo OK: `main 15f34b7`, remote `origin https://github.com/gmhelmold/hugit`
- Untracked local artifacts (safe to delete or commit): 
  - `docs/plan/2026-09-02-go-live-v2-parallel.md` (superseded plan, keep as
    history or remove)
  - `docs/plan/wp-specs/` (empty scaffold dir + `fix_results/`; remove)
- **Leftover worktrees on WRONG branches** (`git worktree list`):
  - wp-05, 10-13, 16 have commits (wrong-scope WP work), wp-14/15/17 are
    empty. Recommend `git worktree remove --force` all of
    `../hugit-wp-{05,10,11,12,13,14,15,16,17}` + `git branch -D` the wp/*
    branches before starting fresh on the git-local direction.
- Disk ~3.5 GB free on `/`.

## Recommendation

Rebase the roadmap to "git-native CLI for agent-era version control" — version
intent/context/proof locally inside a normal git repo, ride git's object model,
zero CoreLink in the default path. The 24-verb local surface is already the
product; the work is **scoping/pruning** (remove CI/forge-hosting verbs) +
**git-proximate UX** (init/checkout flow) + **tests/journeys that prove using**
it like git. Start from the audit at `docs/review/2026-06-17-honest-delivery-audit-double-checked.md`
for what is hermetic vs live.