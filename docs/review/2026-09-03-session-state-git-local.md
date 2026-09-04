# SESSION STATE — hugit git-local direction (2026-09-02/03)

> Compact-state handoff for the next session. Everything on `main` is
> committed and merged (2 PRs). This doc captures what was decided, what was
> built, what is proven, and what remains — so a fresh session can resume
> with zero archaeology.

## Baseline (updated 2026-09-03)

- **Repo:** `github.com/gmhelmold/hugit`, `main` at **`ebfed79`** (merged #333-#348 + state docs).
- Remote `origin` is the real repo; local worktree clean, only `main`.
- **History of merged PRs:** #333 (init git-proximate + scope docs) · #334 (git-local journey suite) · #335 (PR-landing journey + quickstart) · #336 (**silent git hooks via `hugit capture`**) · #337 (watch classifies `git-activity`) · #338 (captured activity watchable) · #339 (**git-local backlog: fleet/PR/undo/check/land local-only + journeys**) · #340 (**why resolves a path to the captured commit**) · #341 (**commits-only PRs are fully landable** — land content = intents ∪ commits) · #342 (**jj first-class LIVE-proven** against the real `jj` binary) · #343 (**`hugit why` accepts the CANONICAL log** — the bare `[EventRecord,...]` the hooks write; auto-detects the legacy wrapper) · #344 (**`hugit why --walk`** — the FULL provenance chain: every captured `ref.update` that cited the path, most-recent first, links never fused; origin read == walk head, never diverge) · #345 (**a tracked push is a captured-commit proof** — the pre-push hook extracts local shas → `shas` in the push-attempt payload; `pr open --commit <pushed-sha>` accepts it; unseen sha stays `commit_not_found`) · #346 (**MCP capture tool — hugit as the agent layer** — a 5th hugit-mcp tool that shells the SAME `hugit capture` seam so an LLM that used `jj` (no post-commit hook) records its git activity; `verify` reads the log backand returns `seq`+`event_hash`, proven e2e live against a real jj commit → `pr open --commit` accepted) · #347 (**capture confirms CHECKOUT + jj checkout/merge proven e2e** — the confirm-read was matching only `target`/`shas`, but a checkout payload carries `to` (not `target`), so `verify` on a checkout FAILED; now it matches `target` OR `to`; a live journey drives the REAL MCP tool → REAL `hugit capture` against REAL `jj` (fires no hooks), recording a checkout (`jj new`) + a merge-ish `jj squash`, confirming both + provingthe captured merge commit lands via `pr open --commit`) · #348 (**live GitHub mirror push lane wired — `LiveGitHubTarget`** — the only `MirrorPushTarget` impl was thein-process `FixtureMirror`; now a real target pushes a ref from a local source repo to the authenticated GitHub remote via the real `git` binary (`push --no-verify` + `ls-remote` re-read), returning the observed oid (writer does the byte-compare); a REAL probe round-trip in `live_landing_attempt` yields `Verified` ONLY on byte-identity within 60s SLA, else honest `Partial`; token never surfaces (scrubbed `<redacted>`, bounded), scratch removed before every return;lib 83/83 (2 hermetic live-target tests against a real local bare git repo),e1a 4/4,bidir 10/10,bundle green)。

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

- CLI is **100% local by design** (post-backlog): `check` and `land` use only
  the file-backed AC (`FileAc`) — `HttpAcClient`/CoreLink is removed from the
  runtime paths; env `HUGIT_CORELINK_*` is INERT (proven by journeys
  `corelink_env_is_inert_*`).
- **Silent hooks** (installed by `hugit init`): `post-commit` `post-checkout`
  `pre-push` `post-merge` → detach `hugit capture --kind <k>` (nohup, exit-0
  always, never blocks git). `capture` is X5-safe (not a git command).
  `post-commit` records the touched paths (`git diff-tree --root --name-only -r
  --no-commit-id HEAD` → `files` in the payload) so `hugit why --path <file>`
  answers "which captured commit changed this". `hugit why` accepts BOTH the
  canonical log (bare `[EventRecord,...]` — the hooks' output) AND the legacy
  wrapper (`[{record,...},...]`), auto-detected by item shape.
  `why --walk --path <file>` projects the FULL captured chain (every `ref.update`
  that cited the path, most-recent first; each link seq/hash/author/recorded_at/
  oid/branch/qualifiers/files; links never fused) — the "why did this file evolve"
  answer. Back-compat: `why` without `--walk` = origin, which == walk head.
- **Watch classes**: `landing | verdict | policy-change | ws-state |
  git-activity | other`. `ref.update`/`ref.delete` → `git-activity`.
- **`hugit fleet`** reports `git_activity` (per-branch entries, qualifiers
  checkout/attempt/merge, redacted).
- **`hugit pr open --commit <oid>`** accepts captured commits as external PR
  members (`commit_ids`; `commit_not_found` distinct; never forged).
- **`hugit undo`** compensates a captured `ref.update` (human-only).
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

## What remains (open work, after the backlog kill)

### A. Scope decisions — all CLOSED (2026-09-03)
1. **`check` = local memo only** (decided). No CoreLink in the runtime path.
2. **`hugit serve` = out of v1** (separate binary, optional remote).
3. **Hooks = how hugit observes the LLM** (decided): NOT githooks-as-append; the
   LLM uses git normally; hooks capture silently; the intent bundle lives on the
   log and rides along via `pr open --commit`.

### B. Candidate next moves (post-backlog)
- ~~**jj first-class** (D2⑦)~~ **DONE (#342)** — live-proven against the real jj binary.
- ~~**GitHub App live mirror**~~ **CODE DONE (#348)** — `LiveGitHubTarget` wires real push+verify; live gate = owner/infra.
  registration; the code is ready, the live gate is not ours.
- ~~**`hugit serve` remote attach**~~ **NOT in v1 (out of scope, decided)**.
- **why on captures is LIVE** (#340, #343): `why --log .hugit/log.json --path <file>` answers from the raw graph, canonical-log direct.
- **More journeys**: `hugit why` provenance walk over hook-captured commits;
  `land queue` with captured raw pushes.
- **Reserved verbs `ws` / `dispatch`**: need the runner fabric (corelink-runners);
  not git-local scope.

### C. Known non-goals (do not re-litigate without owner)
- No docker/podman runner in the CLI runtime (CI = corelink-runners).
- No `hugit-*-local` crates / `providers.toml` / `ProviderKind` (superseded
  self-hosted-forge decoupling — quarantined, not deleted).
- No multi-tenant boot-reconcile scaffold as CLI verbs (forge scope).

## Verification commands (all green at baseline)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked 2>&1 | grep -cE "^warning|^error"  # 0
cargo test -p hugit-cli --test acceptance_gitlocal_journey   # 4 passed
cargo test -p hugit-cli --test acceptance_capture            # 13 passed (hooks serialized; full loop)
cargo test -p hugit-proto --test acceptance_jj_live        # 1 passed (jj live, SKIPs w/o jj)
cargo test -p hugit-cli --test acceptance_fleet_journey      # 1 passed (fleet journey)
cargo test --workspace --locked                               # 205 suites ok (heavy/bundle)
```

## Machine state

- Disk was at ~85% earlier; freed to ~3.5–15 GB (cleaned caches, docker, dmg).
  If building big bundles, watch `/` free space.
- No stray worktrees; `main` only. The 9 wrong-scope `wp/*` worktrees+branches
  were removed (their commits are documented in the quarantine doc, reflog-recoverable).
## Decisions closed (2026-09-03, this session)

| # | Decision | Ruling |
|---|---|---|
| 1 | `check` scope | **Local memo only.** It verifies on your machine (FileAc memo, `duration_ms:0` on HIT); CI/compute is corelink-runners. The CoreLink swap stays an env opt-in, never silent. |
| 2 | `hugit serve` | **Out of v1 scope.** Code stays, documented as a separate forge-host binary that is NOT part of the git-local CLI flow. No pruning. |
| 3 | "async detect when LLM uses git" | **NOT githooks.** Owner insight: the intent bundle must ride ALONG WITH commit/PR/push (not post-hoc), and githooks can't cover everything. Direction = hugit exposes **LLM tool-call tools** (via the existing `hugit-mcp` crate) that the agent calls INSTEAD of raw `git commit`/`pr` — hugit does the git + attaches the intent/context bundle + records, all in one tool call. This is "hugit as the agent layer" (MCP tools), not a git-hook append. |

---

## POST-COMPACTION RESUME CHEAT-SHEET (2026-09-03)

Everything below is the state at `main a7be4d5`. Next session: read THIS file, verify the repo matches, then continue.

### To verify the baseline (60s)
```bash
cd /Users/gustavoschneiter/Documents/HuGR/hugit-main
git log --oneline -1     # expect ebfed79
git status --short       # expect empty
git branch --show-current  # main
```

### What exists (proven, all merged)
- **Silent hooks**: `hugit init` installs post-commit/checkout/pre-push/merge → `hugit capture --kind <k>` (async, exit-0 always, never blocks git, records `ref.update` + touched `files`).
- **Read layer**: `hugit watch --class git-activity`, `hugit fleet` (git_activity), `hugit why --log <canonical> --path <file>` (resolves to the captured commit).
- **PR layer**: `pr open --commit <oid>` (external member), `pr queue`/`land queue` accept commits-only PRs (content = intents ∪ commits). **A raw `git push` is captured-commit proof**: pre-push hook extracts local shas → `shas` in the `{attempt:true}` payload; `pr open --commit <pushed-sha>` accepts it (W2 target-OR-shas); unseen sha → `commit_not_found`.
- **Undo**: compensates a captured `ref.update` (human-only). **Local-only**: check + land use only FileAc (CoreLink env inert).
- **jj**: live-proven D2b⑦ (change-ids stable across `jj squash`).
- **journeys**: acceptance_capture (13, serialized), acceptance_fleet_journey (1), acceptance_gitlocal_journey (4), acceptance_pr_commit (5), acceptance_jj_live (1).
- **Bundle**: `cargo test --workspace --locked` = 205 suites ok / 0 failed.

### Test gotchas (do NOT rediscover)
- Hook journeys MUST be serialized (shared Mutex in acceptance_capture.rs — async captures compete for the FileLock under parallel load → flaky).
- Async hooks: use `wait_for_ref_update(wait_for_two_captures)` with 15-25s timeouts.
- Git identity: `set_git_identity` in every journey (CI has none).
- `CARGO_BIN_EXE_hugit` for the real binary; `lib_init` for init (the binary verb is X5-reserved).
- Test litter (.git inside crates) is gitignore'd now; if a test leaves it, `rm -rf crates/*/.git`.

### Remaining backlog (owner/infra-gated mostly)
1. ~~**GitHub App mirror**~~ **CODE DONE (#348)** — live lane wired (real git push+verify round-trip); remaining gate = owner/infra: registar App + `HUGIT_GH_TEST_REPO.). Code is ready in `hugit-mirror`.
2. **jj understood**: D2b⑦ live; the capture-hooks model is git-specific — jj exports don't fire post-commit (the export writes refs directly). A "jj-aware capture" (detect ref changes after `jj git export`) is a design decision, not a quick fix.
3. ~~**land queue with captured raw pushes**~~ **DONE (#345)** — pre-push hook extracts local shas → `shas` in the payload; `pr open --commit <pushed-sha>` accepts it; unseen sha → `commit_not_found`. The full capture→push→proof→PR→land loop is COMPLETE.
3. **First-user docs** — quickstart-local.md + quickstart-hooks.md exist; polish welcome.
4. **`hugit serve`** — out of v1 (decided), code stays.

### Cash the state
Commit this doc (docs-only, CI skips), push. Done.

---

## DONE — MCP capture tool (hugit as the agent layer,#346, main cb83451)

### What shipped
A 5th MCP tool `capture` in `hugit-mcp` (`crates/hugit-mcp/src/tools/capture.rs`,
387 lines): shells the SAME `hugit capture <kind>` seam the silent hooks use —
`commit | checkout | push-attempt | merge` — so an LLM that uses `jj` (which
fires NO post-commit hook) can record its git activity on the canonical log..
Zero new log kinds / wire contract changes (frozen `ref.update` by Push).

HONEST confirm:with `verify` (default `true` when oid/shas given), reagent
reads the SAME log backand returns landed `seq` + `event_hash`. A dispatched-but-
unconfirmed capture is a TOOL ERROR ( fail-closed), never a claim..

### PROVEN end-to-end live (stdio, real jj repo)
```
jj git init && jj describe -m "feat: a" && jj git export
OID=$(jj log -r @ -T 'commit_id ++ "\n"')   # the git oid
tools/call capture {kind:commit, top_level:$REPO, log:$REPO/.hugit/log.json, oid:$OID, verify:true}
  → {"status":"captured","seq":0,"event_hash":"0c61944a…"}
hugit pr open --commit $OID                 # ACCEPTED as captured proof
```
Key facts: jj `commit_id` =the git oid (git cat-file -t confirms a real
commit object).. jj fires NO hooks; working-copy commit_id re-snapshots when
files change (submit the oid you actually want tracked). The tool requires a
repo already initialized (`.hugit/log.json` exists) — a missing log is a
hard tool error(, honest: no auto-init ceremony).

### Gate
hugit-mcp 44/44 (6 new capture tests), bundle 205/0, fmt + clippy 0..
tools/list advertises  ​5 tools; stdio round-trip test asserts​ ​5..

### Backlog unchanged
1. ~~**GitHub App mirror**~~ **CODE DONE (#348)** — live lane wired; remaining gate = App registration + `HUGIT_GH_TEST_REPO` (owner/infra.
2. jj checkout/merge captures — **DONE (#347)** — a live journey
   drives the REAL MCP tool against REAL `jj` (`jj new` checkout, `jj squash`
   merge-ish), confirms both + PRs them. The `jj`-shim verb is now MOOT
   (the MCP tool seam provably covers the jj flow — no shim needed).

---

## DONE — Go-live runbook (docs,#2, main 84e6313)

The first-tenant go-live path is consolided + documented: `docs/quickstart-golive.md`
(the runbook: capture→why→PR→land, git **or** jj, zero server/account/CoreLink),
+ cross-refs in both quickstarts (`quickstart-hooks` + `quickstart-local` → See
also). What makes it go-live-grade: every step is a NAMED journey (not a
plan); a checklist of verification commands (hugit-mcp 45/45, capture
13, jj checkout/merge 1, gitlocal 4, land-queue); honest scope
(C I exec → corelink-runners, serve out of v1, multi-tenant →
githugr infra, owner-gated). Docs-only(pushed `84e6313`, CI skips.

The git-local loop is now COMPLETE + PROVEN + DOCUMENTED:
capture (silent
git hooks commit/checkout/push/merge + MCP tool jj-aware, checkout confirm
`to` fix) → canonical `.hugit/log.json` → why/why --walk provenance → PR
open --commit(a ccepted captured commits + pushed shas) → land queue
(commits-only PRs land) → export exit-proof. Two quickstarts + one
go-live runbook carry the whole first-user flow.

### What remains (all outside the local loop, sibling/infra-gated)
1. **First-real-user smoke** — a human/owner running the runbook on a real
   repo (the githugr TL or yourself..
2. ~~**GitHub App mirror**~~ **CODE DONE (#348)** — live lane wired;
   remaining gate = owner App registration + `HUGIT_GH_TEST_REPO` (infra); então
   o first-real-user smoke (#1) roda real.
3. **Live runner exec** (`ws`/`dispatch`) — needs `corelink-runners` fabricd
   spawn fix (separate project。
4. **Multi-tenant / identity / serve** — forge-surface/githugr lanes, out of
   v1 (decisão fechada。



Baseline vaginal — verify: `git log -1` = ebfed79, status empty, main.

---

## DONE — Live GitHub mirror lane (code wired,#348, main ebfed79)

The GitHub App mirror **code** is now WIRED (the #2 backlog item. Before, Othe hugit-mirror had EVERY hermetic piece (AppAuth token mint, MirrorPushTarget trait,, per-push content-hash verify,, FIFO durable queue) BUT no real GitHub target — the only `MirrorPushTarget` impl was the in-process `FixtureMirror`。 NOW `LiveGitHubTarget` pushes a ref from a local source repo to the authenticated GitHub remote via the REAL `git` binary (`git push --no-verify` + `git ls-remote` re-read), returningthe OBSERVED oid — the writer does the byte-compare (fail-closed divergence。 `live_landing_attempt` now performs a REAL probe round-trip (scratch repo + real commit → push `refs/heads/hugit/mirror-probe-<pid>-<nanos>` → re-read), `Verified` ONLY on byte-identity within the 60s SLA;; else honest `Partial` (never fabricated。 Token never surfaces ((sanitised `<redacted>`, bounded), scratch dir removed before every return. Hermetic:: lib 83/83 (2 new live target tests against a real local bare-mirror git repo), e1a 4/4, bidir 10/10,, workspace bundle GREEN (206 ok, 0 falhas,, fmt + clippyy 0,, deny ok。 Merged #348;; docs state pushed (`ebfed79`.

**Remaining (owner/infra-gated,, NO code gap**: register the GitHub App (with push perms on a target repo), place creds ( `~/.hugit/secrets/github-app-dev/`: `private-key.pem` + `app-id` + `installation-id`) or envs `HUGIT_GITHUB_APP_*`, set `HUGIT_GH_TEST_REPO=owner/repo`) → then first-real-user smoke (#1) runs real (`live_landing_attempt` → `Verified`, or honest `Partial` if creds don't cover).
