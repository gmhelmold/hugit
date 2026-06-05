# Research — competitive landscape + market wedge

> Lane 4 of the 4-agent evidence sweep behind the founding brief.
> Researched 2026-06-05 (Opus web-research agent). Citations inline.

## Competitive map (incumbents / challengers / adjacents)

**INCUMBENTS**

- **GitHub (Microsoft)** — 180M+ devs, 36M added in 2025, 630M repos, 90% of Fortune 100; 42% AI-assistant share ([sqmagazine](https://sqmagazine.co.uk/github-statistics/), [coinlaw](https://coinlaw.io/github-statistics/)). *Agent-readiness: HIGH* — **Agent HQ** (Oct 2025) makes it the multi-agent orchestration hub: third-party agents (Anthropic, OpenAI, Google, Cognition, xAI) under one Copilot subscription, Mission Control assigns work in parallel, each session gets its own git worktree/branch/PR ([github.blog](https://github.blog/news-insights/company-news/welcome-home-agents/)). Build 2026 added multi-agent VS Code, Agent Sandbox (ephemeral container/task), Autonomous/Fleet/Autopilot modes (Jul 2026) ([techtimes](https://www.techtimes.com/articles/317596/20260602/github-copilot-replaces-gpt-4-project-polaris-ships-multi-agent-vs-code-build.htm)). *Threat: VERY HIGH — but see backlash.* **Weakness for fleets:** the new usage-based Copilot billing (Jun 1 2026) drives agentic bills **10x–50x**; code review now also burns Actions minutes ([github.blog](https://github.blog/news-insights/company-news/github-copilot-is-moving-to-usage-based-billing/), [techtimes](https://www.techtimes.com/articles/317536/20260601/github-copilot-pricing-change-drives-backlash-agentic-bills-jump-10x-50x-power-users.htm)). Cost of running fleets on GitHub is now the open wound.
- **GitLab** — Duo (Vertex/Google models); ~$8B, **in active acquisition talks with Datadog** since 2024, still unresolved Oct 2025 — strategically distracted ([siliconangle](https://siliconangle.com/2024/07/17/report-github-rival-gitlab-acquired-datadog/), [techtarget](https://www.techtarget.com/searchitoperations/news/366596593/)). *Agent-readiness: MED. Threat: LOW for our wedge.*
- **Bitbucket** — declining mindshare, not a factor for agent-native. *Threat: LOW.*
- **Forgejo/Codeberg/Gitea** — Forgejo the new self-host default ("90% of new self-hosters in 2026"), Codeberg 300k+ repos, Fedora migrated off Pagure ([serverspan](https://www.serverspan.com/en/blog/the-2026-guide-to-self-hosted-git-gitea-forgejo-and-the-future-of-code-hosting)). Governance/sovereignty play, **not** agent-native. *Threat: LOW — but a license/UX baseline and possible upstream.*
- **SourceHut / Radicle / Tangled** — niche/ideological (P2P, atproto). No agent thesis. *Threat: NONE.*

**CHALLENGERS (new-wave forges)**

- **Pierre (pierre.co)** — **closest direct competitor.** YC W23, $23.5M (CRV, O1A), Jacob Thornton/Ian Ownbey. Explicit thesis: opinionated git platform rebuilt from the metal "for small, focused teams… augmented by AI" — hosting + realtime review + scriptable CI + **auto-mirror to GitHub** ([ycombinator](https://www.ycombinator.com/companies/pierre), [docs.pierre.co](https://docs.pierre.co/integrations/github)). Real traction: 9M repos created in 30 days, peaks of 15k repos/min ([aipure](https://aipure.ai/products/pierre)). *Agent-readiness: MED-HIGH (human-team-first, not fleet-first). Threat: HIGH — same wedge, already shipping the GitHub-mirror bridge.*
- **GitButler** — Scott Chacon (GitHub cofounder), $17M Series A a16z (Apr 2026); "what comes after Git" for AI: parallel virtual branches, agent-specific commands, auto-stacked GitHub PRs ([blog.gitbutler](https://blog.gitbutler.com/series-a), [a16z](https://a16z.com/announcement/investing-in-gitbutler/)). *Client, not a forge (yet) — sits on top of GitHub. Threat: MED, could move down-stack.*
- **jj (Jujutsu) / Sapling** — git-backend VCS, frictionless GitHub interop; the model for "live beside GitHub" ([github.com/jj-vcs/jj](https://github.com/jj-vcs/jj)). Tooling, not a forge. *Validates the bridge thesis.*

**ADJACENTS (the real morph risk)**

- **Cursor/Anysphere** — **the biggest threat.** $29.3B valuation; **acquired Graphite (Dec 2025) "way over" $290M** to fuse code-gen + review ([techcrunch](https://techcrunch.com/2025/12/19/cursor-continues-acquisition-spree-with-graphite-deal/), [axios](https://www.axios.com/pro/enterprise-software-deals/2025/12/19/)). Graphite = stacked PRs, 100k users, 500 cos (Shopify/Snowflake/Figma) + AI review. Cursor now owns editor → review and can extend to a forge. *Threat: VERY HIGH.*
- **CodeRabbit** — $60M Series B, $550M val, 2M repos, 13M PRs, 8k paying customers; most-installed AI review app ([siliconangle](https://siliconangle.com/2025/09/23/)). Review layer, GitHub-dependent. *Threat: MED (could add hosting).*
- **Greptile** — $25M, 2k customers (Brex, Substack), $30/dev. *Threat: MED.*
- **CI runners — Depot / Namespace / Blacksmith / WarpBuild / RunsOn** — drop-in GH Actions replacements, ~$0.004/min, cache+Docker-layer focus ([blacksmith.sh](https://www.blacksmith.sh/), [warpbuild](https://www.warpbuild.com/blog/)). **This is the lane CoreLink already occupies** — closest economic neighbors; best-positioned to morph upward via cache+runners. *Threat as competitor: LOW; as template/peer: HIGH.*
- **Merge queues — Aviator / Mergify / Trunk / Gitar** — Mergify $21/seat, Aviator speculative-parallel; Gitar leads CI auto-fix. AI-PR data: agent PRs wait **4.6x longer**, **32.7% accept vs 84.4% human** ([codeant](https://www.codeant.ai/blogs/top-pull-request-automation-tools)) — proves agent-fleet review/merge is broken today. *Threat: LOW; signal: HIGH.*
- **Sandboxes — E2B ($35M), Daytona ($24M), Modal** — sub-90ms agent sandboxes ([modal.com](https://modal.com/blog/top-code-agent-sandbox-products)). Maps to CoreLink "workspace snapshots." *Peer, not forge competitor.*

## Is anyone already building 'the agent-native forge'? (verdict + evidence)

**Verdict: Partially — no one owns the full stack yet, and the window is closing fast.** GitHub Agent HQ is the only true agent-fleet *orchestration* surface, but it's an expensive walled garden on a billing backlash. **Pierre** is the only independent forge with the exact "small AI-augmented teams + GitHub mirror" thesis, but it's human-team-first, not fleet-first. **Cursor+Graphite** has editor+review and the capital to build the rest — the most credible 12-month forge entrant. GitButler is rebuilding git semantics for agents but stays a client on GitHub. **Nobody combines: agent-fleet-native forge + git-protocol compat + bidirectional GitHub mirror + owned cache/CI/snapshot economics.** That precise intersection is open.

## The bridge/interop requirement (what minimum GitHub compat buys adoption)

Evidence is unambiguous: jj/Sapling/Graphite/GitButler/Pierre **all win by living on/beside GitHub**, never by replacing it ([jj git-compat](https://docs.jj-vcs.dev/latest/git-compatibility/), [docs.pierre.co](https://docs.pierre.co/integrations/github)). Minimum bar to get tried:

- **Standard git protocol** (clone/push/pull unchanged) — table stakes; jj proves teammates don't notice.
- **One-way GitHub mirror minimum**; Pierre ships auto-mirror as its trust unlock.
- **Bidirectional is viable but bounded:** GitLab/Harness do it in prod, but watch the **5,000 req/hr API limit, ~300-files/commit ceiling, and push-race conditions** — use a GitHub App (not PAT) and webhook-driven sync ([gitlab docs](https://docs.gitlab.com/user/project/repository/mirror/bidirectional/), [harness](https://developer.harness.io/kb/continuous-delivery/articles/biderectional-sync-prevent-github-api-limit/)). Realistic target: forge-authoritative with write-back PR/branch mirroring, not naive symmetric replication.

## Stack consolidation economics (what agent-heavy teams pay today, per dev/mo)

- GitHub Team/Enterprise seat: **$4–$21**
- Copilot (now usage-based): Pro $10 / Pro+ $39 / Max **$100**; **agentic sessions running 10x–50x → effectively $100–$500+/dev** ([copilot plans](https://github.com/features/copilot/plans), [techtimes](https://www.techtimes.com/articles/317536/20260601/))
- GitHub Actions: $0.008–0.016/min + new **$0.002/min platform fee**; **Copilot review now also burns Actions minutes** ([resources.github.com](https://resources.github.com/actions/2026-pricing-changes-for-github-actions/))
- 3rd-party runners (Depot/Blacksmith): **~$0.004/min** (cuts CI 30–50%)
- Graphite/stacked-PR: ~$20–30; CodeRabbit/Greptile review: **$24–30**; Mergify queue: **$21**; sandboxes metered
- **All-in agent-heavy dev today: easily $200–$600/dev/mo across GitHub + Copilot agents + runners + Graphite + CodeRabbit + sandbox.** Consolidation prize: one bill, cache-driven COGS. (CoreLink benchmark: $30/mo solo, ~$5 COGS, ~80% margin.) AI-code-tools TAM **$9.5B (2026) → $22B (2030)** ([researchandmarkets](https://www.researchandmarkets.com/reports/6225896/ai-code-tools-market-report)).

## Realistic wedge + sequencing recommendation

- **Wedge = "the cheapest, fastest place to run agent fleets," not "a better GitHub."** Lead with the cost wound: GitHub's 10x–50x agentic billing + Actions-minute review tax is the acute pain *right now*.
- **Sequence 1 — sell the economics you already have:** cache + ephemeral runners + workspace snapshots as the agent-fleet execution substrate, mirroring INTO GitHub (one-way first). Win on COGS/margin, zero migration ask.
- **Sequence 2 — own review/merge for agent PRs:** the broken metric (agent PRs 4.6x slower, 32.7% accept) is the highest-ROI forge surface; stacked-PR + auto-fix merge queue tuned for fleets. This is exactly what Cursor paid >$290M to bolt on.
- **Sequence 3 — promote to authoritative forge** only after bridge trust is proven; keep bidirectional GitHub write-back (GitHub App, webhook sync, ≤300-file commits).
- **Differentiate on multi-tenant content-addressed cache network effect** — the defensible moat vs. every runner/sandbox startup; shared public deps, isolated private.
- **Target Pierre's adjacent ICP but go fleet-first** (orchestrated agents as first-class identities/branches/worktrees), where Pierre is human-multiplayer-first.
- **Price as consolidation:** one bill replacing GitHub+Copilot-agents+runners+Graphite+CodeRabbit; anchor on the $200–600 they pay today.
- **Don't fight GitHub's editor/agent-model layer** — be the cheap execution+forge backend that any agent (Claude/Codex/Cognition) plugs into.

## Risks/threats

- **GitHub Agent HQ + Microsoft distribution** can make "good enough" fleet orchestration free-in-bundle and erase the wedge before we scale; their billing backlash may be temporary.
- **Cursor+Graphite** (>$290M, $29.3B parent) is the most likely to ship the integrated agent-native forge first and outspend a small company.
- **Bidirectional mirror is the technical Achilles' heel** — API rate limits, file-count ceilings, push races; a broken bridge kills trust instantly.
- **Pierre is already executing the same thesis** with $23.5M and real repo volume; first-mover on the "AI-team forge + GitHub mirror" story.
- **Single-vendor lock risk on Cloudflare** (Workers/R2/DO) for a forge workload; outages/limits hit the money path, and CF could ship competing primitives.
