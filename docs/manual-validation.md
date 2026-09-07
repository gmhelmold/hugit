# Manual validation — hugit go-live (user-real simulation)

This is the evidence + runbook for the **user-real simulation** of hugit: a
fresh binary, a fresh git repo, an isolated HOME/XDG, and the CLI exercised
the way a developer would use it. Every step below ran the REAL
`target/release/hugit` (no mocks, no fixtures beyond a scratch repo).

## Instrument: `scripts/validate-go-live.sh`

The reproducible simulation. Deterministic, fail-fast, **11 steps / ~56
asserts**, covering every behavior that the manual walkthrough proved (the
user-real session's transcript behaviors are now ASSERTED, not just shown):

1. binary `-V` + `--help` lists verbs + dock
2. `hugit setup` → template dir + the 4 hooks + `OWNED-BY-HUGIT` marker + git
   global `init.templateDir`; `hugit setup --repo /path/to/repo` installs same
   hooks into an existing repo without changing Git data or non-hugit hooks
3. fresh `git init` (ships hooks from template) → first commit **lazy-boots**
   `.hugit/log.json` and captures a `ref.update`
4. `campaign open` → `intent new` → `check run` (MISS) → `check run` (HIT,
   zero local exec, saved_ms) with an env-independent green check
5. `pr open` (captured commit) → `pr queue` → `land queue` (union green, lands
   PR-1) → `pr show`
6. `git worktree add` + `dock ls` (the worktree's dock is `open`)
7. `export` → `export.json` + `repo.git` + redaction manifest **with a listed
   removal**
8. D14 (human author without principal → structured error) + campaign seal
   (close; idempotent `already_closed`; **sealed campaign refuses both `pr
   open` and `intent new`** with `campaign_sealed`)
9. `dock land` **`byte_identity:verified` + `landed:true`**; worktree remove →
   dock shows `ghost` + `dock reconcile` idempotent
10. `ctx usage` verbatim (input+output total) + `dock insight` honest-zero cost
    + residual bucket
11. `policy test` (missing context errs; dco gate evaluated), `symbol` rust fn
    outline, `why` unresolved (never fabricates), `undo` → nothing_to_
    compensate (honest), `verdict approve` persists verdict_recorded:true

It never touches the real gitconfig (isolated `HOME`/`XDG_CONFIG_HOME` +
`GIT_CONFIG_NOSYSTEM`).

### Executed (LOCAL, macOS real — no GitHub runner dependency):
```
HUGIT_BIN=<release>/hugit ./scripts/validate-go-live.sh
```

**Result: `ALL PASS (0 failures)` — `hugit 0.1.2`, exit 0** (reproducible).

### Full local suite (2026-09-06, macOS)
| Crate group | Suites ok | Failed |
|---|---|---|
| hugit-cli (product + all git-real acceptance + the platform fixes) | 66 | 0 |
| refstore + contracts + proto + ledger | 32 | 0 |
| checks + symbols + queue + policy + fence + diag | 45 | 0 |
| mirror + mcp + app + invariants + dogfood | 35 | 0 |
| fmt · clippy --workspace --all-targets · deny | clean · 0 · exit 0 | — |

**178 suites green, 0 failures, on macOS.** Clippy/fmt/deny clean.

### Platform matrix: evidence + status
The current release matrix runs the full `cargo test` on all four OS (Linux,
macOS arm64, macOS x86-64, Windows). The last fully green release run
(`34042599031`, v0.1.1) proved Linux test execution plus build/package/smoke on
all four targets; its older workflow skipped tests on macOS and Windows. Later
matrix execution surfaced real Windows issues, each fixed + guarded:

| Windows issue found by the matrix | Fix |
|---|---|
| `worktree` gitdir detection was unix-only (`contains("/worktrees/")`) | separator-agnostic `is_worktree_gitdir` (regression test + mutation-probe) |
| L2: test depended on async hooked post-checkout (race) | `git worktree add --no-checkout` (no hook) |
| E5 exit-proof: `which_git` looked for `git` without `.exe` | try `git.exe` on Windows |
| dock resolver tests: assumed `.git` is a directory | use `git --absolute-git-dir` (the FILE truth) |
| fleet_journey wait 8s too tight on slower Intel runner | 25s |

**Open status:** re-running the full 4-leg release CI is currently BLOCKED by
the GitHub account billing state (the hosted runner refuses to start jobs:
"account payments have failed / spending limit"). This is infra, not code —
the complete local suite (above) is green, and the platform fixes are
evidence-driven from the matrix runs that DID execute.

### Mutation proof that the instrument bites
Two mutations each made a COPY of the script FAIL (fail-fast rc≠0), proving
the asserts detect a wrong state rather than passing vacuously:
- `byte_identity` "verified" → "BROKEN": **FAIL** at step 9.
- `verdict_recorded` true → false: **FAIL** at step 11.

## Manual walkthrough (this session, real binary)

Beyond the script, a human-style session exercised the CLI commands directly
on scratch repos, capturing each command's JSON output:

| Command | Observed (evidence) |
|---|---|
| `campaign open` | `{"opened":true,...}` + log auto-created `.hugit/log.json` |
| `intent new` | returned `intent_id`; store + (with `--log`) `intent.landed` |
| `git commit` + capture | `ref.update {branch,target}` appended |
| `pr open --commit <sha>` | PR accepted a captured commit; **human author requires `--principal`** (D14, exit 2) |
| `check run --def fmt` ×2 | MISS (exec 1) → **HIT (exec 0, saved 5525ms)** — memoization real |
| `pr queue` → `land queue` | union engine `landed:["PR-1"], verdict:"green"` |
| `campaign close` | seal works; **idempotent** (`already_closed:true`) |
| PR in sealed campaign | **refused** `campaign_sealed` (and `intent new` now refuses too — see the fix) |
| `pr abandon` ×2 | `abandoned:true` then `already_abandoned:true` |
| `dock coin` + `dock ls` | worktree dock `open`; `dock insight` honest (0 cost) |
| `dock land` no recorded tip | `no_recorded_tip, landed:false` (fail-closed) |
| commit in worktree → `dock land` | **`byte_identity:"verified", landed:true`** |
| worktree remove → `dock reconcile` | ghost closed onto the log, idempotent |
| `ctx usage` | recorded verbatim token counts |
| `export` | envelope JSON + repo.git + redaction manifest |
| `why` on a path | `unresolved` when the capture had no `files` (never fabricates) |
| `policy test --context` | house gates evaluated: dco/changelog/secrets outcomes real |
| `undo --seq` on a note | `nothing_to_compensate` (honest) |
| `verdict record` no `--store` | dry run (correct); `verdict approve` persists |
| `symbol --file` | tree-sitter outline (e.g. 51 items on the symbols lib) |
| `issue transition` · `meta set` · `fleet` · `diag` · `tournament` · `impact` · `review` · `ctx resume` | each returned correct structured JSON (see session transcript) |

## Bugs the user-real runs caught (all fixed + test-guarded)

1. **`hugit export` failed** on any repo that had captured a checkout
   ("malformed payload for ref.update") — `replay` treated a checkout as a
   malformed `{ref,target}`. Fixed (`checkout:true` is inert); regression
   `checkout_ref_update_is_inert_not_malformed`.
2. **`intent new` ignored a sealed campaign** in `--store`-only mode. Fixed
   (seal-parity via the default log); regression
   `intent_new_refuses_campaign_sealed_on_default_log` (mutation-probed RED).
3. **Windows CI**: byte-exact tests broke (autocrlf) → `.gitattributes`
   `eol=lf`; tests used `:` in a temp-dir name (illegal on Windows) → sanitize.
4. The memoization step of the validation script originally used the `fmt`
   builtin, which records a synthetic RED in a rustfmt-less sandbox → changed
   to an env-independent green check.

## How to reproduce any of this

```sh
git clone https://github.com/gmhelmold/hugit.git
cargo build --release
HUGIT_BIN="$PWD/target/release/hugit" ./scripts/validate-go-live.sh
```

## Honest limits (not covered by the simulation)

- Not exercised here: the paid-service window (deliberately quarantined out of
  the product); multi-tenant hosting (your git remote is the host); file
  blobs/serve routes (not part of the CLI plugin).
- Crypto signature verification of attestations: the integrity spine verifies
  hash-chains (proven by tamper tests), but the off-box signer is out of scope
  for the local CLI.
