# Adversarial review — `hugit as the silent agent layer` (hooks design v1)

**Reviewer:** self-adversarial (lead) · **Date:** 2026-09-03 · **Input:**
`docs/design/2026-09-03-hugit-git-hooks.md` · **Method:** each design claim
attacked against the actual git CLI + the codebase facts, one-by-one.

## Verdict

The v1 design's **direction is right** (hooks = zero-friction capture), but the
spec as written is **not yet buildable** — it ignores five load-bearing git and
codebase facts. All five are fixable in the design; none requires new
invention. The corrected design is below.

---

## Finding 1 — `pre-push` can BLOCK the push (friction, design-breaking)

**Claim in v1:** "each hook exits 0 unconditionally." **Correction:** `pre-push`
is a **pre** hook: the WHOLE POINT of pre- hooks is that `exit != 0` aborts the
operation. If hugit is installed as `pre-push` and any path in the hook
returns non-zero (missing binary, bad config, a `hugit hook` crash), the LLM's
`git push` is **blocked** — the ONE failure mode the design promised to never
have.

**Fix (hard rule):** the `pre-push` hook script MUST be a *pure* passthrough:
it parses stdin (to know what is being pushed), then **always exits 0**,
dispatching the capture asynchronously. It must not even shell out
synchronously to hugit. (Alternative: use `post-update`/`push-to-checkout` —
but `pre-push` is the only one with the pushed-refs input on stdin, so we keep
it, strictly exit-0.)

## Finding 2 — no `post-push` hook exists (data source mismatch)

**Claim:** "`pre-push` captures the pushed refs." **Reality:** `pre-push` sees
the refs BEFORE the remote call; if the push FAILS (network, rejected), the
captured "pushed" state is a **lie**. There is no `post-push` in git.

**Fix:** capture the *intent to push* in `pre-push` (a `push.attempt` record
with the refspecs + local SHAs — honest: "attempted") and let the remote-side
truth be learned by the PR/bundle path (native `hugit pr open` reads the log).
Never record "pushed successfully" from a pre-hook — a failed push must not
leave a false `ref.update` on the log.

## Finding 3 — hook CWD is NOT the worktree root (path resolution)

**Claim:** hooks run "in the repo". **Reality:** git hooks run with cwd = the
**top of the working tree** (or the git-dir for bare/hooks), and in a linked
worktree `.git` is a FILE, not a dir. Assuming `cwd` or `.git/hooks` is wrong.

**Fix:** every hook resolves the truth via git itself:
- `root = git rev-parse --show-toplevel` (works in worktrees and normal repos)
- `git_dir = git rev-parse --absolute-git-dir`
- log path = `<root>/.hugit/log.json` (the absolute, non-relative form) —
  the `DEFAULT_LOG_PATH` rel path must be made absolute from `root`, never
  from cwd.

## Finding 4 — concurrent worktree hooks lose events (race)

**Claim:** "the existing FileLock serializes." **Reality:** the log is
read-modify-write (`load_event_log` → append → persist). Two hooks in two
worktrees both `load`, both append, both persist → **last-writer-wins drops
one event**. The FileLock exists but the append path must hold it across the
read→append→write, not just the write.

**Fix:** the `hugit hook` verb appends under a **single `FileLock` held across
load → `append_external_change` → persist**. This is the ONE change to the
existing discipline (per-verb callers don't need it; the hook path does,
because it is the only concurrent multi-process append).
Also: `append_external_change` must be called with the lock held, and the
dedupe re-check (Finding 5) must happen under the same lock (check-then-act is
atomic only if both are inside the critical section).

## Finding 5 — idempotency is racy as specified

**Claim:** "skip a duplicate (same ref,target) before appending." **Reality:**
without the lock from Finding 4, check-then-append is racy: two concurrent
hooks both see "not present", both append → duplicate.

**Fix:** dedupe check is INSIDE the lock (with Finding 4). Key = natural
identity: `ref.update` on `(ref, target)`; `session.switch` on
`(from,to,branch)`; `push.attempt` on `(refspecs, local_shas)`.
A concurrent duplicate then sees the winner's record and skips.

## Finding 6 — `post-commit` has NO input: it must read git itself

**Claim:** "`post-commit` records `HEAD` → ref.update." **Reality:** the hook
gets no args/stdin; it must run `git rev-parse HEAD` + `git name-rev`/branch,
and `git log -1 --format=%s` for context — all under the understanding that
this is async (small latency, fine). But it MUST NOT fail the commit: it is a
post-hook (exit≠0 ignored by git), still follow exit-0 discipline.

**Fix:** confirmed as designed; note in spec that the oid + branch come from
`git rev-parse` inside the hook's detached child, with the cwd fix from
Finding 3.

## Finding 7 — `session.switch` is scoped wrong (post-checkout noise)

**Claim:** capture every checkout as `session.switch`. **Reality:**
`post-checkout` fires on file-level checkouts too (`git checkout -- file`),
and on every branch switch — including the same-branch detach/attach inside
tooling. Recording every one = log spam + hard-to-read ledger.

**Fix:** only record `session.switch` when `<flag> == 1` (branch checkout) AND
`<to>` is a different commit than the log's last switch (dedupe by
`(from, to, branch)`), AND the ref actually moved. File-level checkouts
(flag 0) are skipped. Also `.git/hooks/quarantine` — keep the sample silent.

## Finding 8 — PR capture: the design conflates two different moments

**Claim:** "PR hook via pr.open." **Reality:** opening a PR (native
`hugit pr open` on the log, intents validated) and capturing a detachable
GitHub PR (via `gh`) are DIFFERENT: the first is already native; the second is
a remote state a local hook cannot observe honestly.

**Fix (boundary made explicit):** v1 captures ONLY the local truth:
- `commit` → ref.update (oid, branch)
- `checkout(branch)` → session.switch (from,to)
- `pre-push` → push.attempt (refspecs, shas) — async, exit-0
- `pr open` native → pr.opened (existing, unchanged)
- Local merge → ref.update (existing `post-merge`, exit≠0 won't block)

Remote GitHub PR creation is **out of scope for hooks**; the honest seam is
the native `pr open` + a documented `gh`-side note. A `gh` shim is a later,
explicitly-optional add.

## Finding 9 — the "drag" of async capture can lose ordering

**Claim:** detach + append. **Reality:** if two git events happen in quick
succession (commit; commit; push), the async children may append in the wrong
order relative to wall-clock (commit2 could append before commit1). The chain
is still valid (hash chain, monotone seq) but the *logical order* reflects
append order, not git time.

**Fix (acceptable, honest):** each record carries `recorded_at` (unix ms) and
the hook sets it from the git event (e.g. `post-commit` uses the commit
`committer date`; `pre-push` uses now). Ordering by `recorded_at` is then
stable and truthful even if append order races. This is the documented
semantic: *log order = hash-chain append order; true order = `recorded_at`
field*. Keep `seq` as the chain pointer.

## Finding 10 — testability is underspecified

**Claim:** "journey test proves zero friction." **Reality:** hard to test hooks
end-to-end (they run under git). 

**Fix (concrete):**
- unit: the `hugit hook <kind>` verb parses each hook's real stdin/arg shape
  (feed it the exact bytes `pre-push` and `post-checkout` pass) → asserts the
  right event lands on the log, dedupe fires, lock serializes concurrent
  appends (two threads, two worktrees, one log).
- journey: `git init` in a temp dir via `hugit init` → run REAL `git commit`
  twice → assert the log has 2 `ref.update` with the right oids; run
  `git checkout -b` → assert session.switch; run `git push` to a local bare
  remote → assert push.attempt (and that a FAILED push records attempt, not
  success); assert exit codes are 0 and git never blocks.
- adversarial: simulate `hugit hook` failing (missing bin) → assert hooks
  still exit 0 and git ops proceed.

---

## Corrected spec deltas (what changes from v1)

1. `pre-push` is a **pure, always-exit-0** capture of `push.attempt` (never
   blocks; never claims success).
2. New kind `push.attempt` (instead of "captures pushed refs").
3. Every hook resolves root/git-dir via `git rev-parse --absolute-git-dir` /
   `--show-toplevel`; log path is ABSOLUTE.
4. `hugit hook` appends under a **single FileLock spanning load→append→persist**;
   dedupe check inside the lock.
5. `session.switch` only on branch checkouts (`flag==1`), dedupe by
   `(from,to,branch)`, skip file-level.
6. `recorded_at` from the git event (committer date for commit) — append order
   ≠ true order documented.
7. Pr captured natively only (`pr.opened`), with remote-GitHub explicitly out
   of hook scope.
8. Test plan: unit (real stdin shapes), journey (real git ops), adversarial
   (missing-bin → still exit 0).

**Net:** the corrected design still delivers "the LLM uses git normally, hugit
does its part silently" — with the five load-bearing facts (pre-blocks,
no-post-push, worktree cwd, read-modify-write race, stdin shapes) designed in,
not discovered late.
---

## Finding 11 (final review pass) — `push.attempt` is NOT a new kind (contract freeze)

**Claim (v2):** introduce kind `push.attempt`. **Correction:** `ExternalChangeKind`
is a **frozen, closed enum** (`RefUpdate`/`RefDelete`) and `RAW_PUSH_KINDS` is a
2-element const array — both load-bearing for `hugit-proto`'s raw-push recorder
(the no-fake-intent guarantee is a *compile-time* fact from the closed kind).
Adding `push.attempt` = contract change touching a sibling crate. Violates the
"no forged kinds, reuse the frozen vocabulary" principle.

**Fix (v2 final):** `pre-push` records a **`ref.update`** (the existing kind that
means "a ref points at a sha") with a payload flagged `attempt: true` +
`{refspecs, shas}`. Same kind, honest payload: *the ref was being pointed at
this sha (attempted push)*. The distinction "attempted vs landed" lives in the
payload marker + `recorded_at`, not in a new kind. `ExternalChangeKind` and
`RAW_PUSH_KINDS` stay untouched. Zero sibling-crate change.

## Final v2.1 spec — the kinds the hooks emit (all frozen vocabulary)

| Hook | Kind (existing) | Payload |
|---|---|---|
| `post-commit` | `ref.update` | `{ref, target: <oid>, branch}` |
| `post-checkout` (branch) | `session.switch` — **SECOND REVIEW: NOT a new kind either** | see below |
| `pre-push` | `ref.update` (attempt flag) | `{attempt: true, refspecs, shas}` |
| `post-merge` | `ref.update` | `{merged_from, target}` |
| PR open (native) | `pr.opened` | existing, unchanged |

### `session.switch` — same contract-freeze reasoning

`session.switch` was proposed as the ONE new kind. Same problem: any new kind
outside `RAW_PUSH_KINDS`/`ExternalChangeKind`/`pr.*`/etc. is a registry-level
contract change. **Final ruling : do NOT introduce it.** `post-checkout` (branch,
flag==1) also records a **`ref.update`** with payload `{checkout: true, from,
to, branch}` — again the same existing kind, honest payload. The layout of "is
it a commit, a checkout, a push-attempt or a merge" is a **payload qualifier**,
not a kind. This keeps the frozen vocabulary 100% intact and the log readable
(the `ref.update` fold already exists in replay).

## Final verdict (v2.1)

The hooks emit ONLY frozen kinds (`ref.update` with honest payload qualifiers,
`pr.opened` native). Zero contract changes, zero sibling crates, zero new kinds.
All 10 prior findings fixed. The design is now **buildable without touching any
frozen type** — only the `hugit hook` verb + hook install in `init` + tests.
