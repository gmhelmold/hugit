# hugit — command catalog (v0.1)

> Working backwards, step 1.5: the product surface as commands. What's new,
> what's upgraded, what doesn't change at all (and why that's sacred), what
> changes completely, the tradeoffs, the safety model, and how a human always
> keeps the ability to follow.
>
> **Owner:** Gustavo Schneiter · **Drafted:** 2026-06-05 · status: REFINING
> Companion to `dream-product.md` (v0.3 — the five Inversions).

---

## 0. The operating model (who types what)

One repository. Three classes of principals, one chain of trust:

```
HUMAN STAKEHOLDER  (decides, approves, interrogates — never does ceremony)
      │  policy, budgets, verdicts on what policy marks human-mandatory
      ▼
ORCHESTRATOR AGENT (e.g. Claude Opus — the tech lead seat)
      │  owns campaigns: plans waves, declares claims, dispatches, lands
      ▼
AGENT SQUADS       (workers — one intent each, fenced by claims)
      campaign "checkout"   campaign "perf"   campaign "security"
      [agent][agent][agent] [agent][agent]    [agent]
                    └────────── same repo ──────────┘
```

- A **campaign** is a long-running stream of work inside the repo (e.g.
  `checkout`, `perf`, `sec-hardening`). Campaigns are the unit of *team*
  parallelism; intents are the unit of *work* parallelism.
- Concurrency between campaigns is arbitrated by **claims** (declared at
  dispatch) + the **landing queue** — not by humans deconflicting in Slack.
- Multiple orchestrators are allowed (one per campaign) under a root
  orchestrator; permissions are per-principal, enforced by policy.
- **The human can always follow** — every layer projects into a view the
  human reads without learning anything new (§8).

---

## 1. Namespace rules (is it organized? — yes, by three hard rules)

1. **`git` is never shadowed.** We do not wrap, alias, or modify any git verb.
   `git anything` behaves exactly as upstream git — forever. All new behavior
   lives under `hugit` (alias `hu`). One namespace to learn, zero to relearn.
2. **Refs are namespaced and auto-managed:** `refs/hugit/campaigns/<c>/<intent>`.
   Human-visible branch lists show campaigns and active intents only; landed
   and archived refs fold away (recoverable — CAS never forgets).
3. **Degradation invariant:** if every intelligent layer dies, what remains is
   a **valid, complete git repository** served over the standard wire
   protocol. The smart layer can fail; your repo cannot.

---

## 2. Catalog A — what does NOT change (and why that's sacred)

| Command | Status | Why unchanged |
|---|---|---|
| `git clone / fetch / pull / push` | **identical syntax & semantics** | wire-protocol compatibility is the adoption strategy |
| `git add / commit / status / diff / log / show` | identical | 20 years of human muscle memory |
| `git switch / checkout / branch / tag` | identical | every IDE, script and tool keeps working |
| `git stash / cherry-pick / revert` | identical | LLM training corpus: agents are deeply trained on git — deviation costs model accuracy (the naming principle, applied to UX) |
| plumbing (`rev-parse`, `cat-file`, …) | identical | the projection is *real git objects*, not an emulation |
| `.gitignore`, hooks, attributes | identical | ecosystem contracts stay intact |

**Why this matters more than it looks:** the unchanged set is not
conservatism — it is the moat of zero migration cost. A teammate (human or
agent) who never heard of hugit can clone, commit, push, and collaborate
without noticing anything — and everything they do is still captured by the
intent layer above them (their pushes become anonymous intents the
orchestrator can see in the ledger).

---

## 3. Catalog B — same command, upgraded engine (syntax unchanged, gains real)

| Command | What upgrades under the hood | Real gain (measured pain it kills) |
|---|---|---|
| `git push` | lands a snapshot; triggers shadow checks; "rejected, fetch first" effectively disappears (conflicts become objects, not walls) | the push-race / force-push class of loss → impossible (event-sourced refs) |
| `git merge` | three-tier resolution server-side: derived files **regenerate**, code merges **AST-aware**, remainder goes to gated LLM arbitration; markers only as last resort | lockfile conflicts (the #1 measured git pain) → extinct; 27.67% agent-PR conflict rate → write-time events |
| `git rebase` | safe by construction (nothing is ever lost; undo exists); `--regen` flag adds regenerative mode (§4) | rebase footguns + "dropped hours of work" → recoverable in one command |
| `git bisect` | memoized checks make each probe ~instant; usually unnecessary — the forge auto-bisects every regression | "when did this break?" → answered before you ask |
| `git blame` | works as-is; each line also links to its **Intent** (the why, not just the who) | archaeology → query |
| `git worktree` | works as-is; superseded by `hugit ws` (CAS-deduped, claim-fenced, <1s) | 28 GB / 256-worktree blowup → ~1× repo + deltas |
| `git log` | works as-is = **machine altitude**; `hugit log` shows the same history at **intent altitude** | 400 robot commits vs 14 intents — same data, two zooms |
| `git clone` | works as-is; `hugit clone` adds lazy materialization + warm semantic index | monorepo cold-clone pain → subtree hydration in seconds |

---

## 4. Catalog C — new commands (grouped by who types them)

### 4.1 The human stakeholder (follow · interrogate · decide — never ceremony)

| Command | What it does | Why / real gain |
|---|---|---|
| `hugit ledger [--live]` | the default human view: what was asked → done → proven, per campaign, risk-ranked | replaces reading 20 prose PRs/day with one narrated stream; the 98%-more-PRs review collapse → a 5-minute triage |
| `hugit review <intent>` | opens an **interrogation session** on a change: ask anything — *"where does this touch the money path?" "what changes for a logged-out user?" "convince me this is safe"* — answered from intent + context + evidence | human review at LLM pace becomes possible *and pleasant*; you review meaning, not lines (lines remain one drill-down away) |
| `hugit approve / reject <intent> [-m]` | issue a human verdict (policy decides which intents require one) | judgment is spent only where policy says it matters |
| `hugit watch` | TUI mission control: campaigns → intents → agents, live; conflict heat-map; landing queue; spend | "trackear, controlar e acompanhar" in one screen, terminal-native |
| `hugit why <file:line \| symbol \| intent>` | the founding intent + reasoning behind any line of the repo | tribal knowledge → durable, queryable memory |
| `hugit undo <operation>` | forge-level universal undo (landings, policy changes, ref moves) | the fear that justifies babysitting → reversibility |
| `hugit policy edit / test` | declarative gates (DCO, coverage, "auth needs a human", autonomy levels per risk class), locally testable | the bash-script gate museum → 30 lines of config, enforced identically everywhere |

### 4.2 The orchestrator (the tech-lead seat — plans, dispatches, lands)

| Command | What it does | Why / real gain |
|---|---|---|
| `hugit campaign new/list/status <name>` | create/inspect long-running work streams; squads attach to campaigns | many teams, one repo, zero Slack-deconfliction |
| `hugit plan apply <plan.yaml>` | declare a wave: intents, DAG, claims, acceptance criteria. **Claim intersections are checked NOW** | conflicts surface at dispatch — before any work, instead of 3h later in a 540-line merge |
| `hugit dispatch <intent>` | materialize a claim-fenced workspace + forge-built **context packet**; returns one URL to hand the agent | kills the 30–50% cold-start context burn; dispatching = one URL |
| `hugit fleet` | live machine-readable status of every agent/intent/workspace | the orchestrator's situational awareness, as data |
| `hugit land [<intent>…]` | enter the landing queue: speculative **union testing**, regenerative rebase, dependency-ordered landing, main always green | A+B-red caught pre-merge; reconciliation rounds → extinct |
| `hugit verdict request <intent> --lens security,correctness` | fan out independent reviewer agents with surgical packets; collect structured verdicts | LLM review precision: from 24–46% (raw-diff reviewers) to claim-verification with served context |
| `hugit tournament <intent> -n 3` | run N competing implementations; judge panel picks; losers stay addressable | exploration becomes a verb, not a mess |

### 4.3 The worker agent (executes one intent inside its fence)

| Command | What it does | Why / real gain |
|---|---|---|
| `hugit ctx snap / resume / diff / audit` | **version the context window**: snapshot understanding; resume someone else's; diff two minds; audit what info produced a change | the Inversion-2 superpowers; no agent ever starts cold; reconciliation dies at the root |
| `hugit map "<query>"` | semantic index queries: who-calls, contracts, conventions, altitude reads (`--alt api\|contract\|sig\|src`) | reading at the right zoom instead of grepping blind |
| `hugit impact [<change>]` | blast radius from the build/call graph before editing | "will this break something?" answered in ms, pre-edit |
| `hugit status` | **ambient truth**: live green/red of your claims (shadow checks), conflicts, landing position — the semantic counterpart of `git status` | the minutes-of-darkness between edit and verdict → a live signal |
| `hugit diag <failure>` | the failure as a diagnosis: culprit, diff-vs-green, suspect lines, similar past fixes | 4,000-line prose logs → structured causality; ~100× cheaper to act on |
| `hugit check [--local]` | run checks as the same pure function the forge runs (byte-identical) | push-and-pray YAML → locally replayable truth |
| `hugit intent seal` | declare done: evidence bundle assembled, acceptance verified, verdicts requested, enters landing — **this is the entire "ceremony"** | commit messages, branch names, PR opening, PR description: all emitted, none performed |
| `hugit journal note "<insight>"` | append to the session journal (auto-captured too) | understanding outlives the session |

### 4.4 Everyone

| Command | What it does |
|---|---|
| `hugit ask "<question>" [--scope intent\|campaign\|repo]` | interrogate anything — the repo, a change, a campaign — answered from index + intents + evidence |
| `hugit log` | history at intent altitude (the human-readable past); `git log` remains the machine altitude |
| `hugit ws spawn/attach/snap/gc` | workspaces: born <1s, claim-fenced, deduped, resumable anywhere |

---

## 5. Catalog D — what changes COMPLETELY (the absorbed ceremony)

These stop being *acts a worker performs* and become *effects the system emits*:

| Today's act | In hugit | Why it's safe to absorb |
|---|---|---|
| writing commit messages | emitted from the intent (charter + trajectory) | the intent is richer than any hand-written message; `git log` readers see generated, consistent messages |
| naming branches | auto: `campaigns/<c>/<intent-slug>` | humans browse campaigns, not branch soup |
| opening a PR + writing its description | `hugit intent seal` — the intent IS the PR, born with charter/evidence/verdicts attached | nothing to forget, nothing to embellish; reviewers get truth, not marketing |
| `git rebase -i` / squash to "clean history" | **history is never rewritten — it is re-projected.** Altitude folding shows 14 intents or 400 commits from the same immutable data | the single biggest git footgun (history rewriting) is eliminated, not improved |
| hand-editing conflict markers | three-tier server-side resolution; markers only by explicit request | conflicts became objects with owners, not 11 PM emergencies |
| writing CI YAML | checks are code, locally replayable | the "no sensible way to test CI locally" pain → gone |
| dependabot PR floods | dep bumps = speculative pre-tested landings by policy | 200 PRs/week → silent green landings + one task when red |

---

## 6. Tradeoffs (honest — each one named, priced, mitigated)

| Tradeoff | Cost | Mitigation / why we accept it |
|---|---|---|
| **Two mental models coexist** (git view + intent view) | humans/agents must know which altitude they're reading | defaults are role-correct (humans→ledger, agents→API, git-only users→git); both views project from ONE immutable store, so they can never disagree |
| **Regenerative rebase can produce code that differs from what was reviewed** | trust risk on the landing path | acceptance suite must re-pass; diff-beyond-threshold triggers re-verdict; ships **opt-in → default** as confidence data accumulates; every regen is itself an auditable intent |
| **Context capture is sensitive** (prompts may contain secrets/PII) | privacy/storage liability | tenant-private always; policy-based redaction; retention policy; **never** cross-tenant; capture level configurable per repo |
| **Everything-kept-forever storage growth** | COGS | CAS dedup + tiered eviction with pins; context/journals are tiny next to build artifacts; the value asymmetry is enormous |
| **The forge enters the write path** (shadow checks, claims) | new failure domain | the degradation invariant (§1.3): smart layer down → plain fast git keeps working; claims enforcement degrades to advisory + post-hoc audit, never to data loss |
| **New commands = learning curve** | onboarding cost | the curve is role-shaped: a human can operate forever with `ledger / review / approve / watch / why`; a git purist can operate with zero new commands and still be tracked |

**Is it safe? — the model in five locks:** (1) claims as *physical* fences
(capability-scoped workspaces, secrets broker — the credential never enters);
(2) event-sourced everything → universal undo, nothing ever lost; (3) history
immutable — re-projected, never rewritten; (4) policy gates fail-closed; (5)
the degradation invariant: worst case is a healthy git repo.

---

## 7. What stays the same — the one-line summary

**The repository, as a human knows it, stays the same: files, directories,
git commands, IDE integrations, the GitHub mirror.** What changes is who
carries the ceremony (the system, not the worker) and what gets remembered
(intent + context + proof, not just text).

---

## 8. Human visual navigation (o humano sempre acompanha)

The human never loses the thread, at any altitude:

1. **The file tree is just a file tree.** Browsing code — locally, in the web
   UI, or on the GitHub mirror — is exactly what it is today. No new concepts
   stand between a human and a file.
2. **History defaults to the intent ledger** (what was asked → done → proven,
   grouped by campaign), with a toggle to raw commits. Same data, two zooms —
   like switching between a map and satellite view.
3. **Live work is a dashboard, not a branch list:** campaigns → intents →
   agents, each with progress, risk, and cost. `hugit watch` in the terminal;
   Mission Control on the web; both read the same event stream.
4. **Every entity deep-links:** ledger entry → trajectory → semantic diff →
   raw lines → the exact context the agent had. Four clicks from "what
   happened today" to "the byte that changed", with meaning preserved at
   every step.
5. **The attention queue is the inbox:** policy decides what needs a human;
   everything else is narration you *can* read, never homework you *must*.
6. **The escape hatch is permanent:** the GitHub mirror renders the whole
   repo in the most familiar UI on earth. A stakeholder who refuses to learn
   anything new loses live narration — but never loses the ability to see
   the code and its history.

---

## 9. A 60-second example (the whole model in one session)

```text
# ── human ──────────────────────────────────────────────
$ hugit ledger --live
  ▸ campaign checkout   6/14 intents landed   2 in review   0 conflicts
  ▸ campaign perf       3/3 landed            cache hit 97%
  ⚠ attention (1): intent#11 claims auth contract → your verdict required

$ hugit review 11
  > where does this touch token validation?
  …answers from intent + context + evidence, with deep links…
$ hugit approve 11 -m "scope ok, evidence solid"

# ── orchestrator (Opus) ───────────────────────────────
$ hugit plan apply wave-2.plan        # 8 intents, claims checked NOW
  ✗ intent#17 claim overlaps #14 (payments contract) → re-slice before work
$ hugit dispatch 15..22               # 8 workspaces + context packets, <1s each
$ hugit land --queue                  # union-tested, regen-rebased, ordered

# ── worker agent ──────────────────────────────────────
$ hugit ctx resume intent#15          # inherits prior understanding
$ hugit impact src/checkout/tier.rs   # blast radius before editing
$ hugit status                        # ambient truth: 14/14 shadow checks green
$ hugit intent seal                   # evidence + verdicts + landing: done
```

Three principals, one repo, zero ceremony, full human visibility.
