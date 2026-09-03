# SESSION STATE — hugit git-local direction (2026-09-02/03)

> Compact-state handoff for the next session. Everything on `main` is
> committed and merged (2 PRs). This doc captures what was decided, what was
> built, what is proven, and what remains — so a fresh session can resume
> with zero archaeology.

## Baseline (updated 2026-09-03)

- **Repo:** `github.com/gmhelmold/hugit`, `main` at **`d8021a8`** (merged #333-#345 + state doc).
- Remote `origin` is the real repo; local worktree clean, only `main`.
- **History of merged PRs:** #333 (init git-proximate + scope docs) · #334 (git-local journey suite) · #335 (PR-landing journey + quickstart) · #336 (**silent git hooks via `hugit capture`**) · #337 (watch classifies `git-activity`) · #338 (captured activity watchable) · #339 (**git-local backlog: fleet/PR/undo/check/land local-only + journeys**) · #340 (**why resolves a path to the captured commit**) · #341 (**commits-only PRs are fully landable** — land content = intents ∪ commits) · #342 (**jj first-class LIVE-proven** against the real `jj` binary) · #343 (**`hugit why` accepts the CANONICAL log** — the bare `[EventRecord,...]` the hooks write; auto-detects the legacy wrapper) · #344 (**`hugit why --walk`** — the FULL provenance chain: every captured `ref.update` that cited the path, most-recent first, links never fused; origin read == walk head, never diverge) · #345 (**a tracked push is a captured-commit proof** — the pre-push hook extracts local shas → `shas` in the push-attempt payload; `pr open --commit <pushed-sha>` accepts it; unseen sha stays `commit_not_found`).

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
- **GitHub App live mirror** (`HUGIT_GH_TEST_REPO`): needs owner/infra App
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

Everything below is the state at `main d8021a8`. Next session: read THIS file, verify the repo matches, then continue.

### To verify the baseline (60s)
```bash
cd /Users/gustavoschneiter/Documents/HuGR/hugit-main
git log --oneline -1     # expect d8021a8
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
1. **GitHub App mirror** — needs `HUGIT_GH_TEST_REPO` + a registered GitHub App (owner/infra). Code is ready in `hugit-mirror`.
2. **jj understood**: D2b⑦ live; the capture-hooks model is git-specific — jj exports don't fire post-commit (the export writes refs directly). A "jj-aware capture" (detect ref changes after `jj git export`) is a design decision, not a quick fix.
3. ~~**land queue with captured raw pushes**~~ **DONE (#345)** — pre-push hook extracts local shas → `shas` in the payload; `pr open --commit <pushed-sha>` accepts it; unseen sha → `commit_not_found`. The full capture→push→proof→PR→land loop is COMPLETE.
3. **First-user docs** — quickstart-local.md + quickstart-hooks.md exist; polish welcome.
4. **`hugit serve`** — out of v1 (decided), code stays.

### Cash the state
Commit this doc (docs-only, CI skips), push. Done.

---

## IN-PROGRESS — MCP capture tool (jj-aware capture direction) — NEEDS RESUME

### ⚠️ STATE AT MAIN `d8021a8`: WORKTREE DIRTY — UNSTAGED, UNCOMMITTED
```
 M crates/hugit-mcp/src/lib.rs          (3-line test: tools list len 4→5)
 M crates/hugit-mcp/src/server.rs       (dispatch capture + spec + instructions)
 M crates/hugit-mcp/src/tools/mod.rs    (mod capture + doc)
?? crates/hugit-mcp/src/tools/capture.rs (NEW 387-line tool)
```
Nothing committed yet. **IMPORTANT: the capture.rs new file is UNTRACKED — any
stash/reset must preserve it.** Do NOT commit this partial state to main as a
"half-PR"; finish the work then branch.

### What the in-progress work IS
The **jj-aware capture design decision** got a concrete materialization: a 5th
MCP tool `capture` in `hugit-mcp` that shells the REAL `hugit capture` seam
(the same one the silent git hooks use) so an LLM that used `jj` (which fires
NO post-commit hook — `jj describe` + `jj git export` write refs directly,
proven empirically with a real `.git/hooks/post-commit` that did NOT fire) can
record its git activity on the canonical log. This is the start of scope
decision #3's "hugit as the agent layer" (MCP tools IN PLACE of raw git/jj).

### Done so far (verify each before trusting)
- `capture.rs`: shells `hugit capture --kind ... --top-level ... --log ...`
  (all opts: oid/branch/from/recorded_at/refspecs/shas/files/hook_log), silent
  exit-0 contract honored (non-zero exit → tool error).
- Honest verify path: with `verify:true` (DEFAULT true), reads the SAME
  canonical log back and returns the landed ref.update `seq` + `event_hash`
  (never a bare "dispatched" promise). `confirm_on_log` matches payload
  `target` OR `shas` (whitespace-separated).
- server.rs: `capture` routed in tools/call + tool_specs + initialize
  instructions + tools/list test 4→5 tools.
- lib.rs stdio round-trip test: tool count 4→5.
- **ALL hugit-mcp tests pass: 44/44. clippy 0 happened but RE-RUN after resume.**
- Real-e2e: `cargo build -p hugit-mcp --bin hugit-mcp` compiles.

### What's NOT done (the resume point)
1. **The e2e journey end-to-end** was STARTED but INTERRUPTED mid-probe: a real
   jj repo in `/var/folders/lt/z11pyzhj0m17vn798jkk69hh0000gn/T/opencode/hg-jj`
   (`jj git init` + describes), the jj `commit_id` = git sha (probe succeeded:
   `jj log -r @ -T 'commit_id ++ "\n"'` → 40-hex git sha). Was about to: create
   a hugit log, call the real `hugit-mcp` stdio `tools/call capture` with the
   jj commit_id + verify:true, assert the ref.update lands + why-able.
2. **No CLI journey added yet** to `acceptance_capture.rs`/`acceptance_pr_commit.rs`
   proving "a jj commit captured via the MCP capture tool is pr-open-able". The
   acme of this work: capture a real jj commit through the tool, then
   `pr open --commit <commit_id>` + `land queue` — closing the jj loop.
3. **CHANGELOG** not updated.
4. **fmt/clippy** — run AFTER the e2e journey + any final edits.
5. **Branch + PR** — `feat/mcp-capture-tool`.

### The jj facts (proven, use them)
- `jj` v0.42.0 at `/usr/local/bin/jj`. `jj git init` repo has a `.git/` +
  `.jj/`. **`jj describe` does NOT fire post-commit** even with a hook installed
  (empirical proof). `jj git export` (or auto-export) writes refs behind.
- jj `change_id` (stable across squash) ≠ jj `commit_id` (the git sha). For
  capture we use `commit_id` (the git oid). `pr open --commit` wants the git oid.
- `jj log -r @ --no-graph -T 'commit_id ++ "\n"'` → the 40-hex git sha.

### How to finish the e2e (the verify path)
```bash
# scratch a jj repo
cd /var/folders/lt/z11pyzhj0m17vn798jkk69hh0000gn/T/opencode && rm -rf hg-jj2 && mkdir hg-jj2 && cd hg-jj2
jj git init && jj config set --user user.name Test && jj config set --user user.email t@t.com
echo x > a.txt && jj describe -m "feat: a" && jj git export
OID=$(jj log -r @ --no-graph -T 'commit_id ++ "\n"' | head -1)
# init a hugit log at the .git top-level
(use lib_init / the real hugit binary: the repo top-level is $PWD/.git because jj git init makes $PWD the repo root)
# pipe a tools/call capture to hugit-mcp stdio:
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
 "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"capture\",\"arguments\":{\"kind\":\"commit\",\"top_level\":\"$PWD/.git\",\"log\":\"<log>\",\"oid\":\"$OID\",\"branch\":\"main\",\"verify\":true}}}" \
 | target/debug/hugit-mcp
# assert {"status":"captured","seq":0,"event_hash":"..."}
# then: hugit why --walk ... && pr open --commit $OID
```
Note: `top_level` for the MCP tool is the repo dir hugit init created .hugit
in — for a jj repo the .git lives at $PWD/.git but the log is at
$PWD/.hugit/log.json (top_level = $PWD). Verify which the hook uses before
routing.

### Backlog unchanged (from prior resume sheet)
1. GitHub App mirror — infra-gated (`HUGIT_GH_TEST_REPO`).
2. jj-aware capture — NOW THIS WORK (MCP capture tool materializes it).
3. More journeys: `why` walk full, land with captured pushes — the latter is
   DONE (#345).
