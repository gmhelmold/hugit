# Research — multi-agent / parallel-development workflow pains

> Lane 3 of the 4-agent evidence sweep behind the founding brief.
> Researched 2026-06-05 (Opus web-research agent). Citations inline.

## Top workflow pains (ranked)

1. **Integration/landing bottleneck > review bottleneck.** Agents make producing overlapping work cheaper than reconciling it; one dev now creates the concurrency a whole team used to. *Who:* anyone running >2 parallel agents. *Evidence:* ctx.rs "Agents have a *landing* problem… 'Change A can be green on its own. Change B can be green on its own. A plus B can still be red.'… 'A local agent merge queue protects one developer from the concurrency they can now create for themselves'" (https://ctx.rs/blog/merge-queue-for-agents/). **Severity: critical.**

2. **Merge conflicts are frequent and large in agentic PRs.** *Evidence:* AgenticFlict — 142,652 agentic PRs / 59,412 repos; **27.67% had merge conflicts**, 336K conflict regions, **avg 540 conflicting lines / 4.36 files per conflicting PR**; per-agent: Codex 31.85%, Claude Code 25.93%, Devin 22.85%, Cursor 19.75%, Copilot 15.24% (https://arxiv.org/html/2604.03551v2). **Severity: critical.**

3. **Review can't keep up (LGTM is the constraint).** *Evidence:* AI usage correlates with **98% more PRs, 154% larger PRs, 91% longer review times**; "human 'Looks Good To Me' has become the single biggest liability in the deployment cycle" (https://www.aviator.co/blog/the-ai-code-verification-bottleneck.../); OSS maintainers "drowning in AI-generated PRs… enterprise teams next" (https://thenewstack.io/ai-generated-code-crisis/); GitHub's own guidance acknowledges the flood (https://github.blog/ai-and-ml/generative-ai/agent-pull-requests-are-everywhere-heres-how-to-review-them/). **Severity: critical.**

4. **Semantic/architectural conflicts invisible to git.** Branches auto-merge cleanly yet break combined; "Git can help align the text. It cannot tell you whether the combined branch still reflects one coherent direction" (ctx.rs). Academic corroboration: SAM/SemanticMerge, DeltaImpactFinder, change-impact-analysis literature (https://www.sciencedirect.com/science/article/pii/S0164121224001158). **Severity: high.**

5. **CI queue saturation + wasted compute.** Full suites re-run on every WIP push across many agent branches. *Evidence:* real self-hosted saturation snapshot "382 queued runs, p50 queue age 212.5 min" (https://github.com/zeroclaw-labs/zeroclaw/issues/2299); remedy is tiered CI + affected-package detection (Turbo/Nx/Bazel) + merge-once (https://github.com/microsoft/apm/issues/770). **Severity: high.**

6. **Worktree disk blowup + branch sprawl.** *Evidence:* practitioners hit "256+ worktrees consuming 28GB, 700+ stale local branches, 70% referencing branches merged months ago"; each worktree duplicates node_modules (gigabytes); "for a team running 10–20 agent sessions/day, this becomes a real disk problem within a week" (https://www.gitworktree.org/guides/best-practices, https://pnpm.io/next/git-worktrees). **Severity: high.**

7. **Lockfile/central-registry conflicts + port collisions.** "Two agents adding separate features often need to register those features in the same central file" (lockfiles, route tables, DI registries); each worktree needs its own DB/ports (https://zenvanriel.com/.../running-multiple-ai-coding-agents-parallel/, Claude Code worktree docs). **Severity: medium-high.**

8. **Context loss / cascade rebase between sessions.** Stacked-PR change in lower PR cascades conflicts upward; "cascading rebase conflicts remain an unresolved technical problem" (HN https://news.ycombinator.com/item?id=32216027; https://www.infoq.com/news/2026/04/github-stacked-prs/). **Severity: medium.**

## Current tooling landscape + declared gaps

- **Worktree mgrs / orchestrators:** Claude Code built-in worktrees + `isolation: worktree` subagents (https://code.claude.com/docs/en/worktrees); **Conductor** (Melty), **Crystal→Nimbalyst** (deprecated Feb 2026), **claude-squad** (tmux multiplex), **gw/uzi**, awesome-agent-orchestrators list (https://github.com/andyrewlee/awesome-agent-orchestrators). *Declared gap:* they isolate execution but **stop at the merge/integration boundary** — "per-edit approval, human-in-the-loop coordination," no combined-state verification.
- **Sandboxes (container-per-agent):** **E2B** (Firecracker microVM, ~150ms), **Daytona** (sub-90ms, raised $24M), **Modal** (GPU), **Morph** (forks running VM into parallel copies in 250ms for branching exploration), **Vercel Sandbox** (https://www.startuphub.ai/ai-news/.../daytona-vs-e2b-vs-modal-vs-vercel-sandbox-2026). *Gap:* compute isolation solved; **state/version/merge layer is not their problem** — they hand back a diff and stop.
- **Agent code review:** **CodeRabbit** (pre-merge checks, NL policy gates), **Greptile** (whole-repo index, catches cross-diff/caller issues), **Graphite Diamond** (low false-positive complement). *Gap:* still emit **prose comments to humans**; detection rates 24–46% (https://macroscope.com/content/best-ai-code-review-tools-github-2026). Empirical study tempers vendor claims (https://arxiv.org/pdf/2604.03196).
- **Agent merge tooling (nascent):** **Clash** CLI (shows conflicts immediately, Show HN https://news.ycombinator.com/item?id=46887382), **Weave** language-aware/entity merge (https://news.ycombinator.com/item?id=47241976), GitHub native **Merge Queue + stacked PRs** (Apr 2026). *Gap:* still text-merge-centric, remote-only, no content-addressed memoization.

## Big-tech internal prior art (validates memoized CI / speculative merge)

- **Uber SubmitQueue** (EuroSys'19) — keeps mainline green at thousands of commits/day via **speculative builds**, **parallel independent evaluation when changes touch no shared build targets**, **ML to predict change success/build time**, **conflict analysis via the build graph** (https://www.uber.com/blog/research/keeping-master-green-at-scale/; https://www.uber.com/blog/bypassing-large-diffs-in-submitqueue/). This is almost exactly the orchestrator+fleet integration problem.
- **Google TAP** presubmit + post-merge with result memoization; **Bazel remote cache** = content-addressable action cache (action-hash → result) + CAS, "if test inputs haven't changed, reuse cached result" (https://bazel.build/remote/caching). **Memoized-CI-by-content-hash is proven at Google/Uber scale** — directly transferable to a CAS-backed forge.
- Affected-target/test-impact analysis (Bazel/Nx/Turbo) is the accepted scale answer to "don't run the full suite per PR."

## Explicit 'git/GitHub for agents' attempts

- **GitAgent Protocol / OpenGap** — "the agent IS a git repo"; identity/rules/memory/skills version-controlled; `gitagent validate` in CI (https://www.gitagent.sh/, https://github.com/open-gitagent/gitagent). *Note:* this is agents-as-repos, **not a forge for agent-produced code**.
- **AgentGit** (arXiv 2511.00628) — Git-like state commit/revert/branching for multi-agent LangGraph trajectories.
- **Git Context Controller (GCC)** (arXiv 2508.00031) — COMMIT/BRANCH/MERGE/CONTEXT ops over agent *memory*.
- *White space:* all target agent **memory/trajectory** versioning. **No one is building a content-addressed VCS/forge whose merge, CI-memoization, and review objects are designed for orchestrator+fleet *code* integration.** That niche is open.

## Trend numbers

- **Most rigorous:** 4.2M-developer study (Nov'25–Feb'26) → **26.9% of production code AI-authored** (https://lenz.io/c/ai-code-generation-2026-...). GitHub: **46%** of code on platform AI-generated. Survey self-report: **42% AI-assisted**, 72% of AI-tool users use daily (https://shiftmag.dev/state-of-code-2025-7978/).
- **Multi-file/agentic shift:** Claude Code multi-file edit sessions **34% (Q1'25) → 78% (Q1'26)**; **57% of orgs run multi-step agent workflows**; 23% scaling agentic AI.
- **Fleets normalizing:** OpenHands SDK "run locally or **scale to thousands in the cloud**"; "engineers are setting off **fleets of agents**" (https://www.openhands.dev/blog/automating-massive-refactors-with-parallel-agents); Devin "**parallel fleet** for well-scoped maintenance at scale"; Cognition raised at **$26B** betting agent-first beats IDE tools (https://www.techtimes.com/articles/317354/...). Anthropic/OpenAI engineers claim ~100% AI-written code internally (https://fortune.com/2026/01/29/...).

## Implications for an LLM-native forge (CAS-backed)

1. **The wedge is integration, not authoring/review** — own the "landing problem" the orchestrators explicitly punt on; a per-developer/per-orchestrator merge queue is the unmet primitive (ctx.rs).
2. **Speculative/optimistic combined verification** — replay branch sets against target and test A+B+C together before landing; Uber SubmitQueue is the proven blueprint.
3. **Content-hash-memoized CI is your home turf** — CAS already deduplicates; key CI results by action hash so the 28GB/382-queued-runs waste collapses; this is Bazel/TAP-proven and aligns with CoreLink's existing CAS/AC + REAPI surface.
4. **Build-graph conflict analysis, not text diff** — declare two agent branches independent iff they touch disjoint build targets → parallel land; surface semantic/architectural conflicts (central-registry, lockfile, contract drift) pre-merge.
5. **Structured, machine-consumable PR/review objects** — emit JSON review/verdict objects (APPROVE/FIX/REJECT + evidence) for orchestrator consumption, not prose comments humans must read; review-as-data.
6. **Policy-as-code merge gates** — NL/codified pre-merge checks (CodeRabbit-style) as first-class, enforced by the forge.
7. **Solve worktree/branch sprawl at the storage layer** — CAS-backed worktrees dedupe node_modules/build artifacts (pnpm proves the model); auto-GC merged branches/worktrees; bound the 256-worktree/700-branch blowup.
8. **Memoize per-content, not per-commit** — identical sub-trees across agent branches share cache/test results; the multi-tenant network effect (more agents → fuller cache) is the same moat as the cache product.
9. **Lockfile/registry-aware merge** — auto-reconcile the "everyone edits the central file" class (Weave-style entity merge) instead of conflicting.
10. **Position as natural evolution of the cache** — same CAS substrate, same win-win (faster+cheaper integration), same moat; this is the CI/build-acceleration expansion lane made concrete, with merge/landing as the new defensible primitive.
