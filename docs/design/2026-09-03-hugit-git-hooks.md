# hugit as the silent agent layer — git hooks design (v1)

**Status:** design for owner review · **Date:** 2026-09-03 · **Scope:** git-local
CLI (no server, no CoreLink in the default path) · **Owner direction verbatim:**
"_o llm usa git normalmente, o hugit detecta e faz o dele async em background sem
fricção nenhuma_".

## 1. Why hooks, not MCP tools (the decision already made)

The earlier MCP `commit` tool was REJECTED because it is friction: the LLM must
call `hugit commit` INSTEAD of `git commit`. The owner's model is the opposite:
**the LLM uses git normally; hugit observes silently.** Git hooks are the only
mechanism that fires *because the LLM used git* — the agent never changes a
single call.

This is the whole point: zero-friction capture by construction. The LLM's
`git commit`, `git checkout`, `git push`, `git merge` each trigger a hook that
hugit handles asynchronously. No wrapper, no new tool, no new habit.

## 2. The failure mode this must NEVER have (design constraints)

A hook that breaks `git commit` is catastrophic — it corrupts the LLM's most
sacred operation. Every design decision below serves one non-negotiable:

**A hugit hook MUST NEVER fail a git operation, MUST NEVER block longer than a
few ms, MUST NEVER emit output, and MUST be idempotent** (duplicate events are
impossible).

If `hugit` is missing, misconfigured, or the log is unwritable, the hook exits
**0** silently — the git operation proceeds, the capture is dropped that once
(nothing is corrupted, nothing is invented).

## 3. The event vocabulary (anchored to the existing log kinds)

Each hook feeds a specific [`kind`](kind) into the canonical `.hugit/log.json`
through the REAL hash-chained append (`EventLog::append_external_change` for raw
external change; `pr.opened` etc. for the PR capture). No new kinds invented
that would diverge the chain — we reuse the frozen vocabulary.

| # | Hook | Fires when | Records kind | Payload (redacted) | Purpose in the agent cycle |
|---|---|---|---|---|---|
| 1 | `post-commit` | the LLM ran `git commit` (non-merge) successfully | `ref.update` | `{ref: HEAD, target: <new oid>}` via `ExternalChangeKind::RefUpdate` | snapshot the branch tip after an incremental step; the raw source of `why`-on-latest |
| 2 | `post-checkout` | `git checkout <branch>` / switch | `ref.update` (or a highlighter `session.switch` alias) | `{ref: "HEAD", from: <prev>, to: <new>, branch: <name>}` | the LLM moved context — hugit knows which intent/campaign is now in focus |
| 3 | `pre-push` | `git push <remote> <branch>` | `ref.update`-class raw (the push source) + captures the pushed refs/commits | `{remote, refspecs, pushed_commits: [oid…]}` | the branch is leaving the machine — the "PR is coming" moment |
| 4 | `post-merge` | `git merge` landed a merge locally | `intent.landed`? NO — a merge is a raw event | `ref.update` with `{merged_from, target}` | consolidation in the local history |
| 5 | PR open (via `hugit pr open`, or a hidden `capture pr` hook invoked by the LLM's `gh`-equivalent) | a PR was proposed on the log | `pr.opened` | `{pr_id, campaign, intents[]}` (the EXISTING kind) | the bundle point — intents registered in log, not forged into git |

## 4. Async + silent (the zero-friction mechanics)

- Each hook is a tiny shell that **detaches**: `nohup hugit hook <kind> --git-dir <dir> </dev/null >/dev/null 2>&1 &`
  — the hook returns I'mmediately to git (exit 0), hugit does the append in a
  background child. Git never waits.
- **Fail-closed by loss, not by breakage:** if the child fails, the moment is
  lost (the event is not recorded) but `git` is NOT blocked — the constraint in
  §2.
- **Concurrency/locking:** the append uses the EXISTING atomic file lock
  (`FileLock` in `hugit-cli/src/pr/filelock.rs`), so parallel hooks (two commits
  in two worktrees) serialize without data loss.

## 5. Idempotency + dedupe (no double-record, no amplification)

- **Record key:** each `kind` carries a natural unique id — `ref.update` on the
  same `(ref, target)`; `pr.opened` on the same `pr_id`. Before appending, the
  hook's capture path reads the log's recent records and **skips a duplicate**
  (same `(ref,target)` already present).
- **Re-init safe:** `hugit init` re-running does NOT re-record; it only
  re-establishes the hooks (idempotent install) — never duplicates.
- **Crash-safe:** the append is atomic (tmp+rename) and the re-check depends on
  the monotonic chain, so a crash mid-hook leaves either no record or one
  complete record — never a torn one.

## 6. Privacy / redaction (inherited from the spine)

- The commit message is NOT embedded verbatim in v1 — only the oid + ref +
  branch + the `git`-hashed pointer. If a later revision wants message
  snippets, it goes through the SAME scrub/redaction path as every other
  porcelain emit (`hugit-ledger` redaction).
- Hooks capture only reference/graph facts (refs, oids, branch names), which
  are addresses — not content. Secret hygiene is preserved by never putting
  message/token content in the event payload.

## 7. Install + lifecycle (in `hugit init`)

- `hugit init` (which already runs `git init`) additionally writes
  `.git/hooks/{post-commit,post-checkout,pre-push,post-merge}` — tiny
  self-contained scripts that point at `hugit hook <kind>`.
- **Idempotent:** existing hooks are appended-to (not clobbered) if they don't
  already contain the hugit marker; a `HUGIT_HOOK_MARKER` comment makes
  re-install a no-op.
- **Removal:** `hugit init --reset` (or a future `hugit config` flag) removes
  the hugit-marked lines.
- **Silence:** the scripts exit 0 unconditionally before any hugit logic, so an
  env/host failure can't fail the git op.

## 8. What hooks do NOT capture (boundary, honest)

- **PR on GitHub / a remote forge** is not a *local* event — a local hook cannot
  observe `gh pr create` on the remote. The PR-capture is the **native hugit
  path**: the LLM (orchestrator) calls `hugit pr open` (or the `capture pr`
  seam) which records `pr.opened` with the intents from the log. The hooks
  capture the local *commits/pushes* that feed it. This is the honest seam —
  we never fake a `pr.opened` from a local push alone.
- **Remote merge** likewise: `land` is the hugit verb; a github merge button is
  out of hook reach. The design documents this boundary rather than pretending.

## 9. What "awesome" looks like (the product outcome)

An orchestrator running 10 agents in one repo:
- every `git commit` from every worktree leaves a `ref.update` on the collapse
  log — `hugit ledger` shows exactly which agent committed what, when
- `why --symbol` on the current HEAD resolves against the captured commits
- a push to a PR branch leaves a trace that the PR will consume
- the bundle (`pr.opened` with intents) is native and never fabricated
- the human sees a **complete, honest, chronological history** of what the LLM
  actually did to the git graph — evolve as if they NEVER touched a hugit
  command.

## 10. Open items for owner review (before coding)

1. **`post-checkout` kind**: reuse `ref.update` (with a `branch:`-titled field) or
   introduce a new kind `session.switch` on the log? Marting a new kind is a
   registry-level change; reusing `ref.update` is zero-additive but less
   expressive. Recommendation: introduce `session.switch` — it's the ONE new
   kind hooks legitimately need, and it's an observable, not a forged intent.
2. **`pre-push` payload**: capture the pushed commits by oid-list (correlatable
   to post-commit records) — recommend yes, bounded.
3. **Hook binary**: a real Rust bin (`hugit hook <kind>`) vs `hugit` verb +
   subcommand. Recommend a dedicated `hook` verb (faster, no subcommand
   dispatch) — one code path, minimal.
4. **The "PR hook"**: native `hugit pr open` is already the record of `pr.opened`.
   The per-LLM `gh pr` path is documented as out-of-reach of local hooks —
   x can add a `gh` shim later if wanted, but the honest v1 is native capture.