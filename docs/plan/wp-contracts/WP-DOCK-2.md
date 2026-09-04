# WP-DOCK-2 — dock resolver (cwd→gitdir→dock; env fast-path, cwd truth)

squad: hugit-core · M · opus · 80k · branch: wp/dock-2

## Charter
**What:** the resolver that answers "which dock am I in?" from any cwd —
walking up to the gitdir, mapping to the dock, honoring the env fast-path,
and enforcing the RULE that cwd (the physical truth) wins on mismatch.
**Why:** cost lands via this resolver (gateway + hook both call it). If it can
be wrong, cost lands on the wrong dock — split-brain. This is the correctness
spine of the whole design (A1, R2, R5, A2).

## Owned acceptance (VERBATIM from design 2026-09-04-dock-worktree-v1 §3.3, §7)
- (A1) — cwd beats env when gitdir differs: env's dock.gitdir ≠ cwd's gitdir
  ⇒ env ignored, child dock coined, `parent_id` recorded. Cost never on the
  wrong dock from fork-inheritance.
- (R5) — SAME rule on BOTH sides (gateway + resolver): cwd (physical truth)
  wins; env is only a fast-path when it agrees. Mismatch is reconciled to cwd.
- (A2) — cost before dock (micro-window): gateway/spool records `unlabeled`
  when no dock; when the dock appears (self-heal) reconcile `reconciled-late`
  — marked, never fabricated.
- (R4) — lazy-close proactive: a dock whose gitdir vanished is marked `ghost`
  at observation; `dock ls` lists ghosts; reconciliation runs on ghosts; the
  dock is closed on the durable log.
- (M5) — migration: repos with no docks yet ⇒ auto-coin a repo-scope dock on
  first read; unbound intents link to it; state marked `upgraded`.

## Contract deps
- `git rev-parse --show-toplevel` / `--git-dir` / `--git-path` (worktree-safe).
- `.hugit/log.json` EventLog (the Truth; dock records live here).
- The dock record shape frozen in WP-DOCK-1.
- `attest_keyset` (the verify seam) — NOT modified here (F2).

## Claims (paths)
- `crates/hugit-cli/src/dock/resolve.rs` — the resolver (cwd→gitdir→dock,
  env fast-path, mismatch rule, self-heal coin).
- `crates/hugit-cli/src/dock/ghost.rs` — ghost marking + proactive lazy-close.
- `crates/hugit-cli/src/dock/ls.rs`, `show.rs` — discovery (incl. ghosts).
- `crates/hugit-cli/tests/dock_resolver_*.rs`.

## Dispatch packet
- This contract + design §3.3, §7 A1/A2/R4/R5/M5.
- Anchor: WP-DOCK-1's dock-record shape + marker format; resolver is the
  reader of both.

## Properties (Lamport-style)

**P1 (safety — identity):** `resolve(cwd)` returns exactly ONE dock id for
every cwd inside a git repository, OR a well-formed `Err(Unbound)` — never a
wrong dock, never "guessed" (fail-closed on ambiguity: two open docks same
gitdir+branch ⇒ ambiguous, must self-heal to one).

**P2 (safety — cwd-wins on both sides):** IF env dock gitdir ≠ cwd gitdir
THEN the resolver's answer is the cwd dock (env ignored), AND a
`reconcile(dock_env → dock_cwd)` MUST be recorded (R5) — the invariant that
prevents gateway↔resolver split-brain.

**P3 (safety — no fabricated binding):** `resolve(cwd)` NEVER returns a dock
whose gitdir does not exist on disk at call time (fail-closed; an absent
gitdir yields `ghost` or `Unbound`, never a live dock).

**P4 (safety — reconciliation honest):** records reconciled cost EXACTLY once;
`reconciled-late`/`unlabeled` buckets never double-count a sample. The sum of
(bucket costs) == total gateway spend (assembled per dock), with rounding to
zero only by explicit bucket, never silently.

**L2 (liveness — unlabeled resolves):** IF an `unlabeled` cost sample exists
AND a dock for its gitdir later appears, THEN (eventually) the sample is
reconciled to that dock (marked `reconciled-late`) or stays `unlabeled`
(visible residual) — never lost, never fabricated.

## Implementation notes (pre-decided)
- 1. Resolver order: cwd walk-up → gitdir → marker file → dock.record in log.
- 2. Env fast-path: if `HUGIT_DOCK_ID` set AND its gitdir == cwd gitdir, use
  it (no log read). Else resolve by cwd and reconcile.
- 3. Ghost: resolver sees gitdir record but no dir → mark ghost in log (once),
  return `ghost` state. `dock ls` shows ghosts explicitly.
- 4. Self-heal covers R1 (clone-without-hook): resolving a gitdir with no
  marker coins a dock (kind: clone|repo) — recorded, `derived` charter.
- 5. Ambiguity (P1 fail-closed): two open docks same (gitdir,branch) ⇒
  resolver errors `AmbiguousDock{ids}` — must never guess.

## DoD
- Global gate green (fmt+clippy+test+deny).
- A1/R5/A2/R4/M5 red→green, hermetic AND e2e (real worktrees, env vs cwd
  mismatch scenarios, ghost removal, unlabeled→reconciled).
- P1-P4 + L2 proven by ≥1 hermetic test each.
- Cold-verify pass by non-author.

## Completeness
All owned items green · zero writes outside claims · evidence bundle attached
to SEAL.

## Return shape (SEAL: ≤20 lines)
status, evidence refs, deviations.