# Implementation plan — hugit silent git hooks (v2.1 design)

> **Planning doc** (2026-09-03) — the build blueprint for
> `docs/design/2026-09-03-hugit-git-hooks.md` + its adversarial review.
> Design is CLOSED; this is the executable plan. Nothing here changes the
> design; it fixes WHERE the code lands and WHAT passes before merge.

---

## 0. Goal (the deliverable)

The LLM uses `git commit / checkout / push / merge` normally. Hugit observes
silently (async, exit-0-always, never blocks, never emits) and records
`ref.update` (with honest payload qualifiers) onto the canonical log. The
human sees the full chronological agent history in `hugit ledger` — with zero
friction for the agent.

## 1. Deliverables (3 code changes + 1 test suite)

| # | Change | Where | Kind |
|---|---|---|---|
| **D1** | `hugit capture <kind>` verb — the capture path | `crates/hugit-cli/src/capture/mod.rs` (new) + register | code (new) |
| **D2** | Hook install in `hugit init` | `crates/hugit-cli/src/init/mod.rs` (modify) | code |
| **D3** | 4 hook scripts written by init | `.git/hooks/{post-commit,post-checkout,pre-push,post-merge}` | generated files |
| **D4** | `hugit capture` verb tests | `crates/hugit-cli/src/capture/` unit + `crates/hugit-cli/tests/acceptance_capture.rs` | tests |

## 2. D1 — the `hugit capture` verb (the capture path)

A single verb, hidden from `--help` porcelain surface? NO — it's a real verb but
must be X5-safe (`hook` is not a git command — verify). Subcommands:

- `hugit capture commit --top-level <path> --log <abs> --oid <sha> --branch <name>`
  → append `ref.update {ref, target: <oid>, branch}` (attr principal
  `orchestrator:hugit-hook`).
- `hugit capture checkout --top-level <path> --log <abs> --from <sha> --to <sha>
  --branch <name>` → `ref.update {checkout: true, from, to, branch}`.
- `hugit capture push-attempt --top-level <path> --log <abs>` → reads refspecs/shas
  from STDIN (the pre-push hook's stdin shape), appends
  `ref.update {attempt: true, refspecs, shas}`.
- `hugit capture merge --top-level <path> --log <abs> --from <sha> --to <sha>` →
  `ref.update {merged_from, target: <to>}`.

**Hard rules (from the adversarial review):**
- **exit 0 ALWAYS** — any error prints nothing to stdout/stderr? NO: the POST
  hooks' stderr is visible to the user, so the verb must write errors ONLY to a
  log file (`.hugit/hooks.log` appended, best-effort) and exit 0. The `pre-push`
  passthrough is a script that always exits 0 regardless.
- **Append under a single `FileLock`** spanning load → `append_external_change`
  → persist (the read-modify-write race fix). `FileLock::acquire(log_path)` is
  reused (`crates/hugit-cli/src/pr/filelock.rs`).
- **Dedupe inside the lock**: before append, read the recent `ref.update`
  records; skip if same `(kind-payload-key)` already present.
- **`recorded_at`** = the git event time (`committer date` for commit/merge;
  now for push-attempt) — ordering by real time, documented.
- **Use the `load_event_log` chokepoint** (`crates/hugit-cli/src/checks/mod.rs:556`)
  — verifies the chain before append, so hooks never write onto a broken log.

## 3. D2/D3 — hook install in `hugit init`

After the `.hugit/` + log creation (and after git exists), `init`:

1. Resolve `git_dir = git rev-parse --absolute-git-dir` and
   `hooks_dir = git rev-parse --git-path hooks` (worktree-safe).
2. Write 4 scripts, each:
   - starts with the fixed marker line `# hugit-hook (managed; safe to edit)`
   - resolves the hugit binary (env `HUGIT_BIN` then `hugit` on PATH)
   - **detaches**: `nohup ... </dev/null >>.hugit/hooks.log 2>&1 &` then `exit 0`
   - `pre-push` parses stdin (refspecs/shas) and always exits 0
   - `post-checkout` only acts when `flag == 1` (branch)
   - `post-commit`/`post-merge` read `git rev-parse` in the background child
3. Idempotent: scripts that already contain the marker are left untouched.
4. `init` reports `hooks_installed: ["post-commit", ...]` in the JSON output
   (new field, additive).

## 4. The 4 hook scripts (shell, generated — the exact content)

All follow: `#!/bin/sh`, marker comment, quiet-ued, detach, exit 0 upfront.

```
#!/bin/sh
# hugit-hook (managed; safe to edit; optional)
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
LOG="$ROOT/.hugit/log.json"
[ -f "$LOG" ] || exit 0
# post-commit: no args; capture HEAD in background
( sleep 0 ; "$HUGIT_BIN" capture commit --top-level "$ROOT" --log "$LOG" \
    --oid "$(git rev-parse HEAD 2>/dev/null)" \
    --branch "$(git branch --show-current 2>/dev/null)" \
  >>"$ROOT/.hugit/hooks.log" 2>&1 ) &
exit 0
```

(pre-push variant reads stdin; post-checkout checks `$3 = 1`; post-merge passes
the squash flag and captures HEAD~1..HEAD.)

## 5. Test plan (D4)

**Unit (in `hugit_cli::hook`):**
- each `hook <kind>` subcommand, fed the REAL stdin/arg shapes `git` passes,
  asserts the right event lands on the log (kind + payload qualifier).
- dedupe: append same `(ref,target)` twice → 1 record.
- concurrency: two threads append to one log simultaneously → both records
  survive (the lock works), chain verifies.
- `recorded_at` from an explicit `--recorded-at` (git date) → stable.

**Journey (`tests/acceptance_hook.rs`):**
- `hugit init` in temp dir → asserts hooks dir contains the 4 scripts with the
  marker + `hooks_installed` field.
- REAL `git commit` twice → assert log has 2 `ref.update` with the right oids.
- `git checkout -b` → assert `ref.update {checkout:true,...}` (and a
  `git checkout -- file` does NOT record).
- `git push` to a local bare remote → assert `ref.update {attempt:true,...}`;
  a FAILED push (bad remote) records attempt, NEVER success.
- `hugit ledger` on the log → the agent's history renders (the fold reads
  `ref.update`).
- adversarial: set `HUGIT_BIN=/nonexistent` → git commit still succeeds, hooks
  still exit 0 (capture dropped, nothing broken).

**Bundle:** the full `cargo test --workspace --locked` (heavy) is run ONCE at
the bund gate, not per change.

## 6. Sequence + gates

| Step | Gate (must pass before next) |
|---|---|
| 1. `hugit capture` verb + unit tests | `cargo test -p hugit-cli --lib` + fmt + clippy 0 |
| 2. init hook install + journey | journey tests green |
| 3. X5 check (no verb shadows git) | `cargo test -p hugit-invariants --test acceptance_x5` |
| 4. Bundle | `cargo test --workspace --locked` — 201+ suites, 0 failed |
| 5. PR + merge + push | dco + gates CI green |

## 7. Open micro-decision (resolve during impl, not design)

- `hugit capture` verb: hidden from `--help` (like internal plumbing) or visible?
  Recommend **visible but documented as "internal plumbing — git hooks call
  this; you probably won't"** — honest, no secrets.
- Payload `principal_chain` for hook records: recommend
  `["orchestrator:hugit-hook"]` (the recorder identity, not a user) — the fold
  keeps attribution distinct from human/orchestrator intents.
- Hook script log: `.hugit/hooks.log` appended best-effort (never fatal).

---

*Plan is the execution contract — after owner review, code lands in D1→D4 with
the gates above and nothing else.*
## 8. REVIEW-ADVERSARIAL AMENDMENTS (2026-09-03, second pass)

### A1 — `hugit hook` SHADOWS `git hook` (X5 violation) — RENAMED
`git --list-cmds=builtins,main` includes **`hook`** (git 2.36+ added the core
`git hook run` command). The plan's verb `hugit hook` would violate the X5
no-shadow law — the exact bug `init` had (and was reverted for). **Verb renamed
to `hugit capture`** (`capture` is X5-safe, verified: not in
`git --list-cmds=builtins,main`; `annotate` etc. shadow, avoided). All hook
scripts call `hugit capture <kind>`.

### A2 — append via `append_authorized(Endpoint::Push)`, not bare external
The check verb already appends `ref.update`-class events under
`append_authorized(PrincipalClass::Orchestrator, Endpoint::Push)` with default
principal `orchestrator:hugit` (`checks/run.rs:1787,1797-1799`). The capture verb
MUST use the SAME path: `append_authorized(Orchestrator, Push, "ref.update", [principal], payload, recorded_at)`.
- Endpoint::Push is the universal verb — every class may push (authz matrix
  `(_, Push) => true`), so hooks never hit a denial.
- Auditable: a denied hook append would itself be audited (fail-closed).
- Principal default: `orchestrator:hugit-hook` (consistent recorder identity);
  NO new PrincipalClass needed (Orchestrator classifies `orchestrator:`).
- NOTE: this means the closed `ExternalChangeKind` enum is NOT involved — we
  append a `ref.update` payload through the authorized door, same as the CLI
  porcelain does. Contract stays frozen, sibling crates untouched.

### A3 — Confirmed from verification (no plan change)
- `Endpoint::Push` is "the universal git mutation; every class allowed"
  (`authz/mod.rs:76,208`) — hooks can push-record without new authz.
- Hook CWD = top-level of the worktree (`git rev-parse --show-toplevel` works);
  hooks dir = `git rev-parse --git-path hooks` (worktree-safe).
- `nohup ... &` + redirect detach works; the child survives git exit.

### Net
D1 verb is `hugit capture` (renamed), appending under `append_authorized`.
D2/D3 scripts call `hugit capture`. D4 tests use `capture`. The X5 gate now
checks `capture` (passes). No new kinds, no contract changes, no sibling crates.
