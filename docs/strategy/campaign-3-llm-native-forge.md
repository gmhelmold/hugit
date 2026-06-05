# hugit — the LLM-native VCS + forge (CoreLink Expansion Campaign #3)

> **Status:** founding strategy brief. Campaign #3 of the CoreLink product
> family — post-launch (phase 3++) relative to CoreLink's route. NOT a change
> to CoreLink's launch, NOT a change to campaigns #1 (CI runners) or #2
> (Workspaces) — it is the next floor of the same building. Campaign #2's
> caveat #5 named "the mutable namespace over the immutable CAS with git-like
> semantics" as the genuinely new engineering; this brief is what that
> namespace becomes when finished: a version-control + merge + CI surface
> designed for orchestrated fleets of AI agents.
>
> **Owner:** Gustavo Schneiter · **Drafted:** 2026-06-05 ·
> **Evidence base:** 4-agent research sweep, 2026-06-05 (`../research/`).

---

## The one-sentence thesis

**Git is a content-addressed store with refs on top — and CoreLink already runs
the content-addressed store. The forge is the third campaign on the same
primitive: a forge where worktrees are workspace snapshots, conflicts are
first-class objects, checks are memoized by content hash, merges are speculative
and continuous, and a PR is a structured machine verdict — built for the
orchestrator-plus-fleet workflow that git and GitHub were never designed for.**

---

## Why now — the landing problem (the evidence)

The 2026 pain is not *writing* code; it is **landing** it:

- **26.9% of production code is AI-authored** (4.2M-developer study, Nov'25–Feb'26);
  Claude Code multi-file sessions went 34% → 78% in a year; OpenHands/Devin/Cognition
  sell "fleets of agents" as the unit of work.
- **27.67% of 142,652 agentic PRs hit merge conflicts** — avg 540 conflicting lines
  across 4.36 files ([AgenticFlict](https://arxiv.org/html/2604.03551v2)).
- **Agent PRs wait 4.6× longer and land at 32.7% acceptance vs 84.4% for humans**
  ([codeant](https://www.codeant.ai/blogs/top-pull-request-automation-tools)).
- The orchestrator ecosystem (Conductor, claude-squad, worktree managers) isolates
  *execution* and **stops at the merge boundary**: *"Change A can be green on its
  own. Change B can be green on its own. A plus B can still be red."*
  ([ctx.rs](https://ctx.rs/blog/merge-queue-for-agents/)).
- Meanwhile GitHub is bleeding trust: **257 incidents in 12 months (~84.9% measured
  90-day uptime)**, the self-hosted-runner fee backlash, **Copilot agentic bills up
  10–50×** after the Jun 2026 usage-billing switch, Ghostty and Zig leaving, the
  post-Dohmke leadership void
  ([IncidentHub](https://blog.incidenthub.cloud/github-reliability-outage-history-2025-2026),
  [LeadDev](https://leaddev.com/software-quality/whats-gone-wrong-at-github)).

And the founder is **customer #0**: the `techlead` skill's conflict-map, DAG
scheduler, merge-order, SEAL, and post-flight sweep are hand-built prosthetics
for exactly what this forge does natively.

---

## One primitive, continued

| Campaign | Surface | What it actually is |
|---|---|---|
| #1 CI runners | ephemeral compute | **workspace + a command** |
| #2 Workspaces | snapshots/sandboxes | **the workspace as a CAS object** |
| **#3 hugit** | VCS + merge + checks | **the namespace, history, and policy over workspaces** |

Nothing is built twice: #1 supplies the compute shell, #2 supplies hydration and
the manifest tree, #3 adds refs, merge, identity, and policy on top.

---

## What "LLM-native" means (product pillars)

1. **Worktree = workspace snapshot** (#2, verbatim). N agents = N disposable
   cursors over one durable object; CAS dedup ends the documented
   256-worktrees/28 GB blowup. Branches/snapshots become nearly free — the agent
   primitive ([AgentGit](https://arxiv.org/abs/2511.00628) validates demand).
2. **First-class conflicts, server-side** (jj's killer feature, which no forge
   offers): a conflict is a stored object, never a blocking state. Fleets keep
   moving; resolution is a task, not a wall.
3. **Continuous speculative merge.** The orchestrator declares the WP DAG; the
   forge continuously computes the merged state of all in-flight branches and
   surfaces conflicts **at write time, not merge time**. Proven blueprint:
   [Uber SubmitQueue](https://www.uber.com/blog/research/keeping-master-green-at-scale/)
   (speculative builds, build-graph conflict analysis, thousands of commits/day).
4. **Memoized checks = the AC.** A check is a function of the tree hash —
   Google-proven (TAP presubmit, [Bazel action cache](https://bazel.build/remote/caching)).
   Merging N green branches whose union tree was already tested = **zero re-runs**.
   This is campaign #1's win-win applied to CI itself: instant for the customer,
   near-zero COGS for us — and it kills our own lived pains (CoreLink's heavy
   gates moved off-PR 2026-06-02, dependabot rebase re-floods).
5. **Semantic merge as a server feature.** Lockfile/registry-aware drivers
   ("regenerate, don't text-merge" — the #1 measured git pain), AST-aware merge
   (Mergiraf-class), LLM arbitration behind a confidence gate. The research
   exists (ConGra, ChatMerge); **no forge ships it**.
6. **PR/review as a structured object.** Machine verdicts
   (APPROVE / FIX-FIRST / REJECT — the techlead-verify return shape), evidence
   attached, consumed by orchestrators — not prose threads humans must parse.
   Policy-as-code gates native: CoreLink's DCO/changelog/secrets-matrix scripts
   become forge config.
7. **Agent-paced platform.** Elastic machine-rate budgets, first-class bot
   identity/attribution, guaranteed event delivery — vs GitHub's human-paced rate
   limits and 10s-timeout fire-and-forget webhooks.
8. **Native large files over R2 (zero egress)** — transparent large blobs, no
   pointer files, no history rewrite: directly attacks documented LFS hate and
   S3-based competitors' egress COGS.

---

## Why CoreLink wins this

- **The hard part is in production:** chunking, Merkle manifests, multi-region
  CAS, tenant isolation, PAT auth, fail-CLOSED audit, memoized AC. Serving the
  git wire protocol from an app-level content-addressed store is **battle-tested
  prior art** — GitHub's own Spokes/DGit, GitLab's Gitaly, libgit2 pluggable
  ODB backends. Engineering, not research.
- **The network effect now applies to checks too:** a memoized check result on a
  public-dep subtree is shared the way the cache is shared. More tenants → more
  pre-verified trees → faster + cheaper landing for everyone.
- **jj is the front door we don't have to build:** 27k stars, Google-funded,
  first-class conflicts — and **explicitly no native forge** (it reuses git
  forges). Support git *and* jj (change-ids, stacked changes) over the same CAS:
  **be the forge jj doesn't have.**

---

## Strategy — embrace, don't assault

GitHub's moat is **social** (180M+ devs); a frontal attack loses. The wedge is
the segment it serves worst and that grows fastest: **teams operating agent
fleets.** The compat ladder:

1. **Git wire protocol** (clone/push/pull unchanged) — table stakes; teammates
   don't notice.
2. **The landing layer rides ON GitHub first** — an agent merge queue with
   speculative union-testing and memoized checks, mirroring into GitHub
   (Pierre's trust unlock; Graphite's adoption path). No migration ask.
3. **Bounded bidirectional mirror** — GitHub App + webhook-driven, forge-
   authoritative with write-back; never naive symmetric replication (rate
   limits, push races are the documented failure mode).
4. **Authoritative forge** — only after the bridge has earned trust.

Land-and-expand from #1/#2: *"your cache, runners, and workspaces already live
here — point your agents' branches here too."*

The **naming principle is the product principle**: don't deviate from git.
LLMs are trained deeply on git; humans have two decades of muscle memory. Every
deviation costs adoption on both axes. (hence: **hugit** — git kept whole in
the name, plus the strategy itself: *hug it*.)

---

## Competitive reality (researched 2026-06-05)

- **GitHub Agent HQ** — the orchestration surface exists, but it's a walled
  garden riding a billing backlash, on primitives that DDoS themselves.
- **Cursor + Graphite** (>$290M acquisition, $29.3B parent) — the most credible
  12-month forge entrant. **This is the clock on the wall.**
- **Pierre** ($23.5M, YC) — same-ish thesis, GitHub mirror shipping, but
  human-team-first, not fleet-first.
- **GitButler** ($17M a16z, Chacon) — agent-aware client *on top of* GitHub.
- **Verdict:** nobody yet combines **fleet-native forge + git compat + GitHub
  mirror + owned cache/CI/workspace economics.** The intersection is open; the
  window is short.

## Economics

- An agent-heavy developer today pays **$200–600/mo** across GitHub + Copilot
  agents + runners + Graphite + CodeRabbit + sandboxes. The forge is the
  consolidation play: one flat, predictable bill (house philosophy — no
  usage-billing whiplash; GitHub's 10–50× shock is the open wound we never
  replicate; never charge for the customer's own compute).
- COGS: memoized checks cut CI compute 5–10× (campaign #1's table, applied to
  every PR); namespace/merge ops are Workers/D1-cheap; R2 zero egress on every
  hydration and large-file pull. AI-code-tools TAM: $9.5B (2026) → $22B (2030).

## Honest caveats (go in eyes-open)

1. **Biggest engineering lift of the three campaigns** — protocol-v2 server,
   namespace layer, speculative merge engine. Not a byte of it leaks into
   CoreLink's launch route or #1/#2 critical paths.
2. **GitHub's social moat is real.** We win the *workflow*, not the community
   tab — issues/stars/discussions stay on GitHub for a long time. Fine.
3. **The bridge is the Achilles' heel.** A broken mirror kills trust instantly.
   Forge-authoritative with bounded write-back, idempotent sync, GitHub App auth.
4. **The window can close** — Cursor/Graphite or GitHub itself. Speed beats
   completeness: the landing layer (step 2) ships first because it needs only
   cache + AC + API, no forge.
5. **Source-of-truth duty is a class above cache duty.** Cache loss = recompute;
   repo loss = catastrophe. Durability discipline goes up accordingly
   (replication, export guarantees, escrow) — and single-vendor Cloudflare risk
   now sits on the money path.
6. **Semantic/LLM merge needs a confidence gate.** A wrong auto-merge is worse
   than an honest conflict.

---

## Decision recorded

- **Name:** **hugit** (decided 2026-06-05) — principle: stay maximally close to
  git for human adoption and LLM affinity; the name doubles as the strategy
  ("hug it" = embrace the community).
- **Shape:** campaign #3 on the same primitive stack — cache (launch) →
  compute (#1) → workspace (#2) → **namespace + merge + policy (#3)**. One
  product family; hugit consumes CoreLink's CAS/AC, never forks it.
- **Wedge:** agent-fleet teams; first SKU is the **landing layer** (agent merge
  queue + memoized checks riding on GitHub), forge promotion later.
- **Timing:** post-launch relative to CoreLink; sequenced after #1 fires. The
  landing layer may fire between #1 and #2 surfaces since it rides on existing
  CAS/AC.
- **Sequence:** CoreLink launch → dogfood → #1 runners → landing layer on
  GitHub → #2 snapshot/hydrate → git-protocol forge → jj-native + mirror →
  authoritative.
- **Home:** this repo (`hugit`), founded 2026-06-05 — separate from
  corelink-server so strategy/build work never collides with CoreLink's launch
  sessions.
