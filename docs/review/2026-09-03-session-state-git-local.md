# SESSION STATE — hugit git-local direction (2026-09-02/03)

> Compact-state handoff for the next session. Everything on `main` is
> committed and merged (2 PRs). This doc captures what was decided, what was
> built, what is proven, and what remains — so a fresh session can resume
> with zero archaeology.

## Baseline

- **Repo:** `github.com/gmhelmold/hugit`, `main` at **`ab37af9`** (merged #333 + #334).
- Clone is fresh; history intact; remote `origin` is the real repo.
- Local worktree clean, only `main` branch checked out.

## Owner direction (verbatim, 2026-09-02)

> "eu quero que seja igual o git. CI e outra coisa, e corelinkrunners, e
> projeto separado. O hugit e pra trabalhar onde o git trabalha"

**Interpretation applied (recommendations accepted by owner):**
1. hugit = git-local CLI (works where git works): no server, no account, no
   CoreLink in the default path.
2. CI/compute execution = separate project (`corelink-runners`), NOT rebuilt
   here.
3. Nothing deleted — superseded work is documented (quarantine), reincubable.

## What was merged

### PR #333 — `feat/git-local-scope` → main (`146658b`)
- `hugit init` is **git-proximate**: runs `git init` on a bare dir, leaves an
  existing `.git` untouched, adds `.hugit/` + canonical log idempotently
  (`crates/hugit-cli/src/init/mod.rs`). New JSON fields `git_created` /
  `git_hint`. 3 init lib tests green.
- Scope record: `docs/quarantine/2026-09-02-scope-decisions.md` (superseded
  9-WP plan + reasoning, nothing deleted).
- Session handoff: `docs/review/2026-09-02-hugit-cli-local-catalog-handoff.md`.
- README architecture clarity (git-local CLI; check local; CI → corelink-runners;
  `hugit serve` = optional remote/forge host, separate binary).

### PR #334 — `feat/gitlocal-journey-test` → main (`ab37af9`)
- **`crates/hugit-cli/tests/acceptance_gitlocal_journey.rs`** — the git-local
  journey acceptance suite, proves through the REAL binary + library:
  1. `hugit init` is git-proximate (via lib `init::run` — see X5 note).
  2. `hugit check run` is local + memoized: cold MISS, warm HIT (`duration_ms:0`),
     zero CoreLink.
  3. `hugit export` is the zero-lock-in exit proof (real git repo artifact).
- 4/4 journey tests green. Workspace bundle: **201 suites ok, 0 failed**.
  X5 no-shadow 3/3. Clippy 0, fmt clean.

## Key discovery (important for future)

**`hugit init` is NOT a binary verb — and must not become one.** The X5
namespace law (`crates/hugit-invariants/x5`) forbids any hugit verb that
shadows a `git` command; `git init` exists, so `init` cannot be a hugit verb.
The module was documented "logic ready; pending an X5 namespace amendment".
The git-proximate behavior is proven via the **library** `init::run`; the
binary verb stays out of `HUGIT_VERBS` and `main.rs` dispatch. Do NOT re-add
it as a top-level verb.

## Facts verified about the codebase (use these, don't re-discover)

- CLI is **already 100% local by default**: `check` default AC is `FileAc`
  (`checks/run.rs:29`, `:1265`), `HttpAcClient` (CoreLink) is only an explicit
  env opt-in, never a silent network call. Zero runtime CLI reference to
  `HttpAcClient`/`HUGIT_CORELINK_*`/`fabricd`.
- 24 verbs real in the CLI dispatch (`main.rs`): why impact tournament export
  campaign intent issue pr land meta queue check verdict undo policy note diag
  ledger fleet watch symbol ctx review.
- Reserved (NOT shipped, need runner fabric): `ws`, `dispatch`.
- `check` subcommands: `run | show | key` (NOT `--cmd` directly).
- `hugit serve` is a **separate binary** (the forge host), not a CLI verb.
- `CHANGELOG.md` gate: any `feat/fix` commit MUST update CHANGELOG or CI fails.
- DCO gate: every commit needs `Signed-off-by:` + `Co-Authored-By:` trailer
  (Repo pattern: `Signed-off-by: Gustavo Schneiter <gustavo@humangr.com>` +
  `Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>`).
- `gh` authenticated as `gmhelmold`.

## What remains (open work for next session)

### A. The 3 open scope decisions (from quarantine §5)
1. **`check` verb scope**: keep local-only (memo on your repo, execute on your
   machine) or re-sit as CI (→ corelink-runners)? Current: local-by-default and
   it works; not yet owner-decided whether "CI é outra coisa" also excludes the
   local memo path.
2. **`hugit serve`**: optional remote/forge host (separate binary) — in or out
   of the git-local product scope? Docs positioned as optional; owner consent
   pending.
3. **git-proximate UX beyond init**: e.g. checkout/branch ergonomics, wrapping
   `git` commands with the intent layer — not yet scoped.

### B. Candidate next moves (git-local direction)
- **Check-resident decisions** per A.1.
- **More git-local journeys**: e.g. `hugit pr open` → `land queue` (union) →
  `watch`; `hugit symbol` on a real repo; `hugit why` provenance walk.
- **`hugit serve` as optional remote**: document "attach a server to a local
  repo" path; verify read-only vs push posture.
- **Docs for first user**: `quickstart-local.md` (download → init → check →
  export) using only local providers.

### C. Known non-goals (do not re-litigate without owner)
- No docker/podman runner in the CLI runtime (CI = corelink-runners).
- No `hugit-*-local` crates / `providers.toml` / `ProviderKind` (the superseded
  self-hosted-forge decoupling plan — quarantined, not deleted).
- No cost-honesty / multi-tenant boot-reconcile scaffold as CLI verbs (forge
  scope).

## Verification commands (all green at baseline)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked 2>&1 | grep -cE "^warning|^error"  # 0
cargo test -p hugit-cli --test acceptance_gitlocal_journey   # 4 passed
cargo test --workspace --locked                               # 201 suites ok (heavy/bundle)
```

## Machine state

- Disk was at ~85% earlier; freed to ~3.5–15 GB (cleaned caches, docker, dmg).
  If building big bundles, watch `/` free space.
- No stray worktrees; `main` only. The 9 wrong-scope `wp/*` worktrees+branches
  were removed (their commits are documented in the quarantine doc, reflog-recoverable).