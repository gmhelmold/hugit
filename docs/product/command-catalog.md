# hugit — feature & command catalog (v2 — post-review, hardened)

> **v2 (2026-06-05):** rewritten after the 3-reviewer brutal panel
> (`docs/reviews/2026-06-05-brutal-review-panel.md`). The amputations are
> applied, every feature now carries its **status** and its **phase**, and the
> catalog is honest about what is core, what is opt-in, what is research-gated,
> and what was cut from v1 (kept in the vision, removed from the build).
> v1 of this file (the pre-review catalog) is preserved in git history.
>
> **Owner:** Gustavo Schneiter · status: REFINING

## Status legend

| Tag | Meaning |
|---|---|
| ✅ **CORE** | survived all three reviewers; load-bearing; build it |
| 🔧 **HARDENED** | survives in a modified, stricter form (the modification is binding) |
| 🎚️ **OPT-IN** | demoted: never the default; value must be proven per-repo |
| 🔬 **GATED** | blocked behind the killer experiment or real usage data |
| 🧊 **DEFERRED** | kept in the vision; cut from the build until its phase |
| ⛔ **CUT** | removed as load-bearing claim (may survive as a demo) |

## The two standing gates (written, dated, binding)

1. **The focus gate — REFORMED by owner decision (2026-06-05):** CoreLink and
   hugit run **in parallel with zero interference** ("um não interfere — AT
   ALL — no outro"). CoreLink's launch route, infra and sessions are
   untouched by hugit work; hugit dogfood excludes `corelink-server` during
   its launch window. The phase-B exit metric still gates *charging money*.
   Execution: warp plan, two phases per sprint (`docs/plan/warp-10-days.md`).
2. **The experiment gate (stands):** claims-as-oracle and regen-rebase are
   promoted only by data — the disjointness-rate + regen-honesty experiment
   on ~200 real fleet changes (`docs/reviews/…`, "the ONE experiment").

---

## Phase B — the Dev Kit: ONE GitHub App
### "Memoized CI for agent PRs — your green checks never re-run."

One install. One number (CI minutes/$ saved). Zero workflow religion. Rides
L0–L2 (CAS + AC + clw), which exist. Everything else in this catalog comes
later.

| Feature | Status | What it is (hardened form) |
|---|---|---|
| **Memoized checks** | ✅ CORE | `check(tree, def, toolchain)` cached in the AC — `clw run`, formalized (~80% extant). Honest scope: full power on hermetic/deterministic repos (Bazel/Nix/Rust/Go); for npm/pip repos, hit-rates are partial — measured and shown, never promised |
| **Union-testing landing queue** | ✅ CORE | batch landable PRs, run affected memoized checks on the **union** (A+B+C together), land green in order, report the minimal failing pair. **This is the conflict oracle** — discovery by speculation, not prediction |
| **Structured diagnoses** | ✅ CORE | a red check returns: culprit PR (auto-bisect over memoized checks ≈ free), diff-vs-last-green, suspect targets — data, not 4,000-line logs |
| **Auto-culprit on regression** | ✅ CORE | memoization makes bisect cheap → run it always, automatically |
| **Intent sidecar** | ✅ CORE | charter + acceptance criteria + context ref **attached to PRs as metadata** — non-authoritative, builds the corpus that later phases (and the experiment gate) need |
| **`hugit check --local`** | ✅ CORE | the same pure function locally and remote — byte-identical; kills push-and-pray |
| Claims at dispatch as conflict prevention | 🔬 GATED | **demoted from phase B entirely.** Conflicts are discovered at landing (union testing) like SubmitQueue proved. Claims return later, advisory-only, if the disjointness experiment justifies them |
| Regenerative rebase | ⛔ CUT from B | textual fallback only in phase B |

**Phase-B exit metric (binding, from the operator review):** 10 external
teams, 3 weeks of use, ≥40% week-3 retention, ≥3 unprompted "I'd pay for
this" — within 90 days of CoreLink's first 10 paying customers. Fail → the
forge thesis is re-examined before another dollar of effort.

---

## Phase C — the fabric (CoreLink runners live)

| Feature | Status | What it is (hardened form) |
|---|---|---|
| **Ephemeral cache-warm runners** | ✅ CORE | campaign #1, reused verbatim — checks + sandboxes off your machine forever |
| **Derived-file regeneration** | ✅ CORE | lockfiles/codegen/snapshots: declared derived, always regenerated, never text-merged. Deterministic — this is NOT the dangerous regen. Kills the #1 measured git pain |
| **Flake intelligence** | ✅ CORE | statistical flake detection + quarantine by policy — needs the fabric's execution volume, hence phase C |
| **Shadow checks** | 🔧 HARDENED | NOT "on every write" (cost/noise unbudgeted — engineer review). On workspace **snapshot cadence**, budget-capped per tenant, opt-in per repo. The ambient-truth dream, paid for honestly |
| **Claim-fenced workspaces (SECURITY)** | ✅ CORE | sparse materialization = physical reach limits; secrets broker (credentials never enter). Claims as **fences** survived review unanimously — it's claims as *conflict oracle* that didn't |
| **Workspace spawn/attach/resume** | ✅ CORE | `clw` + fencing; <1s, deduped, local/remote transparent |

---

## Phase D — the forge

| Feature | Status | What it is (hardened form) |
|---|---|---|
| **git wire protocol over the CAS** | ✅ CORE | the projection layer; degradation invariant binding (smart layer dies → healthy git). Budgeted honestly: pack negotiation is the hard part |
| **Intents native** | 🔧 HARDENED | the unit of work and provenance — framed as **first-class provenance OVER git**, not "code is the projection of intent" (the metaphysics stays in the pitch, out of the spec). Raw `git push` from compat users = recorded as opaque change-events, never reverse-engineered into fake intents |
| **The Ledger** | ✅ CORE | asked → done → proven, by campaign; the human's default history view; toggle to raw commits (two zooms, one store) |
| **The attention queue** | ✅ CORE | policy × blast-radius × verdict-confidence ranks what needs a human |
| **Adversarial verdict panels** | 🔧 HARDENED | review = **independent reviewer agents, different prompts/models, against served ground truth** (build-graph impact, contracts, evidence). The change **never defends itself** — evidence answers. Replaces v1's "interrogate the change" framing (sycophancy risk, red-team review) |
| **Assisted human review** | 🔧 HARDENED | the human asks questions; answers are **citations to evidence objects** (tests, verdicts, impact queries, diffs) — grounded retrieval, not generative persuasion. Approve-in-90s is permitted only where policy says low-risk |
| **`hugit why` / provenance** | ✅ CORE | every line → its intent → its charter/author/model/cost. The part of context-versioning that is durable |
| **Journals + short-horizon resume** | 🔧 HARDENED (narrowed) | session journals as objects; `ctx resume` for the crashed/replaced-agent case (minutes-to-days). The honest core of "gitted context" |
| Context replay as reproducible computation | ⛔ CUT | models deprecate quarterly; no bitwise determinism. Survives as best-effort debug tooling, never a guarantee |
| "Diff two minds" | ⛔ CUT as load-bearing | demo, not primitive |
| **Build-graph impact queries** | ✅ CORE | `hugit impact` — who-calls/blast-radius from the build graph (REAPI heritage). The affordable 80% of the semantic dream |
| Full semantic index (altitude reading, summaries) | 🧊 DEFERRED | 8–14 EM research-grade (engineer review); ships after the forge has revenue; build-graph queries carry the load until then |
| **Policy engine** | ✅ CORE | declarative gates, locally testable, fail-closed; autonomy levels per risk class |
| **Event-sourced refs + universal undo** | 🔧 HARDENED | DO-per-repo with **compaction/cold-tiering to R2 designed in from day one** (DO storage caps are real); "nothing lost" holds at the CAS level, the hot log is bounded |
| **jj first-class** | ✅ CORE | change-ids, stacked changes — the uncontested distribution door |
| **One-way GitHub mirror** | ✅ CORE | the trust unlock AND the durability/DR story for single-vendor risk — continuously verified, marketed as such |
| **GitHub import** | ✅ CORE | one command: history, issues, PRs |
| Regenerative rebase (non-trivial intents) | 🎚️ OPT-IN **forever** | per-repo, per-intent-class opt-in; acceptance must re-pass **plus an independent adversarial verdict on every regen** (no distance-threshold hand-wave — circular-verification risk is structural). Promotion to broader use only via the experiment gate |
| **Tournament intents** | ✅ CORE (cheap) | N competing implementations, judge panel — it's orchestration over existing primitives |

---

## Phase E — head-on (the absorption push)

| Feature | Status |
|---|---|
| Bidirectional mirror (forge-authoritative, bounded write-back) | 🔧 HARDENED — only after months on our own repos; GitHub App + idempotent webhook sync; never naive symmetric |
| Actions-YAML compat shim | ✅ CORE (migration lubricant) |
| Status/badge API compat | ✅ CORE |
| Packages/registry on the CAS | 🧊 DEFERRED until pulled |
| Mission Control web (full) | 🧊 DEFERRED — `hugit watch` TUI + ledger carry phase D |
| Issues→intents, boards→campaigns, wiki→knowledge | 🧊 DEFERRED per absorption map |
| Social layer | 🪞 ride the mirror; not our war |

---

## The command surface (v2 — per principal, with phase)

### Human (phase D unless noted)
| Command | Phase | Notes |
|---|---|---|
| `hugit ledger [--live]` | D | default history view |
| `hugit review <intent>` | D | 🔧 grounded-evidence answers; never self-defense |
| `hugit verdict approve / reject` | D | policy decides what reaches you |
| `hugit watch` | D | TUI; web Mission Control deferred |
| `hugit why <line\|symbol\|intent>` | D | provenance query |
| `hugit undo <op>` | D | forge-level, event-sourced |
| `hugit policy edit / test` | D | fail-closed, locally testable |
| *(phase B human surface)* | B | **the GitHub App dashboard + PR comments — no new CLI for humans at all** |

### Orchestrator
| Command | Phase | Notes |
|---|---|---|
| `hugit land [--queue]` | **B** | the union-testing queue (on GitHub PRs in B; native in D) |
| `hugit verdict request --lens …` | **B** (basic) / D (panels) | independent adversarial reviewers |
| `hugit campaign / plan apply` | D | 🔧 claims advisory-only; DAG + acceptance binding |
| `hugit dispatch <intent>` | D | workspace + context packet |
| `hugit fleet` | D | machine-readable fleet state |
| `hugit tournament -n N` | D | exploration as a verb |

### Worker agent
| Command | Phase | Notes |
|---|---|---|
| `hugit check [--local]` | **B** | memoized, byte-identical local/remote |
| `hugit diag <failure>` | **B** | structured diagnosis |
| `hugit impact <path\|change>` | C/D | build-graph blast radius |
| `hugit status` | C | 🔧 snapshot-cadence shadow checks (budgeted) |
| `hugit ws spawn/attach/snap/gc` | C | claim-fenced (security) |
| `hugit ctx snap / resume` | D | 🔧 short-horizon; journals first-class |
| `hugit intent seal` | D | the one ceremony verb |
| `hugit note` | D | understanding outlives the session |

### Unchanged forever
**Every `git` command, byte-for-byte** (catalog A of v1 stands in full) — plus
the three namespace laws: git never shadowed; refs auto-managed; degradation
invariant (worst case = a healthy git repo).

---

## What changed from v1 of this catalog (the honest diff)

| v1 claimed | v2 says | Why (review) |
|---|---|---|
| claims at dispatch prevent conflicts | union testing at landing is the oracle; claims = security fences + advisory hints (gated) | cross-cutting changes make claim-closures overlap → locks; SubmitQueue's real lesson is speculation, not prediction |
| regen rebase default for orthogonal intents | opt-in forever + independent verdict per regen; derived-files regen stays default | non-determinism + circular verification = Trojan horse to main |
| "gitted context": replay, diff-minds, resume | provenance + journals + short-horizon resume | context half-life ≈ one model rev; replay isn't reproducible |
| review = interrogate the change | adversarial panels + evidence-grounded answers | self-defense optimizes persuasion, not correctness |
| shadow checks on every write | snapshot cadence, budget-capped, opt-in | cost/noise were unbudgeted |
| semantic index in the core | build-graph impact queries now; full index deferred | 8–14 EM of research-grade work |
| one big Dev Kit (claims+CLI+queue) | ONE GitHub App: memoized CI + landing queue | painkiller test; first-dollar discipline |
| "code is the projection of intent" | intent+context+proof as first-class provenance over git | the metaphysics overclaimed; the product stands without it |

**What did NOT change:** the economic physics (memoize/dedupe/zero-rate — the
incentive war), git compatibility as sacred, the degradation invariant, the
human-always-follows guarantees (file tree unchanged, ledger, deep links,
GitHub mirror as permanent escape hatch), the pricing doctrine, and the
end-state ambition — the absorption map stands; only the order and the proof
obligations changed.
