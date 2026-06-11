# CLAUDE.md

Context for AI agents working in this repo. Keep it lean + high-signal.

## What hugit is

The **git-compatible, LLM-native forge** — CoreLink expansion campaign #3.
VCS + merge + CI designed for orchestrated agent fleets, built on CoreLink's
production CAS (Cloudflare Workers/R2/D1/DO). Founded 2026-06-05.

**Status (as of 2026-06-11, post Waves A/B/C/D/E + adversarial rounds 1–2,
Wave F in progress, Round 3 pending): the buildable product is
complete; adversarial hardening is ongoing, not closed.** A **17-package** Rust
workspace (hugit-app + {ui,exit,sidecar} sub-crates = 4 crates + 13 feature
crates — verified by `cargo metadata --no-deps` 2026-06-11; `hugit-web`
MIGRATED OUT 2026-06-10 to ../githugr per the headless-engine doctrine, and
`hugit-runner` TRANSFERRED 2026-06-10 to ../corelink-runners per the
runner-transfer campaign: hugit is git+forge, compute is campaign #1's product;
the seam is the wire contract — shared `conformance/` vectors byte-identical in
both repos, no git dependency in either direction) implements all 67
work-packages of decomposition v2.0 (E6 superseded by the forge-arbitrated
bidirectional-sync design). Adversarial audit arc: Round 1 (fresh 7-agent fleet,
after Wave D) found **7/7 DO-NOT-SHIP**; Wave E remediated all seven. Round 2
(fresh 7-agent fleet, after Wave E) found **7/7 DO-NOT-SHIP again** (narrower —
structural spine held; 1 CRITICAL + consistency + doc/CI debt). Wave F is
remediating Round 2. Round 3 (fresh fleet) will follow. `main` is green by
local gate (fmt + clippy `--workspace --all-targets --locked -D warnings` + test
`--workspace --locked` + deny + audit); remote CI gate passes when it runs to
completion, but the single self-hosted runner is contention-flaky (~63% of
recent runs fail on SIGTERM/exit-127 infra failures, not code — HEAD may show
`in_progress` or a false failure on CI). What remains to flip to end-to-end:
**owner-gated infra** (P2 CoreLink tenant provisioning — see
`docs/handoff/2026-06-08-corelink-p2-tenant-request.md`) + Wave F critical
fixes (WF-REDACT, WF-CLI, WF-AUTHZ). The disclosed live-infra seams (AC HTTP,
runner box, transparency log, live GitHub detect) remain hermetic-proof until
P2.

Read first: `docs/whitepaper/hugit-v1.md` (product design) ·
`docs/product/product.md` (the product brief: ICPs, killers, positioning, pricing posture) ·
`docs/adr/` (0001 context envelope · 0002 HuGR identity) ·
`docs/interop.md` (the microscopic seam map: AC/CAS · runners · GitHub · githugr) ·
`docs/plan/decomposition.md` + `docs/plan/wp-contracts/` (the 67-WP register) ·
`docs/review/2026-06-07-roadmap-gap-build-campaign.md` (what's built) ·
`docs/strategy/campaign-3-llm-native-forge.md` (founding brief) · `docs/research/` ·
`docs/handoff/` (pending cross-repo work: P2 provisioning · identity rollout).

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
- **State the family in the correct tense.** Production-state claims about a
  sibling cite that repo at the time of writing (GA notes, runbooks) — never
  memory of a design. Cautionary tale: the cross-tenant-dedup overclaim
  (`../corelink-runners/docs/review/2026-06-09-cross-tenant-dedup-claim.md`).
- **Identity is decided (ADR-0002):** one **HuGR account** on CoreLink
  machinery (Clerk · org = tenant · PATs) behind a frozen contract; **a PAT
  never reaches a browser**. No new auth service without a forcing function.

## Relationship to the HuGR family

Same primitive stack, nothing built twice:
**HuGR → CoreLink { Cache (launch) · Runners (#1) · Workspaces (#2) } →
hugit (#3, here) → githugr (#4, the forge surface)**. The CAS, AC, manifests,
tenancy, and PAT auth live in corelink-server — hugit consumes them, it does
not fork them. **Do not let hugit work leak into CoreLink's launch route or
campaign #1/#2 critical paths.**

Two incubation repos are managed FROM hugit sessions under owner-approved
fence carve-outs: `../githugr` (campaign #4) and `../corelink-runners`
(campaign #1 — its `docs/spec/hugit-integration-contract.md` is frozen from
hugit's side; **amended to v1.1** 2026-06-10, WP-R6: §13 adds per-job
metrics emission + transcript capture hook obligations; the frozen v1.0
§0–§12 are unchanged).

⚠️ Sibling repos — corelink-server especially, but **also the carve-outs** —
have **other live sessions/worktrees**. Never assume sole ownership; check
status/log before acting. **Never `git commit --amend` or rewrite history in a
sibling**: another session's commit may have become HEAD between your commit
and your amend (it happened 2026-06-09; recovered via atomic ref
compare-and-swap). Fixup commits only; even in hugit, re-check `git log -1` is
yours immediately before any amend.

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
3. The session fence (`.claude/settings.json`) is authoritative; the TechLead
   profile (`.techlead/profile`) lists a subset of the same `neverTouch` paths
   for fleet dispatch.
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
