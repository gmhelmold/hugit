# WP-DOCK-1 — hook-born dock coinage (post-checkout auto-dock)

squad: hugit-core · M · opus · 75k · branch: wp/dock-1

## Charter
**What:** the moment a worktree/clone is born (`git worktree add`, `git clone`,
first checkout), the already-installed `post-checkout` hook must coin **the
dock** — the physical binding (gitdir+branch hash) that later carries cost and
verification. Zero friction: the user's gesture is standard git; the hook does
the work, silently, non-blocking, worktree-safe.
**Why:** without a dock that exists *before* the work, cost can only be
post-hoc reconstruction — the exact failure this design exists to avoid. The
dock is the "doca that existed when the barque left".

## Owned acceptance (VERBATIM from design 2026-09-04-dock-worktree-v1 §4-§7)
- (A1) — the hook detects new gitdir + `flag==1` and coins: `dock_id =
  hash(gitdir+branch)`; charter derived from branch (marked `derived`);
  marker `created_ts+pid` written into `.git/worktrees/<name>/hugit-dock`;
  `dock.record` appended to `.hugit/log.json`.
- (A3) — path re-creation (same worktree name) ⇒ NEW dock (marker `created_ts`
  differs), never a re-open of the old.
- (R1) — `git clone` fires NO local post-checkout: the dock comes from env
  (orchestrator, `HUGIT_DOCK_ID` at clone time) or self-heal (resolver coins by
  gitdir later); never silently missing.
- (R2) — env-vs-cwd divergence: when the hook coins cwd dock B while env says
  A (gitdir mismatch), it logs `parent_id=A` AND emits a visible warning
  (stderr, never fails the git op).
- (C1) — hook never fails the git op: `exit 0` always, detach, `HUGIT_BIN`
  absent ⇒ no-op.
- (B3) — jj colocated (has real `.git`): the SAME hook covers it (gitdir
  exists → dock coined via post-checkout precisely because the repo has a
  real gitdir); jj standalone (no `.git`) is OUT of scope here → falls to
  repo-scope (honest, never fabricated per-unit).

## Contract deps
- `post-checkout` hook installed by `hugit init` (already worktree-safe,
  detach, non-blocking — see `crates/hugit-cli/src/init/mod.rs`).
- `.hugit/log.json` EventLog shape (hash-chained; `hugit_refstore::EventLog`).
- `EventRecord` frozen type.

## Claims (paths — writes outside = leak)
- `crates/hugit-cli/src/dock/` (new module: coinage, marker rw, log append)
- `crates/hugit-cli/src/init/mod.rs` (hook body: add dock-coin + env warning)
- `crates/hugit-cli/tests/dock_coinage_*.rs`

## Dispatch packet
- This contract file + design `docs/design/2026-09-04-dock-worktree-v1.md`
  (§4, §7 A1/A3/R1/R2).
- Anchor: `crates/hugit-cli/src/init/mod.rs` HOOK_KINDS + hook_script; the
  canonical log append path already used by capture.
- The dock record shape: `{dock_id, gitdir, branch, charter(derived),
  created_ts, pid, parent_id?:Option, state:open|ghost|closed, kind:
  worktree|clone|repo}` — to be FROZEN here (the on-disk wire contract).

## Properties (Lamport-style: safety / liveness)

**S1 (safety — unique dock):** for every distinct (gitdir, branch) pair there
is AT MOST ONE open dock with that pair at any time. Coinage is idempotent:
re-running the hook on an existing marker is a no-op (never a duplicate).

**S2 (safety — no silent misroute):** IF env `HUGIT_DOCK_ID` is set AND the
cwd gitdir differs from the env dock's gitdir, THEN the hook coins the cwd
dock AND records `parent_id` AND emits a warning. No silent divergence.

**S3 (safety — never fabricated dock):** the hook coins ONLY on a real gitdir
present on disk at coin time. It NEVER writes a dock.record whose gitdir does
not exist (fail-closed: no dock for a nonexistent workspace).

**L1 (liveness — dock exists):** IF a worktree/clone checkout completes in an
initialized repo, THEN (eventually) a dock.record EXISTS for its
(gitdir, branch). (By hook OR self-heal — the guarantee is the record is
observable via `dock ls`.)

## Implementation notes (pre-decided)
- 1. Detect: `git worktree list --porcelain` or read `.git/worktrees/<name>`
  marker file. flag==1 post-checkout = new checkout.
  marker file. flag==1 post-checkout = new checkout.
- 2. Marker: write `created_ts` + `pid` (ns) — A3's reborn-detection key.
- 3. Env warning: stderr line `[hugit] dock: cwd<B> differs from env<A>;
  parent_id=A` — never `exit != 0`.
- 4. Self-heal OUT of scope here (WP-DOCK-2); this WP only coins on hook.
- 5. Charter derived: branch `feat/rate-limit` → `add rate limit` (strip
  separators, lowercase, mark `derived:true`).

## DoD
- Global gate green: fmt+clippy `--workspace --all-targets --locked -D
  warnings` + test `--workspace --locked` + `cargo deny`.
- Owned items A1/A3/R1/R2/C1 red→green, hermetic (fake worktree) AND e2e
  (real `git worktree add` + `git clone` in a temp repo).
- S1-S3 + L1 all proven by at least one hermetic test each.
- Cold-verify pass by non-author (fresh eye re-runs the red→green w/o hints).
- Zero writes outside Claims; evidence bundle attached to SEAL.

## Completeness
All owned items green · zero writes outside claims · evidence bundle attached
to SEAL.

## Explicit non-goals (existentials — managed, NOT accepted here; honesty)
- **F1 (no worktree — checkout principal):** dock is repo-scope (coarse but
  real); NOT per-unit. Never fabricated per-unit here.
- **F4 (jj standalone, no `.git`):** repo-scope honest; never per-unit.
- **C5 (`CloseResponse` attestation block):** v2 — existing tracked additive
  change; NOT in this series.

## Return shape (SEAL: ≤20 lines)
status, evidence refs (test names + paths), deviations=none|waiver-ref.