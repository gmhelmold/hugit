# hugit — formal decomposition (v1.5 — post critic round 5)

> v1.5 (2026-06-05): integrates **round 5** — **B, C, D all DRY**; E:2, X:2
> (+2 vague adjudicated). Survivors: E6 bidirectional mirror recorded as an
> EXPLICIT gate-bound deferral with acceptance pre-registered (the silence
> was the bug, not the gate); E4④ Actions-shim execution EQUIVALENCE vs real
> GitHub Actions; X13 legibility×degradation/erasure (the human can follow
> in EVERY substrate state — degraded paths fail honestly, erasure chains
> end in honest tombstones); X14 deep-link referential integrity across the
> full object lifecycle (zero dangling links, ever); D12⑤ anti-smuggling
> (false "derived" declarations cannot bypass the regen gate). Orchestrator
> adjudication recorded: "no-fake-intents at runtime" is COVERED by
> D3⑤+D4④+E2⑤ (mapping documented for future critics).
> Convergence: R1=42 → R2=23 → R3=16 → R4=4 → R5=4 (3/5 dry). Round 6
> dispatched; loop closes after two consecutive dry rounds; then re-slice
> into ≤100k-token (80k ideal) WP contracts with DoD.
>
> Sizing: S ≤ ½ agent-day · M ≤ 1 · L ≤ 2. Routing: `opus` = design/security,
> `sonnet` = contract-determined build. New/changed since v1 marked **(+)** /
> **(🔧)**.

## 0. Capability tree (v1.1)

```
B GitHub App       C fabric             D forge                E bridge
├ memoized checks  ├ runners (warm)     ├ event refs+undo      ├ verified 1-way mirror
├ union landing    ├ regen derived      ├ git protocol R/W     ├ import priv/LFS/large
├ diagnosis/bisect ├ shadow checks (+)  ├ intents/ledger/queue ├ compat status+actions
├ intent corpus    ├ fences+broker SEC  ├ why/impact/journals(+)└ export (exit promise)
└ exit telemetry(+)├ ws lifecycle (+)   ├ policy+verdicts
                   └ flakes+budgets     ├ regen gated(+)·tournament(+)·auth(+)
                                        └ experiment harness
X invariants (+): tenant isolation · attestation · context privacy · supply
  chain · namespace laws · resource non-interference · degradation kill-test
```

## 1. Contract freeze (Day 0)

v1 set stands (CheckDef · CheckResult · DiagnosisObject · IntentSidecar ·
RunnerLease · FenceManifest · EventRecord · VerdictObject · QueueApi ·
AppWebhooks) **plus (+)**: `ShadowPolicy {cadence, budget, optin}` ·
`AttentionRank {policy, blast_radius, confidence}` · `ExportSchema`
(versioned, machine-validatable) · `AttestationChain {tree, def, runner,
model, principal, sig}` · `RegenGate {optin_scope, repass, indep_verdict}`.

---

## 2. Phase B — Squad B (10 WPs)

| WP | Size/route | Acceptance items (each red→green) |
|---|---|---|
| **B1** App skeleton | M·sonnet | ① forged webhook→401+audit · ② PR event persisted, ack<1s · ③ check-run on real PR · ④ least-privilege manifest snapshot · **⑤(+) uninstall revokes access + halts processing (audited)** |
| **B2** checks-as-code | L·opus | ① repeat tree+def→AC hit, 0 exec, <500ms · ② glob sensitivity (in→rerun, out→hit) · ③ **🔧 local≡runner BYTE-IDENTICAL (artifact/digest compare, not result-equal)** · ④ **🔧 non-determinism flagged after 3 divergent runs**, honest surface · **⑤(+) npm fixture: partial hit-rate measured & displayed as-is, no full-memo claim** · **⑥(R2) toolchain sensitivity: different toolchain → MISS, never false hit** · **⑦(R3) def sensitivity: changed check definition, same tree+toolchain → MISS + re-execute (3rd key axis)** |
| **B3** affected-targets | M·sonnet | ① golden sets cargo/pnpm/turbo · ② root edit→full set · ③ unknown ecosystem→full set fail-open |
| **B4** union queue | L·opus | ① A+B-red pair excluded+named · ② 5 disjoint greens land, 0 re-runs · ③ force-push recompute · ④ crash idempotent (kill-test) · **⑤(+) lands in queue order; out-of-order structurally prevented** · **⑥(+) protected/required-review PR is HELD+reported, never force-merged; merge method honored** |
| **B5** bisect+diagnosis | M·sonnet | ① culprit ≤log₂ execs · ② diff-vs-green + suspects · ③ <2min fixture · **④(+) diagnosis is bounded schema data, never raw log dump (size assert)** · **⑤(R2) bisect triggers automatically on any red — no manual invocation** |
| **B6** intent sidecar | S·sonnet | ① parsed/validated/rendered · ② malformed→actionable comment · ③ corpus→CAS by intent_id · **④(R2) negative: sidecar is non-authoritative — never gates or blocks landing** |
| **B7** surface v0 | S·sonnet | ① live status page · ② exactly one edited comment/PR · ③ **🔧 every saved-minutes number links to its CheckResult set (auditable)** · **④(R3) the "$ saved" figure derived from minutes via a versioned, auditable cost model (rates stated), reconcilable against the minutes count** |
| **B8** dogfood | M·sonnet | ① real 5-PR wave e2e · ② **🔧 vs defined baseline (same wave, memoization off), versioned report with formulas** · ③ 48h soak: 0 wrong-merge/lost-PR (event-audited) |
| **B9 (+)** exit telemetry | S·sonnet | ① per-install activity → week-3 retention computable vs the ≥40% threshold (privacy-documented) · ② **🔧 feedback capture distinguishes UNPROMPTED ("I'd pay") statements from prompted responses — only unprompted count toward the ≥3 gate** · ③ exit-metric report generated from data, auditable · **④ 🔧 cohort/window guards: n=10 external teams, ≥3 weeks real use, evaluation window ANCHORED to the first-10-paying-customers event and inside 90 days — otherwise "insufficient/out-of-window", never a pass** · **⑤(R4) the ≥3 gate is ENFORCED as pass/fail: 2 correctly-counted unprompted signals → FAIL even with ≥40% retention; exactly 3 → PASS** |
| **B10 (R3)** negative scope | S·sonnet | ① no claim/lease acquired at dispatch — conflict discovery happens ONLY at landing/union (assert mechanism absent) · ② rebase in phase B is textual-fallback only — regenerative path absent/disabled (assert) |

## 3. Phase C — Squad C (10 WPs)

| WP | Size/route | Acceptance items |
|---|---|---|
| **C1** runner inventory | S·opus | ① written inventory + reuse verdict/item. **(R2: "zero changes to adjacent prod repos" reclassified → governance law §8, not a feature acceptance)** |
| **C2** ephemeral runner | L·opus | ① destroy leaves nothing (forensic re-scan) · ② lease isolation (tmp/net) · ③ expiry hard-kill · ④ ≥8 concurrent/box · **⑤(R2) box crash mid-job (not expiry): job detected lost → requeued/surfaced, no silent drop, no false green, lease/fence cleaned up** |
| **C3** cache-warm boot | M·sonnet | ① warm ≤10s vs cold ≥60s · ② toolchain layers shared · **③(+) CAS/AC down mid-job → fail CLOSED, zero poisoned writes, no hang, no false green** |
| **C4** regen drivers | M·sonnet | ① lockfile union regen green, 0 markers · ② deterministic regen · ③ non-lockfile untouched · **④(R2) all three derived classes exercised: codegen + snapshot files regenerate (never text-merge); hand-edits to derived files discarded+regenerated** · **⑤ 🔧 regeneration FAILURE (e.g. unsatisfiable constraints: the regen command itself fails) → fail CLOSED with clear status; never a silently/partially merged derived file** · **⑥(R3) METHOD proof: fixture where a clean text-merge would yield DIFFERENT bytes than regeneration → regenerated bytes win AND the merge codepath is provably never entered for derived-classified paths** |
| **C5** fences+broker SEC | L·opus | ① outside path_set→ENOENT · ② zero secret material in job (red-team env/proc/disk) · ③ broker calls audited w/ principal chain · ④ broker down→fail CLOSED · **⑤(+) active escape red-team: traversal/symlink/out-of-fence writes/fork-bomb/disk-fill contained; cannot reach another lease or starve the box** · **⑥(R2) positive path: job completes a credential-needing operation VIA the broker successfully, raw credential provably absent during and after** |
| **C6** flake stats | S·sonnet | ① every execution feeds stats · ② planted 20% flake detected <30 runs · ③ **🔧 quarantine list = policy artifact; "auto-act" defined: any reorder/skip/block/annotation-that-gates = prohibited in v0 (annotation-only allowed)** · **④(R3) false-positive guard: a deterministically-failing (non-flaky) test is classified as REAL failure and NEVER quarantined within the same volume window** |
| **C7** budgets | S·sonnet | ① **🔧 exhausted→queued not dropped; surfaced as defined status field + event** · ② **🔧 fairness bound: under contention, every tenant's p95 queue wait ≤ defined bound and throughput share ≥ defined floor (interleave fixture)** · **③ 🔧 metering accuracy: accounted ≈ actual within ±5% on the fixture workload** |
| **C8 (+)** shadow checks | M·opus | ① N writes in one snapshot window → exactly ONE shadow pass at boundary (not N, not 0) · ② shadow runs decrement tenant budget; cap halts shadows, explicit jobs proceed per policy · ③ default-off; per-repo opt-in flag scoped to repo, zero runs when off · **④(R2) a failing shadow surfaces as signal/event and NEVER gates/blocks/fails any explicit job; a passing shadow produces an observable result** · **⑤(R2) per-tenant cap isolation: tenant A exhausting its shadow budget leaves tenant B's shadows unaffected** |
| **C9 (+)** ws lifecycle | M·sonnet | ① attach joins live workspace (same fence/materialization) without respawn · ② resume restores state+fence; resumed ws cannot exceed original path_set · ③ spawn <1s; identical concurrent spawns dedup to one materialization · ④ local vs remote execution: identical observable results |
| **C10 (R2)** pricing no-shock | S·sonnet | ① driving a tenant to budget exhaustion on EACH metered surface (runner minutes, shadow spend, storage) → system caps/degrades (pauses or falls back) with pre-exhaustion warning · ② zero overage charge generated — flat means flat (billing fixture assert) |

## 4. Phase D — Squad D (14 WPs)

| WP | Size/route | Acceptance items |
|---|---|---|
| **D1** event refs | L·opus | ① 10k-event replay identical · ② tamper detected · ③ compaction replay-equivalent, hot log bounded · ④ undo restores + preserves history · ⑤ 100 concurrent ops: serialized, 0 loss, p99<500ms · **⑥(+) recovery: hot-DO loss → full ref state rebuilt from cold tier (and/or mirror) replay-identical** |
| **D2** protocol read | L·opus | ① clone byte-identical to mirror · ② delta-only fetch · ③ clients: git 2.40+/jj/libgit2 · ④ **🔧 500MB fixture: per-request CPU-time p95 ≤70% of the platform per-request CPU limit; beyond → chunked fallback path exercised+passing** · **⑤ 🔧 degradation kill-test: smart layers disabled (both steady-state AND injected mid-operation) → vanilla git clone/fetch still serves valid repo** · **⑥ 🔧 scale ceilings defined+tested per dimension (repo size, ref count, concurrent clients, pack size): at each limit → documented bounded behavior, never silent failure** · **⑦(R2) jj FIRST-CLASS: stacked-changes series round-trips via jj with change-ids stable across forge ops; stack reconstructs identically** |
| **D3** push v0 (flagged) | L·opus | ① push→clone round-trip identical · ② concurrent pushes: total order, correct stale rejection · ③ raw push = external-change w/ attribution · ④ flag off unless self-hosted-alpha · **⑤(+) negative: NO synthetic intent fabricated for a raw push (intent log clean)** |
| **D4** intents+projection | M·opus | ① commits embed intent_id, reproducible from log · ② two altitudes consistent (50-intent fixture) · ③ sidecar corpus importable · **④(+) mixed fixture (intents + raw pushes interleaved on one ref): altitudes stay provably consistent, externals as external-change** |
| **D5** ledger+watch+fleet | M·sonnet | ① asked→done→proven per campaign · ② **🔧 watch: EventRecord-to-display p95 <2s (measured per event class)** · ③ **🔧 deep-links resolve to golden expected targets (not just non-error)** · **④(+) planted secret renders REDACTED in ledger/verdict views** · **⑤(R2) two-zoom toggle: intent view ⇄ raw-commit view mutually consistent over the same fixture (one store)** · **⑥(R2) `hugit fleet` emits documented machine-readable schema reflecting true ws/agent state vs fixture** |
| **D6** policy engine | M·sonnet | ① 3 ported gates local≡forge · ② engine down→landing blocks (kill-test) · ③ policy change = audited event |
| **D7** verdict panels + review Q&A | M·opus | ① lenses isolated (prompt audit) · ② valid VerdictObject[] + evidence refs · ③ **🔧 planted bug of a NON-author-visible class (semantic/logic, demonstrably uncovered by any author test) caught by ≥1 lens** · **④(R2) human review Q&A: answers = citations to real evidence objects; no grounding → explicit refusal, never fabricated** · **⑤(R3) DIVERSITY enforced: homogeneous (same prompt+model) panel rejected/flagged; real panel dispatches distinct prompts and ≥2 distinct models** · **⑥(R3) no-self-defense negative: planted persuasive false self-justification in author-controlled fields is unreachable by reviewers; verdict identical with vs without it (no persuasion channel exists)** |
| **D8** experiment harness | M·opus | ① every wave auto-contributes datapoints · ② dashboard: disjointness %, regen agree/disagree, n · ③ gate report generated, never hand-written · **④(R2) anti-gaming: promotion corpus pre-registered and SEALED before evaluation; sample selection auditable** · **⑤(R2) post-hoc removal of any change from the corpus is detected and invalidates the promotion verdict** |
| **D9 (+)** attention queue | M·opus | ① fixture with known policy/blast/confidence → documented composite ordering reproduced · ② perturbing one input moves entry to expected position · ③ policy-mandatory items can never be ranked out of the human's view · **④(R2) fast-approve (90s) affordance is BLOCKED for high-risk/policy-mandatory items — forced through full review; permitted for policy-low-risk** |
| **D10 (+)** why+impact | M·sonnet | ① `hugit why <line\|symbol>` → originating intent + charter/author/model/cost, matching event log · ② `hugit impact <path\|change>` → golden affected-set on known build graph · ③ impact feeds verdict-panel ground truth (cross-check) |
| **D11 (+)** journals+resume | M·sonnet | ① journal persisted as tenant-private object bound to ws/intent · ② post-crash `ctx resume` reconstructs session within supported horizon · ③ beyond-horizon resume refused/degraded as documented |
| **D12 (+)** regen gate | M·opus | ① regen only on opt-in scope; non-opted repo never regens · ② regen lands only if acceptance re-passes AND fresh independent adversarial verdict approves · ③ missing/failing either → blocked + reported · ④ every regen auditable as its own revision · **⑤(R5) anti-smuggling: a file not provably derived (regeneration command must deterministically produce it from sources) CANNOT be classified derived — bypassing the regen gate via a false "derived" declaration is blocked + audited** |
| **D13 (+)** tournament | S·sonnet | ① `-n N` produces N independent candidates · ② judge panel selects per documented criteria (fixture w/ known-best) · ③ losers remain addressable as evidence |
| **D14 (+)** forge authz | M·opus | ① mutating endpoints (push/land/undo/policy) reject unauthorized principals · ② permission model documented + golden-tested per principal class · ③ authz denials audited |

## 5. Phase E — Squad E (7 WPs)

| WP | Size/route | Acceptance items |
|---|---|---|
| **E1** verified mirror | L·opus | ① landing on GitHub <60s hash-verified · ② divergence→alarm+repair+incident · ③ 72h soak 100% verified · **④(+) GitHub 429/5xx for N hours: durable queue, bounded backoff, no drop/reorder; on recovery drains to verified sync + incident records gap** · **⑤(+) force-push/branch-delete/tag ops replicate; deleted refs absent; no false divergence from orphans** · **⑥(+) partial divergence: repair scoped to broken ref only; webhook loss → poll fallback still detects within SLA** · **⑦(R2) ONE-WAY enforced: a write made directly on the GitHub mirror never propagates back/never becomes truth — treated as divergence (alarm → forge-authoritative repair → incident), zero reverse sync** · **⑧(R3) cold-seed bootstrap: full existing history replicates to a fresh GitHub repo, hash-verified to byte-identity, resumable mid-seed** · **⑨(R3) GitHub-side loss DR: App revocation or mirror-repo deletion/rename detected → incident → recoverable (re-establish + verified re-seed) with zero forge-side loss** · **⑩ 🔧 the outage queue (④) has a stated capacity bound; overflow → backpressure + incident, never drop** |
| **E2** import | M·sonnet→**L** | ① 1k-commit public import byte-identical · ② PRs/issues→proposed intents w/ provenance · ③ idempotent re-import · **④(+) private repo via installation auth; LFS objects materialized (not pointers); >1-timeout repo resumes and completes byte-identical** · **⑤(R2) import boundary: commit history materializes as opaque change-events — NO intent synthesized from a bare commit; PR/issue intents marked proposed/non-authoritative** · **⑥(R3) PR/issue fidelity contract: stated set (body, comment/review threads, state, labels, cross-refs) preserved with per-element provenance; non-imported elements explicitly enumerated; verified on a fixture containing each element** · **⑦ 🔧 idempotency defined: unchanged source → no-op; changed source → documented incremental re-sync (never dupes)** |
| **E3** status compat | S·sonnet | ① checks appear as GitHub statuses · ② **🔧 badge reflects true state within a stated staleness bound; status-API down → last-known + observable staleness, never silent wrong** · **③(+) status-API 429/5xx: retry w/ backoff → eventually true state; no stuck-pending; failures observable** |
| **E4** Actions shim | M·sonnet | ① **🔧 the SUPPORTED subset is a published contract; "supported" = proven-to-execute, not merely documented** · ② **🔧 outside the contract → explicit actionable report — falsifiable boundary, no silent skip** · **③(+) missing/denied secret → fail CLOSED w/ named secret; material never in logs/env (red-team)** · **④(R5) execution EQUIVALENCE: a fixture workflow inside the contract runs on real GitHub Actions AND on the shim → equivalent observable outcomes (steps, env, artifacts, exit states)** |
| **E6 (R5)** bidirectional mirror — **DEFERRED, gate-bound** | L·opus | **Explicitly OUT of warp scope** (gated behind months of E1 one-way soak — panel + catalog decision). Acceptance PRE-REGISTERED for when the gate opens: ① write-back is BOUNDED (rate/scope limits enforced) · ② webhook sync idempotent (duplicate/out-of-order deliveries converge) · ③ forge-authoritative conflict handling (GitHub-side concurrent edit → forge wins, divergence incident) · ④ property test: no state reachable where the two sides sync symmetrically without forge authority ("never naive symmetric") |
| **E5** export | S·sonnet→**M** | ① one-command dump git + documented JSON · ② restore round-trip reproduces refs+intents+events · **③(🔧 was doc-presence) export validates against versioned ExportSchema (machine check)** · **④(+) export applies context/journal redaction policy (no secret material emitted); multi-GB export streams without OOM** · **⑤(R2) THE EXIT PROOF: the exported git artifact (and the mirror) is fully usable with ZERO hugit/forge dependency — clone/log/branch/push-elsewhere all work with no hugit tooling present** · **⑥(R3) completeness: export+restore reproduces ALL first-class object classes (refs, intents, events, ledger, verdicts, journals, policy, provenance links) object-for-object; any out-of-scope class explicitly enumerated in the ExportSchema** · **⑦(R3) redaction red-team: seeded secrets appear NOWHERE in the exported git/JSON; AND the redacted artifact still passes the exit proof, removals manifested** · **⑧(R3) exit under exit conditions: export succeeds, complete and valid, on a suspended / past-due / offboarding account (read-only terminating path)** · **⑨(R4) "any moment" consistency: export on a LIVE account under concurrent mutation (landings + mirror sync + event append) yields ONE point-in-time-consistent cut — no dangling provenance link, no event referencing an absent object; restore is self-consistent** |

## 6. Squad X — platform invariants (14 WPs, cross-cutting)

| WP | Size/route | Acceptance items | Scheduled |
|---|---|---|---|
| **X1** tenant isolation | L·opus *(red-team)* | ① tenant B requesting a key whose private bytes came from tenant A → miss/deny, never served · ② forged/collision memo-key attempts → deny + alert, no poisoning · ③ public-deterministic artifact IS shared, with proof no private bytes rode along | sprint 1 (before any external tenant) |
| **X2** attestation e2e | M·opus | ① artifact attestation resolves full chain (tree+def+runner+model+principal) cryptographically · ② tampered/unsigned attestation rejected at promotion · ③ verification is a public, documented procedure | sprint 2 |
| **X3** context privacy | M·opus | ① context/journals tenant-scoped (cross-tenant fetch denied) · ② redaction at capture AND export · ③ retention/deletion purges (verified absent) · ④ training/eval exclusion: documented control + audit trail | sprint 2 |
| **X4** supply chain | M·opus | ① runner images content-pinned + integrity-verified at spawn · ② App dependencies pinned + verified in CI · ③ tampered/unpinned image → fail CLOSED before any tenant work | sprint 1 |
| **X5** namespace laws | S·sonnet | ① no hugit CLI verb shadows a git verb (mechanized check against `git help -a`) · ② managed refs (`refs/hugit/…`) never collide with arbitrary user branches/tags (property test) | sprint 2 |
| **X6** non-interference | M·sonnet | ① hugit at full load concurrently with CoreLink workloads → CoreLink latency/availability unaffected within stated tolerance (measured, both SEALs) · ② hugit infra is resource-isolated from CoreLink runners/sessions (separate boxes/quotas, asserted by config test) | both SEALs |
| **X7 (R2)** right-to-erasure | L·opus | ① a data subject's personal data provably erased across CAS + provenance/ledger + context store + the GitHub mirror · ② no orphaned provenance refs survive erasure · ③ attestation chains re-seal or fail CLOSED after erasure (never silently broken) | sprint 2 |
| **X8 (R2)** self-release attestation | M·opus | ① every hugit App/CLI/runner-image release signed + published to a verifiable transparency log · ② the running App verifies its own provenance at boot · ③ unsigned/tampered self-build fails CLOSED | sprint 2 |
| **X9 (R2)** cross-phase CheckResult identity | S·sonnet | ① a CheckResult memoized by the phase-B App is bit-identical to the one served as evidence in a phase-D verdict panel for the same (tree,def,toolchain) · ② mismatch fails CLOSED + alerts | sprint 2 |
| **X10 (R3)** the focus gate itself | M·sonnet | ① under hugit's heaviest sustained load (runner fleet + union queue + dogfood soak): CoreLink's launch route/sessions/CI capacity show ZERO measurable degradation vs a hugit-idle baseline · ② the dogfood target set provably excludes corelink-server — enrollment during the launch window FAILS the build · ③ **🔧 X6 rescoped: X6 = intra-hugit tenant isolation (axes: CPU/IO/DO-storage/CAS-bandwidth/runner-slots, bounds stated); X10 = the adjacent-product boundary** | both SEALs |
| **X11 (R4)** degradation composition | M·opus | ① smart-layer failure injected MID-operation (partial degradation window, not just steady-state outage): secrets broker fails CLOSED — no credential reaches any workspace during degradation · ② objects written during degradation are marked provenance-ABSENT; no synthetic intent/attestation ever fabricated by a fallback path · ③ CoreLink non-interference (X10 baseline) holds WHILE hugit is degraded, not only when healthy | sprint 2 |
| **X12 (R4)** erasure × provenance × mirror | M·opus | ① after an erasure request the attestation chain remains independently verifiable with the erased object as a tamper-evident TOMBSTONE (never silently re-linked) · ② the mirror-side erasure obligation (data already replicated to GitHub) is discharged or explicitly surfaced as residual risk — and that disclosure is part of the export/exit proof | sprint 2 |
| **X13 (R5)** legibility × degradation/erasure | M·opus | ① with the intelligence layer DEGRADED: the human's down-zoom (raw-commit view, deep links, `why`) still resolves via plain git OR fails HONESTLY (explicit "layer unavailable"), never a silent 404/blank · ② after an erasure cascade: following any chain reaches an honest tombstone, never a broken link — the human can ALWAYS follow, in every substrate state | sprint 2 |
| **X14 (R5)** deep-link referential integrity (lifecycle) | M·sonnet | ① property test across the full object lifecycle — after compaction/cold-tier to R2, after mirror round-trip, after tombstoning: every ledger/intent deep link resolves to its target or to a tamper-evident tombstone · ② ZERO dangling links, ever (continuous integrity check as a standing fixture) | sprint 2 |

---

## 7. The dependency DAG (v1.2 — additions: C10⇠C7; X7⇠{D1,X3,E1}; X8⇠X4; X9⇠{B2,D7}; D16-class items live inside D8)

```
Day 0: contracts(+ShadowPolicy/AttentionRank/ExportSchema/AttestationChain/RegenGate)
       ─→ B2 B3 B4 B5 | C2 C4 | D1 D6 | X4

B1 ─→ B6 B7 B9 E3        C2 ─→ C3 C5 C8 C9 E4     D1 ─→ D2 D3 D4 D5 D14 E5
B2 ─→ B5 C6 X1           B4 ─→ C7 D8              D2 ─→ E1 D2.⑤(kill-test)
B6 ─→ D7 D10             C4 ─→ D8 D12             D4 ─→ E2 D5 D9
B1..B7 ─→ B8             C5 ─→ X1 X4              D7 ─→ D12 D9
B8 ─→ B9(report)                                  D10 ─→ D9(blast-radius input)

Critical path S1: contracts → B2 → B4 → B8 (+X1 before any external tenant)
Critical path S2: contracts → D1 → D2 → E1 (+D2.⑤ degradation kill-test)
Sprint barrier = checkpoint, not dependency (D1/D6/X-WPs may start early).
```

**Conflict map:** every WP claims a disjoint crate/path (tables above); shared
types only in `hugit-contracts` (orchestrator-owned, frozen). Cross-claim
writers remain B8 + final SEAL, at barriers. New squad X writes only its own
test crates + red-team fixtures (`crates/hugit-invariants`).

## 8. Routing & verification law (v1.1)

- `opus`: B2 B4 C2 C5 C8 D1–D4 D7 D8 D9 D12 D14 E1 X1–X4 (+C1) — design,
  security, or adversarial judgment.
- `sonnet`: the rest — contracts make them deterministic builds.
- Every WP: failing acceptance suite committed BEFORE implementation; SEAL
  with evidence; cold verification by non-author; security review at both
  SEALs; dedicated red-team passes on C5, D3, X1, X4.
- **Governance laws (reclassified from acceptance):** zero changes to
  adjacent production repos (ex-C1 item) is enforced by the orchestrator's
  profile/neverTouch + the git-hygiene guard — a standing law, not a test.
- **Critic loop status:** R1 = 42+8 → v1.1. R2 = 23+8 → v1.2. R3 = 16+10 →
  v1.3. R4 = 4+2 (C,D dry) → v1.4. **R5 = 4+2 (B,C,D dry; E:2 X:2)** → v1.5.
  Round 6 dispatched. Loop closes after two consecutive dry rounds; then WP
  token re-slicing (≤100k hard / 80k ideal, DoD per WP).
- **Adjudications on record (so future critics don't re-litigate):**
  "no-fake-intents at runtime" covered by D3⑤ (steady-state raw push) +
  D4④ (mixed-altitude fixture) + E2⑤ (import boundary); bidirectional
  mirror = E6, explicitly gate-bound deferral with pre-registered
  acceptance, NOT in warp scope; pip-vs-npm partial-hit honesty =
  representative coverage via B2⑤ (behavior is ecosystem-agnostic).