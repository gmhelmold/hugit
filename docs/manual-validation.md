# Manual validation — hugit go-live (user-real simulation)

This is the evidence + runbook for the **user-real simulation** of hugit: a
fresh binary, a fresh git repo, an isolated HOME/XDG, and the CLI exercised
the way a developer would use it. Every step below ran the REAL
`target/release/hugit` (no mocks, no fixtures beyond a scratch repo).

## Instrument: `scripts/validate-go-live.sh`

The reproducible simulation. Deterministic, fail-fast, 7 steps / 37 asserts:

1. binary `-V` + `--help` lists verbs + dock
2. `hugit setup` → template dir + the 4 hooks + `OWNED-BY-HUGIT` marker + git
   global `init.templateDir`
3. fresh `git init` (ships hooks from template) → first commit **lazy-boots**
   `.hugit/log.json` and captures a `ref.update`
4. `campaign open` → `intent new` → `check run` (MISS) → `check run` (HIT,
   zero local exec, saved_ms) with an env-independent green check
5. `pr open` (captured commit) → `pr queue` → `land queue` (union green, lands
   PR-1) → `pr show`
6. `git worktree add` + `dock ls` (the worktree's dock is `open`)
7. `export` → `export.json` + `repo.git` bundle

It never touches the real gitconfig (isolated `HOME`/`XDG_CONFIG_HOME` +
`GIT_CONFIG_NOSYSTEM`).

### Executed:
```
HUGIT_BIN=<release>/hugit ./scripts/validate-go-live.sh
```

**Result: `ALL PASS (0 failures)` — twice (reproducible), exit 0.**

### Mutation proof that the instrument bites
A copy with `"verdict":"green"` flipped to `"verdict":"PURPLE"` **failed** on
step 5 (`FAIL: union verdict green`). The script detects a wrong state; it is
not vacuously green.

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