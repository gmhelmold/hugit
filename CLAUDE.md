# CLAUDE.md

Context for AI agents working in this repo. Keep it lean + high-signal.

## What hugit is

The **git-compatible, LLM-native forge** — CoreLink expansion campaign #3.
VCS + merge + CI designed for orchestrated agent fleets, built on CoreLink's
production CAS (Cloudflare Workers/R2/D1/DO). Founded 2026-06-05.

**Status (2026-06-08): the buildable product is complete.** A 14-crate Rust
workspace implements all 67 work-packages of decomposition v2.0 (E6 superseded
by the forge-arbitrated bidirectional-sync design); `main` is green by gate
(fmt + clippy `--workspace --all-targets --locked -D warnings` + test
`--workspace --locked` + audit). What remains is **owner-gated infra, not
code**: provisioning the CoreLink prod tenant (P2 — see
`docs/handoff/2026-06-08-corelink-p2-tenant-request.md`) flips the disclosed
live-infra seams (AC HTTP, runner box, transparency log, live GitHub detect)
from hermetic-proof → end-to-end.

Read first: `docs/whitepaper/hugit-v1.md` (product design) ·
`docs/plan/decomposition.md` + `docs/plan/wp-contracts/` (the 67-WP register) ·
`docs/review/2026-06-07-roadmap-gap-build-campaign.md` (what's built) ·
`docs/strategy/campaign-3-llm-native-forge.md` (founding brief) · `docs/research/`.

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

## The session fence (owner mandate 2026-06-05 — MECHANIZED)

It must be **impossible** for hugit work to cross other sessions' repos,
especially corelink-server. Enforcement is physical, not behavioral:

1. **`.claude/settings.json`** (this repo) carries `permissions.deny` rules
   AND a `PreToolUse` hook (`.claude/hooks/forbid-sibling-paths.py`) that
   **blocks every Edit/Write/NotebookEdit into a sibling HuGR project and
   every Bash command referencing one unless it is provably read-only**
   (fail-closed). Every session opened in this directory — and every
   subagent it spawns — inherits the fence automatically.
2. **Open hugit sessions IN `~/Documents/HuGR/hugit`** — never from a
   sibling project's directory (a session anchored elsewhere does not load
   this fence). The founding session was corelink-anchored by historical
   accident; do not repeat it.
3. The TechLead profile (`.techlead/profile`) mirrors the same `neverTouch`
   list for fleet dispatch.
4. Read-only inspection of siblings (cat/grep/git log) is allowed — context
   is fine, mutation never is. Fence changes require explicit owner approval.

## Conventions

- Commits: `Signed-off-by:` trailer (DCO) + `Co-Authored-By: Claude …` trailer.
- English for all repo documents; lean, evidence-cited strategy docs (house
  style mirrors `corelink-server/marketing/expansion/`).
- Once code exists: branch → PR → merge, gates green before merge (inherit the
  CoreLink discipline). Until then, docs may land on `main`.

## Don't touch

Other projects share the parent dir (`corelink-server`, `hugr-wallet`,
`HuGR-Smith`, `HuGR-Arsenal`, `_worktrees/`, etc.). **Only work on hugit here.**
