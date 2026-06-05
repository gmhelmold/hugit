# hugit — the 10-day warp plan (v1)

> **Owner decisions (2026-06-05, recorded):**
> 1. The focus gate is **reformed**: CoreLink and hugit run in parallel with
>    **zero interference** between them ("um não interfere — AT ALL — no
>    outro"). CoreLink's launch route, runners and sessions are untouched by
>    this plan; hugit dogfood targets exclude `corelink-server` during its
>    launch window.
> 2. Execution model: **1 orchestrator × 15 agents, 24/7, two phases per
>    sprint** — Sprint 1 = B+C, Sprint 2 = D+E.
> 3. "Shipped" definition (the honest contract): in 10 days every phase is
>    **built, integrated, cold-verified and dogfooding on our own repos.**
>    Calendar-bound loops (external design partners, source-of-truth soak,
>    bidirectional write-back) start their clocks during the warp and run
>    in parallel after it — they gate *promotion*, never *build*.
>
> The experiment gate stands: claims-as-oracle and regen promotion only by
> data (WP-D8 builds the harness). The phase-B exit metric (10 external
> teams, 3 weeks, ≥40% retention) stands — its clock starts at D3 with
> design-partner recruiting, and it gates *charging money*, not building.

---

## Day 0 — compile the war (orchestrator + 4 agents, ~12h)

The techlead prep that makes aggressive parallelization safe:

| Step | Output |
|---|---|
| **Acceptance suites first** | every WP below gets a failing acceptance suite before any implementation (the demand, externalized — test-first) |
| **Contract freeze** | schemas locked: CheckDef, CheckResult, DiagnosisObject, IntentSidecar, RunnerLease, EventRecord, VerdictObject; API surfaces: App webhooks, queue API, runner API. Frozen = dependent WPs build against stubs |
| **Conflict map + DAG** | WPs sliced conflict-disjoint (table below IS the slice); merge order declared |
| **Scaffold** | `hugit` becomes a Rust workspace (house stack): `crates/{hugit-app, hugit-queue, hugit-checks, hugit-diag, hugit-runner, hugit-fence, hugit-refstore, hugit-proto, hugit-ledger, hugit-policy, hugit-mirror, hugit-cli}` + CI gates (fmt/clippy/test/audit) green on empty scaffold |
| **Provisioning** | GitHub App registered (dev); R2/AC namespaces (new tenant on CoreLink prod API — consuming it as a customer, zero server changes); 1 Hetzner-class box for runners |

**Owner checkpoints:** D0 spec approval → D5 mid-SEAL → D10 final SEAL.
(Your attention is the serialization point — everything else parallelizes.)

---

## Sprint 1 (D1–D5): Phase B ∥ Phase C

### Squad B — the GitHub App (8 agents)
*"Memoized CI for agent PRs — your green checks never re-run."*

| WP | Deliverable | Claims (disjoint) | Depends |
|---|---|---|---|
| B1 | App skeleton: auth, webhook ingest, Checks-API write-back (CF Worker) | `hugit-app` | — |
| B2 | checks-as-code: CheckDef format + executor (generalize `clw run`; memo key `H(tree‖def‖toolchain)`) | `hugit-checks` | contract |
| B3 | affected-targets v0: per-package graphs (cargo metadata / pnpm workspaces / turbo.json) | `hugit-checks/affected` | contract |
| B4 | union landing queue: batch landable PRs, union tree, ordered atomic merge via API, minimal-failing-pair report | `hugit-queue` | contract |
| B5 | auto-bisect over memoized checks + DiagnosisObject (culprit, diff-vs-green, suspect targets) | `hugit-diag` | contract |
| B6 | intent sidecar: PR metadata schema + renderer (charter/acceptance/context-ref as PR comment + check summary) | `hugit-app/sidecar` | B1 |
| B7 | surface v0: PR comments + minimal status page (NO new human CLI in phase B) | `hugit-app/ui` | B1 |
| B8 | dogfood harness: install on `hugit` + `corelink-workspaces` + 2 synthetic fleet repos (NOT corelink-server — non-interference) | `tests/dogfood` | B1–B7 |

### Squad C — the fabric (7 agents)

| WP | Deliverable | Claims | Depends |
|---|---|---|---|
| C1 | inventory + absorb existing runner work (campaign #1 state in corelink ecosystem — read-only, zero changes there) | `docs/inventory` | — |
| C2 | ephemeral runner v0 on the Hetzner box: container-per-job now, Firecracker upgrade path documented | `hugit-runner` | contract |
| C3 | cache-warm boot: `clw hydrate` on lease + toolchain layers from CAS | `hugit-runner/boot` | C2 |
| C4 | derived-file regeneration drivers v0: Cargo.lock + pnpm-lock (regenerate, never merge) wired into B4's union builds | `hugit-checks/regen` | contract |
| C5 | claim-fenced workspaces (SECURITY): sparse hydrate by path-set + secrets broker v0 (runner never holds tenant credentials) | `hugit-fence` | C2 |
| C6 | flake-stats collector: every check execution feeds per-test statistics from D1 (quarantine policy consumes later) | `hugit-diag/flake` | B2 |
| C7 | budgets/quotas: per-tenant check budgets + queue fairness | `hugit-queue/budget` | B4 |

**D5 — mid-SEAL:** integration day. Cold verification of every WP against its
acceptance suite (verifier ≠ author), security review (secrets paths, fence
escapes, webhook auth), gates green, dogfood LIVE on our repos. **Exit
criterion: a real agent-fleet PR wave on `hugit` lands through the union
queue with memoized checks and a regenerated lockfile.**
**D3 (parallel, non-blocking):** design-partner recruiting starts (jj
community + billing-backlash threads) — the calendar clock begins.

---

## Sprint 2 (D6–D10): Phase D ∥ Phase E

### Squad D — the forge, self-hosted alpha (9 agents)

| WP | Deliverable | Claims | Depends |
|---|---|---|---|
| D1 | event-log ref store: DO-per-repo, append-only hash-chained, **compaction/cold-tier to R2 designed in from day 1**; refs as derived view; `undo` as compensating event | `hugit-refstore` | contract |
| D2 | git wire protocol **read path** (clone/fetch, protocol v2, pack assembly from CAS) — serve the `hugit` repo itself | `hugit-proto/read` | D1 |
| D3 | push path v0 (receive-pack → CAS + event log) — **self-hosted flag only**; raw pushes recorded as opaque change-events (never fake intents) | `hugit-proto/write` | D1 |
| D4 | Intent objects native + projection (generated commits embedding intent ids; two-altitude history) | `hugit-refstore/intent` | D1 |
| D5 | `hugit ledger` + `hugit watch` (TUI) v0 over the event stream | `hugit-ledger` | D1,D4 |
| D6 | policy engine v0: declarative gates, fail-closed, locally testable (DCO/changelog/secrets ported as configs — our own gate museum is test case #1) | `hugit-policy` | contract |
| D7 | adversarial verdict panels: `verdict request --lens` fan-out (independent prompts/models, evidence-grounded; the change never defends itself) | `hugit-cli/verdict` | B6 |
| D8 | **the experiment harness**: claim-disjointness + regen-honesty data collection wired into our fleet's real waves (feeds the experiment gate) | `hugit-diag/experiment` | B4,C4 |

### Squad E — the bridge (6 agents)

| WP | Deliverable | Claims | Depends |
|---|---|---|---|
| E1 | one-way mirror hugit→GitHub: continuous, idempotent, **with a verification monitor** (diff-checked every push — the soak instrument; its uptime IS the trust metric) | `hugit-mirror/out` | D2 |
| E2 | GitHub import: history + PR/issue metadata → intents (proposed state) | `hugit-mirror/import` | D4 |
| E3 | status/badge compat emitter (ecosystem tools keep working) | `hugit-mirror/status` | B1 |
| E4 | Actions-YAML shim v0: run simple existing workflows on our runners (migration lubricant) | `hugit-runner/shim` | C2 |
| E5 | export guarantee: full-fidelity dump (git + documented JSON) — the anti-lock-in promise, executable from day 1 | `hugit-cli/export` | D1 |

**D10 — final SEAL:** full cold verification, security review (this time
including D3's write path — the source-of-truth bar), docs current, CHANGELOG,
and the shipped-state demo:

> `git clone https://hugit.humangr.com/humangr-labs/hugit` works (read path) ·
> the hugit repo lands its own fleet waves through its own union queue on its
> own runners · the ledger narrates it · the mirror keeps GitHub perfectly in
> sync · the experiment harness is accumulating the data that decides claims
> and regen.

---

## What D10 does NOT claim (the honest ledger)

| Not in 10 days | Why | Clock |
|---|---|---|
| External teams validated | humans use at human speed | starts D3, runs ~weeks |
| hugit as anyone's source of truth | trust = soak, not typing | write path self-hosted-only until the mirror monitor runs clean for weeks |
| Bidirectional write-back (E full) | the review's hardest "no": months of one-way soak first | instrument live D10 |
| Full semantic index / Mission Control web | deferred by catalog v2 | post-warp, by pull |
| Claims-as-oracle, regen promotion | experiment gate | D8 data decides |

## Quality law (unchanged at any speed)

Acceptance suite before implementation · SEAL + cold verification by
non-author agents per wave · security review at both SEALs · gates green
before merge · DCO + changelog discipline · **no warp exception to any of
these — speed comes from parallelization, never from skipped verification.**
