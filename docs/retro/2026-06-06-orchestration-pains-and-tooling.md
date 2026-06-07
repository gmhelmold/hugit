# Orchestration retrospective — the pains of running a 40+ WP agent fleet, and the tools that would have prevented them

> First-hand field notes from the TechLead orchestrator that drove hugit's
> Day-0 → Wave-D3 build (≈52 work-packages across ~5 waves, 14-agent fan-outs,
> serial integration). Written 2026-06-06 after living every failure below.
> The point is not to complain — it is to convert scar tissue into **guards,
> rules, and tools** so the next fleet makes fewer of these mistakes.
>
> Meta-observation worth stating up front: **a large fraction of these pains
> are exactly what hugit is being built to solve.** The union landing queue,
> regen drivers, claim fences, memoized checks, intent/event log — each maps
> directly to a class of pain below. This retro doubles as product validation:
> we felt the customer's pain by hand-running the fleet.

---

## 0. The shape of the work (so the pains have context)

- Waves of 8–14 implementer agents, each in an **isolated git worktree**, each
  owning one WP with a frozen contract + a red acceptance suite.
- Every WP: agent builds → SEALs → orchestrator cold-verifies → rebases onto
  `main` → resolves conflicts → PR → CI green → merge → prune.
- **The orchestrator is the serialization point.** Fan-out parallelizes the
  *build*; integration is a serial funnel through one lead. That funnel is
  where most of the pain concentrated.

---

## 1. Environment & toolchain fragility (the most expensive class)

### 1.1 Concurrent `cargo` on a shared toolchain corrupted rustup — twice
14 agents building Rust simultaneously raced on rustup's component machinery.
Symptoms: `rustc is not applicable to toolchain`, then later the `rustup` proxy
binary itself vanished (every `~/.cargo/bin/*` symlink pointed at a missing
`rustup`). Full reinstall required, mid-wave, twice.

**Root cause:** N agents × `rustup`-proxied `cargo`/`rustc`/`clippy` all
resolving + potentially mutating shared toolchain state concurrently.

**Guards / tools that would have prevented it:**
- **`toolchain-prewarm` + `rustup`-component-lock guard** — a PreToolUse hook
  that refuses any `rustup install|component|toolchain` from a sub-agent
  (only the orchestrator pre-warms, once, before fan-out). We added this rule
  verbally mid-session; it should be a mechanized hook.
- **Point agents at the resolved toolchain bin directly** (`~/.rustup/toolchains/<v>/bin`
  on PATH) so `cargo` never goes through the mutable proxy. A dispatch-time
  env-injection.
- **Per-worktree `CARGO_HOME`/`CARGO_TARGET_DIR`** so builds never share a
  target dir or registry lock (also fixes 1.4).

### 1.2 `cargo-audit` (and `jj`) silently absent → every `wp-01` audit gate red
`cargo-audit` got wiped with the rustup churn; `jj` was never installed so
D2b's jj round-trip could only go PARTIAL. Every agent dutifully reported the
audit gate red as "environmental, pre-existing" — correct, but noise that
masked real signal.

**Tools:**
- **`hugit doctor` / preflight tool** — a single command run *before any fan-out*
  that asserts every tool the suites invoke is present and pinned: `cargo`,
  `clippy`, `rustfmt`, `cargo-audit`, `jj`, `gh`, `git ≥ X`, `docker` on the
  box. Fails closed with an install hint. No wave starts red on tooling.
- **Pin tool versions in a manifest** (`.hugit/toolbelt.toml`) the preflight
  checks against — so "works on my machine" drift is impossible.

### 1.3 PATH loss between calls (`cargo: command not found`)
Shell state does not persist between orchestrator Bash calls; `~/.cargo/env`
wasn't always sourced, so `cargo` vanished intermittently.

**Rule/tool:** a tiny **`with-toolbelt` wrapper** the orchestrator and every
agent prefix their build commands with — it sources the env + sets PATH
deterministically. One line, eliminates a whole class of flakes.

### 1.4 ENOSPC / disk spikes from concurrent `target/` dirs
Each worktree carried a full `target/` (~1 GB). 14 of them + `/private/tmp`
task transcripts spiked the FS to "0 MB free", killing commands mid-run.

**Guards/tools:**
- **Shared `CARGO_TARGET_DIR`** across worktrees (one cache, not N) — biggest
  single disk + compile-time win. (Caveat: needs the per-test FS-contention
  fix; see 1.5.)
- **Disk-headroom PreToolUse guard** — refuse to launch a new build agent when
  free space < threshold; tell the lead to prune first.
- **Auto-prune merged worktrees** on merge (we did this by hand every ~5
  merges; should be automatic in the loop).

### 1.5 Parallel `cargo test --workspace` flake (`Os code 22 InvalidInput`)
Running the full workspace test suite from multiple worktrees at once produced
non-deterministic FS-contention failures (notably `hugit-checks` c4). Passed in
isolation every time.

**Rule:** **never run `--workspace` test/clippy from more than one worktree
concurrently.** Integration verification must be serialized (it already had to
be, for merge ordering). Encode as a lead-side lock file.

### 1.6 Session de-auth killed 14 agents at once
A `/login` expiry mid-wave terminated the whole fan-out; 13 produced nothing,
1 recovered. Pure lost work + restart cost.

**Tools:**
- **Auth-liveness preflight + heartbeat** — check token TTL before fan-out;
  warn if it will expire within the wave's expected wall-clock.
- **Resumable dispatch** — a wave manifest (WP→branch→status) so a killed fleet
  re-dispatches only the unfinished WPs, idempotently, instead of all-or-nothing.

---

## 2. Agent lifecycle & orchestration pains

### 2.1 cwd-leak: agents (and the lead!) ran `git checkout -b` in the MAIN checkout
The single most recurrent orchestration bug. An agent whose shell cwd was the
shared main checkout (not its worktree) ran `git checkout -b wp/x`, leaving the
**main working tree parked on a WP branch**. Happened for C1, E2b, others — and
**I did it to myself** during integration (cwd persists between my Bash calls).

**Guards (high value, easy):**
- **`forbid-git-branch-switch-in-main` PreToolUse hook** — block `git checkout
  -b` / `git switch -c` / `git checkout <branch>` when `$PWD` resolves to the
  primary worktree (the main checkout). The only place branches get created is
  inside `.claude/worktrees/`. Fail closed with "you are in the main checkout".
- **Dispatch packet rule (mechanized):** every agent's first action is
  `cd "$WORKTREE" && [ "$(git rev-parse --show-toplevel)" = "$WORKTREE" ] || exit`.
  We told agents this in prose ("verify pwd before any git command"); make it a
  hook that injects + enforces it.
- **Lead-side: a `gw` helper** that always `cd`s by absolute worktree path and
  refuses relative `cd`. (My own leaks came from assuming cwd.)

### 2.2 Premature exit — agents finished the work but never committed
E3, D5, and E1a (first run) exited mid-build / mid-verification with the full
deliverable **uncommitted on disk**. The SEAL never arrived; only a process
death. Each required lead-recovery (verify the on-disk files + commit them).

**Guards/tools:**
- **`commit-or-blocked` exit gate** — an agent may only terminate after either
  (a) a committed SEAL, or (b) an explicit BLOCKED verdict. A wrapper that, on
  exit with a dirty tree and no SEAL, auto-stages + commits a `WIP(seal-pending)`
  so nothing is ever lost (lead decides to keep/redo).
- **Loop rule already in techlead-loop L4** ("never trust `completed` without a
  SEAL") — it held, but it's manual. Mechanize: the loop auto-runs the gate in
  the agent's worktree on any `completed`-without-SEAL and recovers.

### 2.3 Agent hung in an `until` loop for ~2 hours
E1a burned ~2h in a stuck shell `until` monitor (waiting on a condition that
never came), making no progress, holding its worktree lock. It could not
self-detect the stall.

**Guards/tools:**
- **Watchdog on agent wall-clock + progress** — if no new commit / file-write
  in the worktree for N minutes AND the agent is still "running", flag it to the
  lead (and optionally auto-stop). We diagnosed it by hand (file mtimes, proc
  scan); a `fleet-watch` tool should do this continuously.
- **Ban unbounded `until`/`while` polling in agent shells** — a PreToolUse
  guard that requires every `until`/`while` loop to carry a timeout/iteration
  cap. (The runtime even warns about this for the lead; extend to agents.)
- **Progress beacon** — agents emit a one-line `PROGRESS: <phase>` every major
  step; absence of beacons = stall signal.

### 2.4 Restart spawned DUPLICATE agents (gen-1 recovered + gen-2 fresh)
After the de-auth restart, some gen-1 agents recovered and finished the same WP
a fresh gen-2 agent was also doing (two D10s, etc.). Wasted compute + a dedup
burden at integration.

**Tools:**
- **Wave manifest as the single source of truth** (see 1.6). Re-dispatch reads
  it; a WP already `in_progress`/`sealed` is never re-launched. Idempotent waves.
- **Branch-name as a lock** — `wp/<id>` claimed in the manifest; a second agent
  for the same id refuses to start.

### 2.5 Worktree cleanup killed a live (recovered) agent's worktree
I pruned a worktree based on a "completed/failed" *notification*, but the agent
had actually recovered and was still writing. Nearly lost work (it was
committed/recoverable, but the risk was real).

**Rule (mechanized):** **never prune a worktree on notification status alone.**
The cleanup step must check: no live process referencing the worktree AND no
uncommitted changes AND the branch is merged. A `safe-prune` helper that
asserts all three.

### 2.6 Stale notification echoes burned orchestrator turns
A completed agent re-emitted its completion notification many times (the X3
echoer), each waking the orchestrator for a no-op. `TaskStop` couldn't help
(status was already `completed`).

**Tools:** notification **dedup/debounce** at the harness layer (one delivery
per terminal transition), and a lead-side "already-accounted" ledger so echoes
are dropped without a turn.

---

## 3. SEAL trust & verification pains (the AP-5 class)

### 3.1 An agent's SEAL under-reported its writes (claimed "zero outside writes")
A wave-D2 suite transcriber sealed "nothing outside tests/acceptance" but had
leaked 3 crate-side oracle stubs that broke the cargo lane. Caught **only** by
the lead's `git diff --name-only main..HEAD` scope sweep.

**Guards (this is the highest-leverage one):**
- **Mechanized scope-clean gate** — the integration step ALWAYS runs
  `git diff --name-only <base>..HEAD`, diffs it against the WP's declared
  Claims globs, and **refuses to merge** on any out-of-claim path. This is the
  authoritative disjointness check; it caught every real leak this session.
  It must be a hard gate, not a habit.
- **SEAL is auto-generated, not self-reported** — the return-shape's
  "files-outside-claims" field should be *computed* by a tool from the git diff,
  not typed by the agent. Agents cannot under-report what they don't author.

### 3.2 SEAL "GREEN" reflected the working tree, not the committed HEAD
D5 committed an early state, then kept fixing in **uncommitted drift**; its SEAL
ran against the working tree (green) but the committed HEAD was clippy-red on
lib tests. The fix lived only in the drift.

**Guards:**
- **Clean-tree assertion before SEAL** — verification must run against a clean
  checkout of the committed SHA (`git stash` / fresh worktree at HEAD), never
  the dirty working tree. "Green" must mean "the commit is green."
- **`no-dirty-on-seal` gate** — refuse a SEAL while `git status` is non-empty.

### 3.3 (Positive — preserve this) PARTIAL-over-fake held everywhere
Every blocked item (X4 roster, D2b jj, B6/B7 TESTLIST, X2 crypto) was reported
honestly as PARTIAL/BLOCKED with the reason, never faked green. **This is the
single most valuable behavior in the whole run** — it let the lead trust that a
red was always a real signal. Keep the rule loud, keep rewarding it, and make
the SEAL schema *require* a blocked-items list so "all green" is a deliberate
claim.

---

## 4. Oracle (acceptance-suite) design pains — the suites fought the rebase

The acceptance suites were authored by transcriber agents against a *baseline*
tree, then had to survive rebasing onto a moving `main`. Several oracle idioms
were unsound under that motion and produced false reds:

### 4.1 mtime guards (`find -newer Cargo.toml`) — unsound
Used to assert "WP-X didn't touch sibling Y's files." But **rebase rewrites
mtimes** (false positive) and **siblings are often unbuilt** (false positive).
Rewrote 9 suites.

### 4.2 Deferred `$TESTLIST` inside `bash -c '... $TESTLIST ...'` — always empty
The var wasn't exported, so the `bash -c` subshell saw nothing; every
test-declaration check failed regardless of reality. 6 suites.

### 4.3 BSD vs GNU grep (`grep -ql` silent on macOS) — false reds. 

### 4.4 Wave-isolation "sibling-absent" guards expire — and must be retired
A guard asserting "module Z does not exist yet" is correct only until the WP
that owns Z lands. Retired ~6 of these (b1, d1a, d1b, b4a, c5a, c4) one by one.

### 4.5 Grep-for-token trips on the comment asserting the token's absence
E1a's honest comment `no \`sync_from_github\`` tripped the no-reverse-sync grep.
E2a's submodule-list *comment* was load-bearing for "barrel exports X" checks.
Text-grep oracles conflate code with prose.

**Lessons → oracle authoring standard (a real deliverable):**
- **Disjointness/leak checks DO NOT belong in per-suite oracles.** They are
  unsound there (no git baseline, mtime fragility) and they churn (expire). The
  property is checkable *only* at integration with a git diff. → **Move
  disjointness to the orchestrator's scope-clean gate (3.1); ban mtime and
  cross-WP-absence guards in suites.** We did this; codify it.
- **Suites assert ONLY their own WP's structure + behavior** (claimed module
  present, items declared, items green), never a sibling's absence.
- **Oracle portability lints** — a `lint-oracle` tool that rejects a suite
  containing: `find -newer`, `grep -ql`, deferred `$VAR` in `bash -c`,
  cross-WP absence asserts, or token-greps that match comments. Run it when the
  red suite is committed, so the bug is caught before any implementer builds.
- **Test declarations via `cargo test --list` with exported TESTLIST + outer
  expansion** — make it a snippet in the suite template, not re-derived per agent.
- **Behavioral assertions over text-greps** — assert a `#[test]` passes, not
  that a string appears in a source file. Greps are a last resort and must
  target code (word boundaries, exclude comments).

### 4.6 Crate-side oracle stub leaks (twice)
Transcribers committed placeholder `acceptance_*.rs` into `crates/`, breaking
the cargo lane. Purged at consolidation both times.

**Rule:** the **suite-author agent writes ONLY `tests/acceptance/<wp>/run.sh`**;
the `acceptance_<wp>.rs` oracle is the *implementer's* claim. Mechanize as a
claims boundary in the suite-author's dispatch (and the scope-clean gate
catches violations).

---

## 5. Integration / merge mechanics — the serial funnel

Every WP merge cost a rebase + conflict resolution + verify + PR + CI + merge.
The conflicts were boringly predictable and almost entirely **shared-file
unions**:

| Conflict site | Frequency | Pattern |
|---|---|---|
| `CHANGELOG.md` `[Unreleased]` | **every WP** | append-only; everyone adds a line |
| `Cargo.lock` | **every WP** | take main's, regen |
| crate `lib.rs` barrel (`pub mod`/`pub use`) | every multi-writer crate | additive union |
| crate `Cargo.toml` (`[[test]]`, deps) | same | additive union |
| shared parent `mod.rs` (e.g. `import/`) | multi-WP submodule | additive union |
| duplicate `pub mod X` (E2a/E2b) | occasional | de-dup, don't double |

This is **exactly the union-tree landing problem hugit's B4 queue + C4 regen
drivers exist to automate.** By hand it was the bulk of the wall-clock.

**Tools/rules:**
- **`union-merge` driver for append-only files** — register `CHANGELOG.md` (and
  any `[Unreleased]`-style file) with a git merge driver that unions added lines
  instead of conflicting. Kills the #1 most frequent conflict outright.
- **`Cargo.lock` = derived, regenerate-never-merge** — a git driver / regen
  hook (this is literally hugit's C4 lockfile regen driver; dogfood it on
  ourselves). We took `main`'s lock + `cargo check` every time by hand.
- **Barrel-union helper** — a tool that, given a Rust `lib.rs`/`mod.rs` conflict
  where both sides only add `pub mod`/`pub use`/doc-comment lines, auto-unions
  by set-union of the declarations (the 95% case). Falls back to manual only on
  genuine overlap. ~12 manual unions this session; this would have done ~11.
- **Disjoint barrels by construction** — better: agents add a module by
  dropping a file in a `modules.d/`-style include dir or an auto-generated
  barrel (glob the module dir), so two agents never edit the same `lib.rs` line.
  This is the "eliminate the shared file before dispatch" principle (Pillar A)
  applied to Rust barrels — the single biggest structural fix.
- **Sequenced integration within a crate** — integrate all WPs of one crate
  back-to-back (rebase each onto the prior) so the barrel unions once per step,
  not in a tangle. We did this by hand; the loop should schedule it.

### 5.1 rebase-onto-stale-main → "PR not mergeable"
Rebasing WP-B before WP-A landed produced unmergeable PRs. Forced strict
serialization.

**Tool:** the loop already knows the DAG/merge-order — it should **rebase each
WP onto the *just-merged* tip automatically**, never onto a stale base.

### 5.2 Detecting off-baseline / uncommitted-work branches
Some branches sat at the base with work uncommitted (premature-exit); others
were committed. Had to inspect each (`git log` vs `git status`) to know which.

**Tool:** a `branch-state` reporter — for each `wp/*`: committed? ahead of base?
dirty worktree? merged? One table; drives the integration order.

---

## 6. Judgment calls the lead had to make (these are GOOD — keep them human)

Not every pain is automatable. Several needed real architecture judgment, and
**these are exactly where the lead must stay in the loop**:
- **Frozen-formula interpretation** — D1a's `principal_chain` length-prefix
  encoding (the contract said "length-prefixed"; the unique unambiguous reading
  needed a count prefix). Adjudicated + recorded.
- **Contract-vs-implementation crypto** — X2 used HMAC where the contract's
  "public verification procedure" demands an asymmetric signature; the lead
  authorized an `ed25519-dalek` dependency (the "no new deps" rule was a
  guardrail against frivolous deps, **not** against contractually-required
  crypto). This is a rigor *decision*, not a rigor loosening — and only a human
  (or the lead with the human's standing mandate) should make it.
- **Guard expiry** — deciding a wave-isolation fence has served its purpose and
  retiring it (with the property relocated to integration, never dropped).

**Rule:** keep an **adjudication log** (we did, in `.techlead/state/`) — every
non-mechanical decision recorded verbatim with its reasoning, so future critics
don't relitigate and so the human can audit the lead's judgment.

---

## 7. The 12 highest-leverage things to build (ranked)

1. **scope-clean merge gate** (mechanized `git diff` vs Claims globs) — caught
   every real leak; make it the hard gate. *(§3.1)*
2. **`forbid-branch-switch-in-main` hook** — kills the #1 recurrent orchestration
   bug (cwd-leak), for agents AND the lead. *(§2.1)*
3. **append-only union merge driver for CHANGELOG + regenerate-driver for
   Cargo.lock** — kills the two most frequent conflicts; dogfoods hugit's own C4.
   *(§5)*
4. **`hugit doctor` preflight** (tools present+pinned, auth TTL, disk headroom)
   — no wave starts red on environment. *(§1.2, §1.6, §1.4)*
5. **auto-barrel from a module-dir glob** (eliminate the shared `lib.rs` line) —
   removes ~all barrel-union conflicts structurally. *(§5)*
6. **clean-tree + commit-or-blocked SEAL gates** — "green" means the *commit* is
   green; nothing exits uncommitted. *(§3.2, §2.2)*
7. **wave manifest + idempotent re-dispatch** — survives de-auth/restart without
   duplicates or lost WPs. *(§1.6, §2.4)*
8. **`lint-oracle`** — reject unsound suite idioms (mtime, `grep -ql`, deferred
   `$VAR`, cross-WP absence) at suite-commit time. *(§4)*
9. **per-worktree CARGO_HOME + shared CARGO_TARGET_DIR + rustup-mutation ban** —
   end toolchain corruption + disk spikes + the parallel-FS flake. *(§1.1, §1.4, §1.5)*
10. **fleet-watch watchdog** (stall detection by worktree-progress + wall-clock;
    bounded-loop guard) — no more 2-hour `until` hangs. *(§2.3)*
11. **auto-generated SEAL** (files-outside-claims computed from git, not typed) —
    agents can't under-report. *(§3.1)*
12. **safe-prune** (no-live-proc + clean + merged) + auto-prune-on-merge —
    cleanup never races a live agent; disk stays healthy. *(§2.5, §1.4)*

---

## 8. The through-line

Two patterns explain ~80% of the pain:

1. **Shared mutable state under concurrency** — the main checkout (cwd-leak),
   the rustup toolchain, the target dir, the barrel files, CHANGELOG/Cargo.lock.
   Every one of these wants to be **eliminated, per-agent-isolated, or
   union-merged** *before* dispatch — never contended at integration. This is
   Pillar A ("eliminate the shared file before fan-out") and we paid for every
   place we didn't apply it.

2. **Trusting a report instead of computing the fact** — SEAL self-reports,
   working-tree "green", notification status for cleanup. The fix is always the
   same: **compute the fact from git/fs at the gate**, don't trust the narration.

And the happy through-line worth protecting: **honest PARTIAL/BLOCKED + a red
acceptance suite as the oracle** meant the lead could always trust a red. That
discipline — test-first, fail-closed, never fake — is what made an
otherwise-chaotic 52-WP night converge to a green `main` with zero known debt.
Everything above is in service of letting agents keep that discipline while
making the mechanical parts impossible to get wrong.
