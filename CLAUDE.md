# CLAUDE.md

Context for AI agents working in this repo. Keep it lean + high-signal.

## What hugit is

The **git-compatible, LLM-native forge** — CoreLink expansion campaign #3.
VCS + merge + CI designed for orchestrated agent fleets, built on CoreLink's
production CAS (Cloudflare Workers/R2/D1/DO). Founded 2026-06-05; currently in
**strategy/research phase** — no product code yet.

Read first: `docs/strategy/campaign-3-llm-native-forge.md` (the founding brief)
and `docs/research/` (the evidence base).

## Principles (decided, don't relitigate without the owner)

- **Don't deviate from git.** Names, CLI shape, mental model stay git-proximate.
  Every deviation costs human adoption AND LLM affinity. (This is why the
  product is "hugit", not a fantasy name.)
- **Embrace, don't assault.** Compat ladder: git wire protocol → landing layer
  riding ON GitHub → bounded bidirectional mirror → authoritative forge. A
  broken bridge kills trust instantly; never naive symmetric sync.
- **The wedge is the landing problem** (integration/merge for agent fleets),
  not authoring, not review prose.
- **Memoize by content, price flat.** Never usage-billing whiplash; never
  charge for the customer's own compute.
- **Zero debt, no loose ends, impeccable repo** (same owner mandate as
  CoreLink). Verify claims; never loosen rigor without an explicit waiver.

## Relationship to CoreLink (`../corelink-server`)

Same primitive stack, nothing built twice: cache (launch) → compute (#1
runners) → workspace (#2 snapshots) → **forge (#3, here)**. The CAS, AC,
manifests, tenancy, and PAT auth live in corelink-server — hugit consumes them,
it does not fork them. **Do not let hugit work leak into CoreLink's launch
route or campaign #1/#2 critical paths.**

⚠️ corelink-server frequently has **other live sessions/worktrees** working in
it. Never assume sole ownership of its checkout; check `git worktree list` and
uncommitted state before touching anything there.

## Conventions

- Commits: `Signed-off-by:` trailer (DCO) + `Co-Authored-By: Claude …` trailer.
- English for all repo documents; lean, evidence-cited strategy docs (house
  style mirrors `corelink-server/marketing/expansion/`).
- Once code exists: branch → PR → merge, gates green before merge (inherit the
  CoreLink discipline). Until then, docs may land on `main`.

## Don't touch

Other projects share the parent dir (`corelink-server`, `hugr-wallet`,
`HuGR-Smith`, `HuGR-Arsenal`, `_worktrees/`, etc.). **Only work on hugit here.**
