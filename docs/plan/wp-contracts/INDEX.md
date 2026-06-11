# WP contracts — the re-slice register (v1)

> The accepted suite (decomposition v2.0, 13-round critic loop) re-sliced
> into **67 SOTA work-package contracts**, each ≤100k tokens hard / 80k ideal
> of executing-agent context, each with zero live decisions. One file per WP
> in this directory. Acceptance items are carried **VERBATIM** from
> decomposition v2.0 — they were adversarially forged; paraphrase is
> corruption.
>
> **Owner:** orchestrator (contracts pre-decided; agents transcribe) ·
> 2026-06-05

## The contract template (binding for every WP file)

```
# WP-<id> — <title>
squad · size · model route · context budget · branch: wp/<id>
## Charter            (what + why, 2–4 lines, zero ambiguity)
## Owned acceptance   (VERBATIM items from decomposition v2.0; the red→green set)
## Contract deps      (frozen types/APIs consumed from hugit-contracts; never modified here)
## Claims             (paths this WP owns — disjoint by construction; writes outside = leak)
## Dispatch packet    (exactly what the executing agent receives: files, anchors, conventions)
## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
## DoD                (global: fmt+clippy+test+audit green · owned items red→green · cold-verify pass by non-author)
## Completeness       (all owned items green · zero writes outside claims · evidence bundle attached to SEAL)
## Return shape       (SEAL: ≤20 lines — status, evidence refs, deviations=none|waiver-ref)
```

## Split decisions (L-sized WPs cut to the 80k sweet spot)

| Original | Split into | Rationale |
|---|---|---|
| B2 | **B2a** CheckDef format + local executor + memo key (AC client) · **B2b** runner-side execution + byte-identity + non-determinism + honest hit-rate | client/runner are separable concerns; B2b depends on B2a's frozen CheckDef |
| B4 | **B4a** queue core (batching, union tree, ordering, idempotent state machine) · **B4b** GitHub integration (merge API, branch protection, force-push recompute, kill-tests) | pure engine vs API surface |
| C2 | **C2a** lease lifecycle + isolation · **C2b** concurrency/throughput + crash recovery + expiry | correctness vs load |
| C5 | **C5a** sparse fence materialization + path enforcement · **C5b** secrets broker + escape red-team harness | fence vs broker; red-team rides b |
| D1 | **D1a** event-log core (append, hash chain, replay, tamper) · **D1b** compaction/cold-tier + recovery + undo · **D1c** concurrency/perf (serialization, p99) | three proof families |
| D2 | **D2a** pack assembly + clone/fetch core · **D2b** client matrix + jj stacks + CPU/chunked fallback + degradation kill-test + scale ceilings | build vs prove |
| D3 | **D3a** receive-pack→CAS+log · **D3b** concurrency/total order + external-change + flag/negatives | same |
| E1 | **E1a** outbound sync + hash verify + ordering/queue · **E1b** failure modes (outage, partial divergence, one-way enforcement) · **E1c** bootstrap + DR (cold-seed, GitHub-side loss, substrate-loss) | happy path / failure / DR |
| E2 | **E2a** git history import (byte-identity, LFS, resumable, idempotency) · **E2b** PR/issue→proposed intents + fidelity contract + boundary | git vs metadata |

## The 67 contracts (id · title · size · route · ctx budget)

### Day 0 (2)
WP-00 hugit-contracts crate (all 15 frozen types, Rust+JSON Schema+golden serde tests) · S · sonnet · 60k
WP-01 workspace scaffold (12 crates, CI gates green on empty, DCO+changelog config) · S · sonnet · 50k

### Squad B (12)
B1 App skeleton · M · sonnet · 70k — B2a checks client · M · opus · 80k — B2b runner execution+honesty · M · opus · 80k — B3 affected-targets · M · sonnet · 70k — B4a queue core · M · opus · 90k — B4b GitHub integration · M · opus · 90k — B5 bisect+diagnosis · M · sonnet · 70k — B6 intent sidecar · S · sonnet · 50k — B7 surface v0 · S · sonnet · 50k — B8 dogfood harness · M · sonnet · 70k — B9 exit telemetry+money gate · S · sonnet · 50k — B10 negative scope · S · sonnet · 40k

### Squad C (12)
C1 runner inventory · S · opus · 40k — C2a lease+isolation · M · opus · 80k — C2b load+crash · M · opus · 70k — C3 cache-warm boot · M · sonnet · 60k — C4 regen drivers · M · sonnet · 70k — C5a fence · M · opus · 80k — C5b broker+red-team · M · opus · 80k — C6 flake stats · S · sonnet · 50k — C7 budgets+fairness · S · sonnet · 60k — C8 shadow checks · M · opus · 70k — C9 ws lifecycle · M · sonnet · 70k — C10 pricing no-shock · S · sonnet · 50k

### Squad D (18)
D1a event-log core · M · opus · 80k — D1b compaction+recovery+undo · M · opus · 80k — D1c concurrency/perf · S · opus · 60k — D2a pack/clone core · L→M · opus · 90k — D2b clients+jj+limits+degradation · M · opus · 90k — D3a push core · M · opus · 80k — D3b push concurrency+negatives · M · opus · 70k — D4 intents+projection · M · opus · 80k — D5 ledger+watch+fleet · M · sonnet · 70k — D6 policy engine · M · sonnet · 60k — D7 verdict panels+Q&A · M · opus · 80k — D8 experiment harness+gate binding · M · opus · 70k — D9 attention queue · M · opus · 60k — D10 why+impact · M · sonnet · 70k — D11 journals+resume · M · sonnet · 60k — D12 regen gate · M · opus · 70k — D13 tournament · S · sonnet · 50k — D14 forge authz · M · opus · 60k

### Squad E (9)
E1a mirror outbound · M · opus · 80k — E1b mirror failure modes · M · opus · 80k — E1c mirror bootstrap+DR · M · opus · 80k — E2a history import · M · sonnet · 80k — E2b PR/issue import · M · sonnet · 70k — E3 status compat · S · sonnet · 50k — E4 Actions shim · M · sonnet · 70k — E5 export+exit proofs · M · opus · 80k — E6 bidirectional — **SUPERSEDED 2026-06-08** by `docs/design/2026-06-08-seamless-bidirectional-sync.md` (forge-arbitrated seamless sync; `main` single-writer via the landing queue). Now ACTIVE (branch `wp/bidir-sync`), no longer deferred · L · opus · 60k

### Squad X (14)
X1 tenant isolation red-team · L→M · opus · 90k — X2 attestation e2e · M · opus · 70k — X3 context privacy · M · opus · 70k — X4 supply chain · M · opus · 60k — X5 namespace laws · S · sonnet · 40k — X6 intra-fabric isolation · M · sonnet · 60k — X7 erasure cascade · L→M · opus · 80k — X8 self-release attestation · M · opus · 60k — X9 cross-phase identity · S · sonnet · 50k — X10 focus gate (incl. API-tenancy channel) · M · sonnet→opus · 80k — X11 degradation composition · M · opus · 80k — X12 erasure×provenance×mirror · M · opus · 70k — X13 legibility composition · M · opus · 60k — X14 deep-link integrity · S · sonnet · 50k

## Post-v1 follow-on (additive — register stays frozen)

ADR-0001 (intent context envelope) adds net-new rework not in the built v1:
**WP-F1** envelope contract freeze · **WP-F1b** session altitude · **WP-F2**
metrics+trajectory emission · **WP-F2b** orchestrator/campaign capture ·
**WP-F3** projection+PR rollup. Contracts + impact map + sequence in
`docs/plan/2026-06-09-adr-0001-context-envelope-rework.md`.
**COMPLETE** (ADR-0001 §7 fully ratified 2026-06-10; F-series built and
gate-green; schema advanced to 1.2.0 — WA4 integer micro-USD).

## Rules of the register
1. Every contract carries its acceptance items VERBATIM from decomposition
   v2.0; splits partition the original's items explicitly (no item orphaned —
   the union of a split's owned items = the original's set).
2. Budgets are HARD: a contract whose execution would exceed 100k re-enters
   the splitter (ADaPT), never silently overflows.
3. DoD is global and identical for all (the one quality bar); acceptance is
   per-WP (the "built the right thing" proof). They never merge.
4. The orchestrator is the only merger; merge order follows the DAG in
   decomposition v2.0 §7.
