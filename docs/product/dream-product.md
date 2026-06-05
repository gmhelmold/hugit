# hugit — the dream product

> **Working backwards, step 1: the finished product.** This document describes
> hugit as if the founder snapped his fingers and it exists, complete, in his
> hand. No roadmap, no phasing, no engineering constraints — those come later
> (decompose → white paper → refine → specs).
>
> **The bar (owner mandate, 2026-06-05):** *"Tudo que não for SOTA, não for
> fucking awesome, não for um wow factor, não entra."* Every feature below
> must be a wow. Anything that is merely "good" gets deleted in refinement.
>
> **Owner:** Gustavo Schneiter · **Draft v0.2:** 2026-06-05 · status: REFINING

---

## 0. The thesis

> **git optimized storage** (content-addressing — it won; we keep it whole).
> **GitHub optimized human collaboration** (the social layer — it won that era).
> **hugit optimizes ATTENTION — the scarce resource of the LLM era.**

An agent's context window and a human's focus are the two most expensive
resources in 2026 software. Today's repository spends both recklessly: agents
burn 30–50% of their context re-discovering the repo every session; humans
drown in prose PRs and 4,000-line logs. Every hugit feature exists to serve
**the minimal sufficient truth, at the right altitude, at the speed of
thought** — to machines and humans alike.

This document was co-designed with a fleet agent describing its own lived
pains. The seven that organize everything:

1. *"I'm born blind, every time"* — cold-start discovery burns the context.
2. *"I read too much because I can't read right"* — no altitude control.
3. *"I work with no ground"* — minutes of darkness between edit and verdict.
4. *"When something breaks, I receive garbage"* — prose logs, no causality.
5. *"My understanding dies with me"* — session knowledge evaporates.
6. *"I'm dangerous, so you babysit me"* — no fences → no real autonomy.
7. *"The machine is the wrong battlefield"* — the laptop as home, not window.

---

## 1. The snap of fingers — a day commanding a fleet

It's 9:00. You type one sentence: *"Ship the checkout flow."* Your
orchestrator decomposes it into 14 work-packages, declares the dependency DAG
to hugit, and asks for 14 agents. **hugit hands each one its dispatch packet**
— the minimal context bundle for its work-package, assembled by the forge
itself: the relevant code at the right altitude, the frozen contracts, the
conventions, the three past changes that touched this area and *why*. No agent
greps around for 40 minutes. No agent starts blind.

Each agent gets a **workspace** — born in under a second, hydrated from the
cache (deps, toolchain, build state), with its own virtual ports and database,
costing almost nothing because 95% of its bytes are shared content. Each
workspace is **fenced by its work-package's claims**: the agent physically
cannot touch what it didn't claim, and secrets never enter it at all.

As they work, every agent stands on **ambient truth**: each write triggers
speculative shadow checks on the affected targets (memoized — nearly free),
so a continuous green/red signal follows the work like a language server that
covers the entire reality of the repo. At 9:40, agent 7 writes into a file
that agent 3's work-package claims — **both orchestrators get a structured
conflict event within seconds.** The conflict never grows past one function.

At 10:55 a check goes red. The agent doesn't receive a log; it receives a
**diagnosis**: the culprit change (auto-bisected — memoization makes bisect
nearly free), the diff against the last green tree, the two suspect lines,
and a pointer to a similar failure fixed three weeks ago. Fix lands at 11:02.
One test in the suite has been flaky for a month; **the forge knows its
statistics and quarantined it by policy** — no agent burned an hour chasing it.

At 11:05 the landing queue **speculatively tests the union** of six green
branches — because A green and B green doesn't make A+B green — finds that
WP-4 + WP-9 break an integration test *together*, tells WP-9's agent exactly
why, and lands the other five in dependency order. No rebase cascade. The
lockfile "conflict" between two of them never existed: generated files are
**regenerated, not text-merged**.

Your phone buzzes once at 14:00 — not with 14 pull requests to read, but with
**one attention item, risk-ranked**: WP-11 touched an auth policy file, which
your policy marks human-mandatory. You open it: the diff at human altitude,
the agent's charter, the acceptance tests it added, three independent machine
verdicts, and the one question that actually needs you. Ninety seconds.

At 16:00, agent 12's session dies mid-work. The replacement agent attaches to
the same workspace and **inherits the journal**: the hypotheses, the dead
ends, the "it's THIS file that matters." It resumes the *understanding*, not
just the files. Zero re-discovery.

At 18:00 the feature is live. The history reads like 14 intents — each
carrying its why, its prompt, its verdicts, addressable forever — not 400
robot micro-commits. Six months later, an agent asks "why does checkout retry
3 times?" and the repo answers from the change itself. No archaeology.

Your Mac was at load average 1.2 all day. The bill is flat and smaller than
what CI minutes alone used to cost.

---

## 2. What hugit is (the object model)

Six first-class objects, all content-addressed, immutable, in the CAS:

| Object | What it is | What it replaces |
|---|---|---|
| **Blob/Tree** | git's data model, unchanged, served over the git wire protocol | git objects (compatible, never forked) |
| **Change** | a stable-identity unit of intent (survives rebase/amend; jj change-id semantics) carrying provenance: prompt, spec, charter, author (human/agent/model), verdicts | the commit-as-the-only-unit |
| **Workspace** | source + deps + toolchain + build state as one addressable snapshot; machines are cursors over it | clones, worktrees, dev envs, CI checkout |
| **Journal** | the transferable understanding of a session — hypotheses, dead ends, discoveries — bound to a workspace/change | tribal knowledge, Slack archaeology, re-discovery |
| **Verdict** | a structured machine/human judgment (APPROVE / FIX-FIRST / REJECT + evidence) bound to a Change at a tree-hash | PR review threads |
| **Check result** | the memoized output of `check(tree-hash, check-def)` — shareable, replayable, attestable | CI runs |

Everything else — branches, conflicts, plans, policies, events, the semantic
index — is a thin namespace over these immutable objects. **Git is kept
whole** (the data model won; the workflow lost); hugit's additions are
layered, never forked.

---

## 3. The features (every one a wow, or it doesn't enter)

### 3.1 The repo that explains itself — *kills "born blind" and "can't read right"*

- **A living semantic index, memoized per tree-hash.** Symbols, call graph,
  module contracts, ownership, conventions, decision links — maintained
  incrementally by the forge (it sees every change), never stale, never
  hand-written. The index is to understanding what the CAS is to bytes.
- **Altitude-controlled reading.** Any subtree, at any zoom: API surface →
  module contract → function signatures → full source. Generated once per
  tree-hash, cached forever, served in tokens-not-files. An agent reads a
  71-crate workspace the way you read a map — zooming, not scrolling.
- **Impact queries as API.** "Who calls this?" "What breaks if this signature
  changes?" "What's the blast radius of this change?" — answered from the
  build/call graph in milliseconds, hash-pinned. `hugit impact <change>` is
  the question every agent asks before every edit; today nobody can answer it.
- **"Why" is queryable.** Every line traces to its Change, every Change to its
  intent, every intent to its decision chain. "Why is this retry 3?" returns
  the founding reasoning, not `git blame`'s shrug.
- **Forge-generated dispatch packets.** Given a work-package and its claims,
  hugit assembles the minimal sufficient context bundle — relevant code at the
  right altitude, frozen contracts, conventions, adjacent history — as one
  addressable object. Dispatching an agent = handing it one URL. (What the
  techlead-pack discipline does by hand today, the forge does natively,
  instantly, always-current.)

### 3.2 Ambient truth — *kills "no ground" and "garbage on failure"*

- **Shadow checks on every write.** Affected-target checks run speculatively
  against workspace snapshots as the agent works — memoization makes them
  nearly free — so a continuous green/red signal follows the work. A language
  server tells you about syntax in milliseconds; hugit tells you about *the
  whole truth of the repo* in seconds. Edit-test darkness disappears.
- **Failures arrive as diagnoses, not logs.** A red check delivers a
  structured object: culprit change (auto-bisected), diff vs last green tree,
  suspect lines, and similar past failures with their fixes. Token cost of
  understanding a failure drops ~100×.
- **Auto-culprit on every regression.** Memoized checks make bisect nearly
  free, so the forge runs it automatically: any new red names its culprit
  with proof, fleet-wide, within minutes. "When did this break?" is never a
  human question again.
- **Fleet-wide flake intelligence.** The forge memoizes every check execution,
  so it *knows* each test's statistical behavior. Flaky tests are detected,
  scored, quarantined, and ticketed by policy. No agent, ever again, burns an
  hour chasing a known flake.

### 3.3 No agent starts cold — *kills "my understanding dies with me"*

- **Journals as first-class objects.** An agent's session understanding —
  hypotheses, dead ends, discoveries, "the file that matters" — attaches to
  its workspace and changes. A replacement agent resumes the *comprehension*,
  not just the bytes. Multiply by a fleet: the same discovery, paid once.
- **The knowledge layer.** ADRs, decisions, and the why of every Change are
  part of the graph, queryable by any actor. The repo becomes the
  organization's long-term memory — the exact thing fleets lack.

### 3.4 Safety by construction — *kills "babysitting"; unlocks real autonomy*

- **Capability-scoped workspaces.** An agent's reach = its work-package's
  claims, enforced physically: files, targets, network, secrets. `rm -rf` has
  nowhere to go. The blast radius is the claim, by construction.
- **Secrets never enter workspaces.** A broker signs/executes privileged
  operations on the workspace's behalf; the agent never sees a credential.
  (The CF write-only secret model, generalized.)
- **Everything attributable, everything reversible.** Every action by every
  principal (human, agent, orchestrator, model) is event-sourced: `hugit undo`
  works on the *forge*. Force-push data loss, orphaned commits, "who did
  this?" — structurally impossible.
- This is the feature that converts fear into autonomy: **you stop approving
  each edit not because models got better, but because the fences got real.**

### 3.5 Workspaces — *kills "the machine is the battlefield"*

- **`hugit ws spawn` → isolated workspace in <1s**, hydrated from CAS, deduped
  (a thousand workspaces ≈ 1× repo + deltas). The 28 GB/256-worktree blowup
  ceases to exist.
- **Local, remote, or both — transparently.** The same workspace object
  materializes on your laptop, a CoreLink runner, or an agent sandbox. Start
  local, resume in the cloud; attach your laptop to an agent's workspace and
  see exactly what it sees. **The laptop becomes a window, not a home.**
- **Virtualized side-state.** Per-workspace ports, env, scratch DBs — agents
  never collide on `localhost:5432` again.
- **Continuous snapshot, build state included.** "Resume where I left off" is
  a manifest download, not an hour of reinstall+recompile.
- **Auto-GC with total recall.** Idle workspaces evaporate to snapshots; landed
  branches fold away; nothing is ever lost (CAS), nothing lingers. The repo is
  impeccable by construction, not by discipline.

### 3.6 The Landing layer — *the merge, rebuilt as a continuous process*

- **The Plan is a forge primitive.** Work-packages, dependency DAG, claims
  (files, targets, contracts) — declared by the orchestrator, scheduled and
  enforced by the forge. The hand-built conflict-maps and merge-order
  spreadsheets of the techlead era become native physics.
- **Continuous speculative merge.** hugit maintains the live union of all
  in-flight branches; conflicts — textual, structural, semantic (build-graph,
  contract drift, registry collisions) — surface **at write time** as events,
  not at merge time as 540-line surprises.
- **Three-tier conflict resolution, server-side:**
  1. **Generated files: regenerate, never text-merge** (lockfiles, snapshots,
     codegen). The #1 measured git pain ceases to exist.
  2. **Code: AST-aware structural merge** for the semantically-disjoint
     changes text-diff falsely flags.
  3. **The remainder: LLM arbitration behind a confidence gate** — the
     resolution lands as a reviewable Change; below threshold, a structured
     conflict task to the right agent.
- **Speculative landing queue** (the SubmitQueue blueprint at fleet scale):
  union-test green branches, exploit build-graph independence to land in
  parallel, auto-reorder around failures. Main is *always* green — at
  thousands of landings a day if the fleet produces them.
- **Stacked changes native** with jj semantics: lower-stack amendments rebase
  the stack automatically; first-class conflicts absorb the cascade.
- **Tournament merges.** N competing implementations of one WP as sibling
  branches; a judge panel (machine verdicts + optional human) picks the
  winner; losers stay addressable as evidence. Exploration is a primitive.

### 3.7 Checks — *CI that never repeats itself*

- **A check is a pure memoized function**: `check(tree-hash, check-def) →
  result`, cached in the AC, shared across branches, stacks, and (for
  public-deterministic subtrees) across tenants. Re-verifying the verified is
  structurally impossible — the defining waste of 2026 CI, deleted.
- **Affected-target execution.** The forge understands the build graph (REAPI
  heritage): a one-crate delta runs one crate's checks.
- **Locally replayable, deterministic.** Check definitions are code; `hugit
  check run` on a laptop is byte-identical to the forge run — both are the
  same function of the same tree. Push-and-pray YAML dies.
- **Dependency updates as a forge feature.** A dep bump is a speculative,
  memoized landing pre-tested against *your* tree: green under policy → lands
  silently with a changelog entry; red → one structured task. Never 200
  PRs/week, never a rebase storm, never an agent's attention.

### 3.8 Review & policy — *verdicts, not threads*

- **The PR is a structured object**: charter, acceptance suite (the demand
  externalized as failing-then-passing tests), diff at both altitudes,
  evidence, machine verdicts, cost. Orchestrators consume data; humans get a
  rendered view; prose is the exception.
- **Policy-as-code, native, locally testable.** DCO, changelog gates, secrets
  scanning, coverage floors, "auth files need a human," "agents may land docs
  autonomously" — declarative, enforced identically everywhere. A bash-script
  gate museum becomes 30 lines of config.
- **Identity for every actor.** Humans, agents, orchestrators, models:
  first-class principals with attribution, permissions, budgets. "Which model
  wrote this line, under whose command, at what cost" is a query.
- **The attention queue.** The human inbox, risk-ranked by policy ×
  blast-radius × verdict-confidence. Twenty agent PRs a day become a 5-minute
  triage. **LGTM stops being the bottleneck of the entire machine.**
- **Provenance & attestation for free.** Every artifact traces to tree-hash,
  check-def, model, prompt, runner — SLSA-class supply chain falls out of the
  object model.

### 3.9 History at two altitudes

- **Machine altitude:** every micro-commit, every agent step, preserved and
  addressable (agents commit at machine speed; storage is CAS-cheap).
- **Human altitude:** the Change log — 14 intents, not 400 robot commits —
  auto-folded by intent, *not* squash-and-destroy. Both views, same data.

### 3.10 The platform — built for machine actors

- **Machine-paced API:** elastic concurrency; budgets and fairness by policy,
  not human-shaped rate limits. An orchestrator fanning 50 calls is the
  designed load, not an abuse pattern.
- **Guaranteed-delivery event stream.** The forge's history IS an event log;
  consumers subscribe, resume, replay from any offset. The 10-second
  fire-and-forget webhook dies; polling dies with it.
- **Everything addressable.** Every object — a conflict, a verdict, a
  workspace at a timestamp — has a stable hash URL you can hand an agent in a
  prompt.
- **Reliability as a feature**: multi-region, no shared fate with anyone
  else's incident weather. (The incumbent's 84.9% measured uptime is the
  easiest marketing we will ever do.)

### 3.11 The bridge — nobody has to know you left

- **One-command import** of a GitHub repo: full history, issues, PRs.
- **Bidirectional mirror, hugit-authoritative:** branches and PRs write back
  so teammates, OSS contributors, and badge-checkers see a normal GitHub repo.
  GitHub becomes a read-mostly window; the work happens here.
- **Exit guaranteed in writing.** Full-fidelity export at any moment (it's
  all git + documented JSON). Anti-lock-in is a feature, a weapon, and an
  honesty requirement at once.

### 3.12 Mission Control — one screen, the whole war

- The live DAG: every WP, agent, workspace; the speculative union state; the
  conflict heat-map; the landing queue; cache-hit rates; spend per principal.
- Fleet-level verbs: approve, redirect, abort, re-plan. Drill from "the plan"
  to "agent 7's workspace 40 minutes ago" (event-sourced time travel) in two
  clicks.

---

## 4. What ceases to exist (the anti-features)

Working backwards includes naming the pains that are simply *gone*:

| Today's pain (measured, in `../research/`, or lived) | In hugit |
|---|---|
| Agents burning 30–50% of context on cold-start discovery | semantic index + dispatch packets; orientation is served, not excavated |
| Reading raw files for lack of zoom | altitude-controlled reading, memoized per hash |
| Minutes of darkness between edit and verdict | ambient truth: shadow checks follow the work |
| 4,000-line prose logs on failure | structured diagnoses with auto-bisected culprits |
| Hours chasing known-flaky tests | fleet-wide flake intelligence, quarantine by policy |
| Session dies → understanding dies | journals; no agent starts cold |
| Babysitting agents edit-by-edit | capability-scoped workspaces; fences enable autonomy |
| Lockfile/generated-file conflicts (#1 git pain) | structurally impossible — derived files regenerate |
| 27.67% of agent PRs conflict, 540 lines avg | write-time conflict events, fenced by claims |
| A+B red when A and B are green | speculative union testing before landing |
| CI re-runs everything; 382-run queues, p50 212 min | memoized checks + affected targets |
| 28 GB of worktrees, 700 stale branches | CAS-deduped workspaces, auto-GC with total recall |
| Dependabot floods, rebase storms | silent pre-tested dep landings by policy |
| Force-push data loss, reflog spelunking | event-sourced forge; universal undo |
| LFS pointer files, lock-in, double billing | native large blobs, zero egress |
| YAML push-and-pray | checks are locally replayable functions |
| Human-paced rate limits strangling orchestrators | machine-paced budgets by policy |
| Webhooks dropped after 10s | replayable guaranteed event stream |
| The human reading 20 prose PRs/day | risk-ranked attention queue + structured verdicts |
| Per-minute billing on your own hardware; 10–50× shocks | flat, predictable; your compute is never metered |
| The founder's Mac as CI infrastructure (load 30, 69 MB free) | the laptop is a window; load average 1.2 |

---

## 5. Design tenets (the commandments)

1. **Don't deviate from git.** The data model won. Compatibility is sacred;
   additions are layered, never forked. (The name is the strategy.)
2. **Attention is the scarce resource.** Serve the minimal sufficient truth,
   at the right altitude, at the speed of thought — to machines and humans.
3. **The orchestrator-plus-fleet is the user.** Every surface has two readers,
   human and machine, and the machine's view is never an afterthought.
4. **Conflicts are objects, never walls.** Nothing blocks; everything
   schedules.
5. **Never verify the already-verified.** Content-addressing makes repetition
   a bug — economically and morally.
6. **Conflict at write time beats conflict at merge time** — by hours, and by
   hundreds of lines.
7. **Generated files are derived, not authored.** Regenerate; never text-merge.
8. **Intent travels with the change.** The why is part of the object, forever.
9. **Fences enable autonomy.** Capability-scoping and universal reversibility
   are what convert babysitting into delegation.
10. **Policy is code; judgment is configured, not improvised.** The human
    decides once; the forge enforces always.
11. **Flat, predictable pricing. Never meter the customer's own compute.**
12. **Exit is guaranteed.** Embrace requires trust; trust requires the door
    to be visibly unlocked.

---

## 6. Open questions for refinement (owner input wanted)

1. **Scope: issues/planning.** Does hugit own work-tracking (the Plan/WP
   primitives suggest yes — acceptance-suite-as-the-issue), or integrate with
   external trackers and stay merge-centric? Draft leans: **own it,
   minimally** — Plan/WP as primitives, not a Jira clone.
2. **Open-source posture.** Protocol + CLI + check-runner open source
   (community embrace, jj-style trust), cloud forge as the paid
   network-effect product? Draft leans: **open-core.**
3. **Local-first depth.** Fully operable offline (local daemon,
   sync-on-reconnect) or cloud-authoritative with rich local caches? Draft
   leans: **cloud-authoritative + rich local cache** (CAS makes offline reads
   trivial; offline *landing* intentionally out).
4. **Does hugit run the agents?** BYO-orchestrator (Claude Code, Devin,
   OpenHands) against the API as first-class, with hosted sandboxes as an
   upsell? Draft leans: **yes — hugit is the ground the war is fought on, not
   one of the armies.**
5. **The human surface.** Mission Control web-first; CLI/TUI at parity; IDE
   extensions — what's in the dream's v1 picture?
