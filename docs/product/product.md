# hugit — Product Design

> Status: product brief, 2026-06-09. The canonical product design is
> `docs/whitepaper/hugit-v1.md` (it wins on vision/principle); this brief is
> the family-format ICP/positioning/pricing view (mirrors
> `../corelink-runners/docs/product/product.md` and
> `../githugr/docs/product/product.md`). Deeper appendices:
> `docs/product/dream-product.md` (the five Inversions) ·
> `docs/product/command-catalog.md` (the verb surface + safety locks) ·
> `docs/strategy/absorption-map.md` (the war plan). Numbers are sourced in
> `docs/research/` or marked **indicative — owner ratifies before anything
> goes public**.

---

## 1. The value couplet

**Your agent fleet ships branches that are green alone and red together. hugit
lands them on a `main` that is always green and re-runs zero CI it has already
paid for — on your existing GitHub repos, migrating nothing.**

That is the one thing hugit is for. Everything else below is how it delivers
that promise and what comes next.

---

## 2. The bet (why this exists now)

1. **The landing problem is the new bottleneck, measured.** 27.67% of agentic
   PRs hit merge conflicts (avg 540 conflicting lines); agent PRs wait 4.6×
   longer and land at 32.7% acceptance vs 84.4% human; AI-heavy teams ship 98%
   more PRs, 154% larger, with 91% longer reviews. "A green + B green ≠ A+B
   green" — and every orchestrator stops at the merge boundary.
   (`docs/research/`, 4-lane sweep.)
2. **The economics are structural, not features.** GitHub's revenue model
   bills the waste (per-minute CI, usage-billed AI); CoreLink's margin model
   deletes it (memoize by content, dedupe, zero-egress R2). The incumbent
   cannot follow without billing itself out of its own P&L.
3. **The substrate already exists.** CoreLink's CAS + Action Cache are live in
   production; `clw` (workspaces) shipped phase 1; the runner fabric is spec'd
   with hugit as anchor tenant. hugit is layer 4 of a platform, not a
   greenfield stack — which is why 67 work-packages went from zero to built,
   hermetically proven, in one campaign.

---

## 3. Who it's for (ICPs) & user stories

### Beachhead ICP — Fleet operators (the wedge)

The fleet operator is an engineer or orchestrator running 10–50 agents in
parallel. They are the primary ICP. Every other persona is expansion from here.

- *"My orchestrator runs 10–50 agents. Each branch is green alone and red
  together. I spend my evenings reconciling work that machines produced in
  minutes."* → the union-testing landing queue: batches tested together
  **before** landing; minimal failing pair isolated by ~free bisection; the
  rest of the batch proceeds.
- *"Every push re-runs a CI suite that already ran on 95% of this tree."* →
  `check(tree ‖ def ‖ toolchain)` memoized in the AC: a hit is a lookup, zero
  execution.
- hugit itself is its own design partner: we run agent fleets daily and our own
  landing pain is the spec. Real hit-rates and cost decomposition are published
  — not claimed.

### Expansion ICP-B — Platform / CI leads on GitHub (the first dollar)
- *"I'm not migrating my repos. Give me the wins on the repo I already have."*
  → the landing layer rides GitHub via the App: memoized checks + the union
  queue + verdicts, zero migration ask; the mirror is the permanent escape
  hatch.

### Expansion ICP-C — Engineering leadership / finance (the buyer)
- *"Agent tooling costs me $200–600/dev/mo across five vendors and spikes when
  the team ships."* → one flat, predictable bill (the consolidation prize);
  never metered on the customer's own compute.

### Expansion ICP-D — jj users
- *"jj has change-ids and no native forge."* → hugit serves git AND jj over
  the same CAS (phase D); stable intent identity ≅ change-id.

---

## 4. The product surface

Status key: **LIVE** = serving real data today · **BUILT** = gate-green, deploy-gated · **ROADMAP** = logic designed/hermetic, not yet executing live.

| Capability | Status | What the customer gets | Why it's ours to win |
|---|---|---|---|
| **Union-tested landing queue** | **ROADMAP** | batches of green PRs tested **together** pre-land; `main` always green; failing pair excluded, rest proceeds | proven blueprint (Uber SubmitQueue) + two upgrades: advisory claims + memoized verification — no forge ships it |
| **Memoized checks** | **ROADMAP** | cache-hit ⇒ zero execution; honest partial hit-rates, measured never promised | requires a production CAS/AC — years of substrate, already live |
| **Derived-file regeneration** | **ROADMAP** | lockfiles/codegen **never text-merged** — regenerated deterministically | kills the #1 measured git pain |
| **Intent + context envelope** | **LIVE** (write path) | every commit carries charter, trajectory (3 altitudes), metrics, verdicts | the durable record of ephemeral authors |
| **Claims as fences** | **LIVE** (hermetic) | a workspace materializes ONLY what the intent claimed; blast radius = the claim | safety by construction; the union test is the runtime oracle |
| **Auto-bisect + flake intelligence** | **LIVE** (`hugit diag`) | red → culprit in ≤log₂ probes over memoized checks; flakes quarantined, annotated | bisect is ~free only when checks are memoized |
| **Event-sourced refs + universal undo** | **LIVE** (`hugit undo`) | force-push data loss is **unexpressible**; every op reversible | agents do dumb things; undo is the trust feature |
| **Symbol outline** | **LIVE** (`hugit symbol`) | structured symbol map of any source file — TS/JS/Python/Go/Java/C/C++/Ruby | standalone; no server; the "first aha" |
| **Export / exit guarantee** | **LIVE** (`hugit export`) | full git + JSON snapshot, redaction applied, exit-proof | the exit guarantee is also the DR plan |
| **Import** | **LIVE** (`hugit import`) | bring a GitHub repo without leaving GitHub | zero migration ask; reversible at every rung |
| **Provenance / attestation** | **LIVE** (object model) | SLSA-class chain **including the model layer**: which model, whose instruction, what cost | falls out of the object model; GitHub cannot express it |
| **Bidirectional GitHub mirror** | **ROADMAP** | branches round-trip; `main` single-writer via the queue; incidents preserved as refs, never dropped | a broken bridge kills trust — ours is forge-arbitrated by design |
| **Actions shim** | **ROADMAP** | imported repos keep `.github/workflows` running (supported subset, explicit) | absorption discipline: nothing absorbed worse |
| **`git clone` / `git fetch`** | **BUILT, deploy-gated** | smart-HTTP upload-pack; `git clone` succeeds in CI | live once `HUGIT_SERVE_GIT_DIR` is set on deploy |
| **`git push`** | **LIVE (caveated)** | receive-pack succeeds on prod (#198, git-free unpack); a pushed ref serves post-reboot, clone-back gated on the public-flag | now |

## 5. The killer features, ranked

Status key: **LIVE** = working today · **ROADMAP** = designed, not yet executing live.

1. **Union testing at fleet scale** *(ROADMAP)* — the wedge; the landing problem solved
   empirically, not predictively. Algorithm built + hermetically proven; needs live runner fabric.
2. **Memoized checks** *(ROADMAP)* — CI that mostly never runs; the margin and the speed
   are the same number. Needs live runner + Action Cache substrate.
3. **Derived-file regeneration** *(ROADMAP)* — highest pain-to-effort ratio in the
   portfolio; lockfiles/codegen regenerated, never text-merged.
4. **The context envelope + three-altitude metrics** *(LIVE — write path)* — fleet
   legibility; every commit carries charter, model, cost, verdicts.
5. **Auto-bisect culprit-finding** *(LIVE — `hugit diag`)* — log-backed bisect; a default, not a luxury.
6. **Universal undo** *(LIVE — `hugit undo`)* — nothing is ever lost, by construction.
7. **Symbol outline** *(LIVE — `hugit symbol`)* — structured symbol map; the "first aha"; standalone, no server.
8. **Export / exit guarantee** *(LIVE — `hugit export`)* — also the DR plan; zero lock-in.
9. *(ROADMAP)* **Regenerative rebase** — opt-in forever for non-trivial intents;
   adversarial re-verdict mandatory (review-panel demotion stands).

## 6. Deliberately NOT the product

Authoring tools (the editor war is crowded) · review-prose AI (CodeRabbit
et al.) · a social network (stars/sponsors stay on GitHub — drained, not
stormed) · per-minute compute resale (that's the meter we're killing) ·
a fantasy-named VCS (git names, git CLI shape, git mental model — forever).

---

## 7. Pricing posture (doctrine decided; numbers are the owner's)

- **Flat per unit-that-scales** (orchestrator seats · parallel runners · warm
  workspaces · pinned storage). **Never** meter the customer's own compute;
  never usage whiplash. Expansion comes from fleet growth.
- **The consolidation prize:** one bill replacing GitHub + Copilot-agents +
  runners + stacked-PR + AI-review (today $200–600/dev/mo, spiky).
- **Open (owner):** the fair-use boundary for flat plans — a 50-agent tenant
  firing speculative union tests has real COGS; the cap must be stated
  **before** the first customer, or flat quietly becomes metering with extra
  steps (review-panel finding, still unresolved).
- COGS physics inherited from CoreLink: zero egress, global dedup
  (intra-tenant at GA; cross-tenant staged — `CAP-DEDUP-CROSS-TENANT`),
  memoized verification, cache-warm ephemeral compute.

---

## 8. Positioning

> **GitHub bills the waste. hugit deletes it — and lands your fleet's work on
> a `main` that never breaks.**

| Against | Their model | Our edge |
|---|---|---|
| **GitHub / Agent HQ** | agents bolted onto human-pace primitives; per-minute CI; waste IS revenue | structural economics + the landing oracle; and we ride their hosting, so trying us risks nothing |
| **Pierre** | prettier git platform, human-team-first, GitHub-mirrored | same bridge, different war: fleet landing + memoized CI + provenance aren't in their model |
| **Cursor + Graphite** | editor→review polish; stacked PRs | review ergonomics on the same blind data model; no landing oracle, no cost model |
| **GitButler** | client-side git semantics for agents | stays a client on GitHub; we own the landing + the cache economics |
| **Merge-queue vendors** (Aviator, Mergify, Trunk) | queue mechanics on GitHub Actions | they re-run everything; we union-test on memoized checks — the queue costs only its novelty |
| **Depot/Blacksmith-class CI** | faster minutes, still metered | we sell the end of the minute (via Runners), under a forge that needs no migration |

Defensibility: the substrate (CAS/AC at production scale + the tenancy/privacy
machinery) is years of work and already live; the landing layer's data
(hit-rates, verdicts, envelopes) compounds into the legibility product
(githugr) that a UI-copy cannot fake.

---

## 9. The route (full table: whitepaper §12)

| Phase | Ships | State |
|---|---|---|
| A | CoreLink launches; `clw` dogfood | cache live (GA staged); clw phase 1 shipped |
| B | **hugit Dev Kit**: claim-fenced workspaces + memoized checks + the landing layer riding GitHub | **built, hermetically proven**; lights live at **P2** (tenant + interim runner box) |
| C | runner fabric takes over execution; flake intel + auto-culprit at volume | Runners M0 spec'd; M1 replaces the interim transport |
| D | the forge proper: wire protocol over CAS, intents native, Ledger/Mission Control (githugr), jj first-class, one-command import | logic built; surface = githugr (design phase) |
| E | bidirectional mirror → authoritative hosting; absorption parity | design done (forge-arbitrated sync built hermetically) |

External dependencies, named: **P2** (`docs/handoff/2026-06-08-corelink-p2-tenant-request.md`
+ runbook) · **Runners M1** (`../corelink-runners/`) · the identity endpoint at
githugr Wave 4 (`docs/handoff/2026-06-09-hugr-identity-rollout.md` §A1).

---

## 10. Open decisions for the owner

1. **Pricing numbers** for the Dev Kit / forge tiers (doctrine fixed; tiers
   indicative until ratified).
2. **The fair-use boundary** for flat plans (§7) — decide before customer #1.
3. **Design-partner cohort shape** — hermeticity-friendly first (Bazel/Nix
   shops where memoization shines) vs typical-npm (where measured hit-rates
   must carry the pitch)? Recommendation: 2–3 of each, instrumented honestly.
4. **ADR-0001 §7 knobs** (capture level default `task` · 90-day raw-transcript
   TTL · `cost_usd` visible to all) — still awaiting ratification.
5. **Public dogfood ledger** (build-in-public with real cost decomposition) as
   the launch marketing motion — recommended; no competitor can fake it.