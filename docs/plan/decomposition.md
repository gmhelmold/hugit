# hugit — formal decomposition (v1)

> The working-backwards step between the refined product (catalog v2) and the
> specs: every work-package with a charter and **testable acceptance
> criteria**, the **contracts to freeze** (with schemas), the **dependency
> DAG**, the conflict map, and routing. This is the artifact Day 0 of the
> warp plan executes against; each WP's acceptance section becomes its
> failing acceptance suite before implementation starts.
>
> **Owner:** Gustavo Schneiter · 2026-06-05 · companion to
> `warp-10-days.md` (schedule) and `command-catalog.md` v2 (the what/why).
> Sizing: S ≤ ½ agent-day · M ≤ 1 agent-day · L ≤ 2 agent-days (24/7 agents).
> Routing: `opus` = design-heavy/ambiguous · `sonnet` = well-specified build.

---

## 0. The capability tree (catalog v2 → buildable capabilities)

```
PHASE B — the GitHub App                 PHASE C — the fabric
├─ B-CAP1 memoized verification          ├─ C-CAP1 ephemeral cache-warm compute
├─ B-CAP2 union landing                  ├─ C-CAP2 derived-file regeneration
├─ B-CAP3 causality (diagnosis/bisect)   ├─ C-CAP3 claim fences + secret broker (SECURITY)
└─ B-CAP4 intent corpus (sidecar)        └─ C-CAP4 fleet telemetry (flakes, budgets)

PHASE D — the forge (self-hosted alpha)  PHASE E — the bridge
├─ D-CAP1 event-sourced truth (refs)     ├─ E-CAP1 verified one-way mirror
├─ D-CAP2 git projection (wire protocol) ├─ E-CAP2 import & compat surface
├─ D-CAP3 intents native + ledger        └─ E-CAP3 exit guarantee (export)
├─ D-CAP4 policy + adversarial review
└─ D-CAP5 the experiment harness (gates claims/regen)
```

---

## 1. Contract freeze (Day 0 — the shared truth all WPs build against)

Frozen = published as Rust types + JSON Schema in `crates/hugit-contracts`
**before** any dependent WP dispatches. Owner: orchestrator. Changes after
freeze require explicit re-freeze + re-dispatch of affected WPs.

| Contract | Essential shape |
|---|---|
| `CheckDef` | `{id, cmd, input_globs[], targets[], toolchain_ref, timeout_s, env_allowlist[]}` |
| `CheckResult` | `{def_digest, tree_root, toolchain_digest, status: Green\|Red\|Skipped, duration_ms, log_ref, artifact_refs[], runner_id, attestation_sig}` — memo key = `H(tree_root ‖ def_digest ‖ toolchain_digest)` |
| `DiagnosisObject` | `{failure: CheckResultRef, culprit: PrRef\|IntentRef, last_green_tree, diff_ref, suspect_targets[], similar_failures[]}` |
| `IntentSidecar` | `{intent_id (ULID), campaign, charter, acceptance[], context_ref?, principal: {actor, model?, orchestrator?}}` |
| `RunnerLease` | `{lease_id, tenant, workspace_ref, fence: FenceManifest, budget: {cpu_s, wall_s}, expires_at}` |
| `FenceManifest` | `{path_set[], net_policy: deny-by-default + allowlist, broker_endpoint}` — **no secret material, ever** |
| `EventRecord` | `{seq, prev_hash, ts, principal, kind, payload_ref, sig}` — append-only, hash-chained |
| `VerdictObject` | `{subject: PrRef\|IntentRef, tree_root, lens, model, verdict: APPROVE\|FIX-FIRST\|REJECT, claims_checked[], evidence_refs[]}` |
| `QueueApi` | `submit(pr_set) → batch_id` · `status(batch_id) → {position, union_tree, results}` · `report(batch_id) → DiagnosisObject[]` |
| `AppWebhooks` | consumed: `pull_request, check_suite, push, installation`; emitted: Checks-API runs + PR comments; permissions: checks RW, contents RW, PRs RW — least-privilege documented |

---

## 2. Work-packages — Phase B (Squad B, 8 agents)

**B1 · App skeleton** — `M · sonnet` — claims `crates/hugit-app`
Charter: GitHub App on a CF Worker: installation auth, webhook ingest (signature-verified), Checks-API write-back.
Accept: ① forged-signature webhook → 401 + audit event; ② valid `pull_request` event → persisted + ack <1s; ③ posts a check-run visible on a real test-repo PR; ④ App manifest pins least-privilege permissions (snapshot-tested).

**B2 · checks-as-code executor** — `L · opus` — claims `crates/hugit-checks` — deps: contracts
Charter: `CheckDef` runner generalizing `clw run`: execute, memoize in CoreLink AC under the frozen key, replay locally byte-identical.
Accept: ① same tree+def twice → second is AC hit, 0 execution, <500ms; ② tree changed by 1 byte in inputs → re-runs; tree changed outside `input_globs` → still hits; ③ `hugit check --local` and runner produce identical `CheckResult` (modulo runner_id); ④ non-deterministic check flagged after N divergent results, surfaced honestly (no fake greens).

**B3 · affected-targets v0** — `M · sonnet` — claims `crates/hugit-checks/affected` — deps: contracts
Charter: per-package change→target mapping for cargo, pnpm workspaces, turborepo.
Accept: ① in a 10-crate fixture, edit crate X → exactly X + reverse-deps selected (golden tests per ecosystem); ② root-file edit (Cargo.toml workspace) → full set; ③ unknown ecosystem → full set (fail-open to safety, never silent skip).

**B4 · union landing queue** — `L · opus` — claims `crates/hugit-queue` — deps: contracts
Charter: batch landable PRs, build union tree, run affected memoized checks on the union, land green in order via merge API, report minimal failing pair.
Accept: ① fixtures A,B green alone / A+B red → batch lands neither A+B, lands the non-conflicting rest, `report` names {A,B} with the failing check; ② 5 green disjoint PRs → all land, **0 re-executed checks** (all AC hits from their PR runs); ③ mid-batch force-push → batch recomputes, no stale union ever merges; ④ crash/restart mid-batch → idempotent (no double-merge), proven by kill-test.

**B5 · auto-bisect + diagnosis** — `M · sonnet` — claims `crates/hugit-diag` — deps: contracts, B2
Charter: on any red, bisect over memoized checks to the culprit; emit `DiagnosisObject`.
Accept: ① 8-PR fixture with 1 planted breaker → culprit named, ≤ log₂ executions (rest AC hits); ② diagnosis includes diff-vs-last-green + suspect targets; ③ end-to-end <2min on the fixture.

**B6 · intent sidecar** — `S · sonnet` — claims `crates/hugit-app/sidecar` — deps: B1
Charter: attach/validate `IntentSidecar` on PRs; render as PR comment + check summary.
Accept: ① PR opened with sidecar (via CLI or PR-body block) → parsed, validated, rendered; ② malformed → actionable error comment, never silent drop; ③ corpus persisted to CAS keyed by intent_id (D8's input).

**B7 · surface v0** — `S · sonnet` — claims `crates/hugit-app/ui` — deps: B1
Charter: PR comments + one status page (`queue position, batch state, minutes/$ saved counter`). No new human CLI in phase B.
Accept: ① status page renders live batch state; ② every PR gets exactly one continuously-edited comment (no spam); ③ the saved-minutes counter is computed from real AC hits (auditable), not vibes.

**B8 · dogfood harness** — `M · sonnet` — claims `tests/dogfood` — deps: B1–B7
Charter: install on `hugit`, `corelink-workspaces`, + 2 synthetic fleet repos (cargo & pnpm). **Never corelink-server** (non-interference).
Accept: ① a real 5-PR agent wave on `hugit` lands through the queue end-to-end; ② cache hit-rate + wall-time vs baseline measured and published in the repo; ③ 48h soak: zero wrong-merge, zero lost-PR (audited from events).

## 3. Work-packages — Phase C (Squad C, 7 agents)

**C1 · runner inventory** — `S · opus` — claims `docs/inventory` — read-only on CoreLink ecosystem.
Accept: written inventory of existing runner/campaign-#1 assets with reuse verdict per item; zero changes to corelink repos.

**C2 · ephemeral runner v0** — `L · opus` — claims `crates/hugit-runner` — deps: contracts
Charter: container-per-job on the Hetzner box; lease lifecycle; Firecracker upgrade path documented.
Accept: ① lease→boot→execute→destroy, nothing persists after destroy (verified by forensic re-scan); ② concurrent leases isolated (no shared tmp/net namespaces); ③ lease expiry hard-kills; ④ throughput: ≥8 concurrent check jobs on 1 box.

**C3 · cache-warm boot** — `M · sonnet` — claims `crates/hugit-runner/boot` — deps: C2
Accept: ① warm boot (CAS-hot workspace) ≤10s to first check command vs cold ≥60s (measured fixture); ② toolchain layers content-addressed and shared across jobs.

**C4 · regeneration drivers** — `M · sonnet` — claims `crates/hugit-checks/regen` — deps: contracts
Charter: Cargo.lock + pnpm-lock declared derived; union builds regenerate instead of merge.
Accept: ① two PRs each adding a dep (textual lockfile conflict) → union regenerates, builds green, **zero conflict markers**; ② regeneration is deterministic across two runs (byte-identical or normalized-equal); ③ non-lockfile conflicts untouched (no scope creep).

**C5 · claim fences + secret broker** — `L · opus` — claims `crates/hugit-fence` — deps: C2 *(SECURITY-critical)*
Charter: sparse workspace materialization by path-set; deny-by-default network; broker performs privileged ops, credentials never on the runner.
Accept: ① job reads outside path_set → ENOENT (file genuinely absent, not permission-flak); ② `env`/proc/disk scan inside job finds zero secret material (red-team test); ③ broker ops are audited per-call with principal chain; ④ kill-test: broker down → jobs fail CLOSED.

**C6 · flake-stats collector** — `S · sonnet` — claims `crates/hugit-diag/flake` — deps: B2
Accept: ① every CheckResult feeds per-(def,target) stats; ② a planted 20%-flaky test is detected <30 runs; ③ quarantine list is a policy artifact (consumed later, never auto-acts in v0).

**C7 · budgets/quotas** — `S · sonnet` — claims `crates/hugit-queue/budget` — deps: B4
Accept: ① per-tenant budget exhausted → queued not dropped, surfaced on status page; ② fairness: no tenant starves another (interleave test).

## 4. Work-packages — Phase D (Squad D, 9 agents)

**D1 · event-sourced ref store** — `L · opus` — claims `crates/hugit-refstore` — deps: contracts
Charter: DO-per-repo append-only hash-chained `EventRecord` log; refs derived; compaction/cold-tier to R2 from day 1; `undo` = compensating event.
Accept: ① 10k-event log replays to identical ref state (determinism test); ② chain verification detects any tampered record; ③ compaction preserves replay-equivalence with hot log ≤ configured bound; ④ `undo` of a landing restores prior refs AND preserves full history; ⑤ 100 concurrent ref ops on one repo: serialized, zero loss, p99 <500ms.

**D2 · wire protocol read path** — `L · opus` — claims `crates/hugit-proto/read` — deps: D1
Charter: smart-HTTP protocol v2 clone/fetch with pack assembly from CoreLink CAS.
Accept: ① `git clone` of the hugit repo from the alpha endpoint → byte-identical tree to the GitHub mirror (verified by hash); ② incremental fetch sends only delta packs; ③ clients tested: git 2.40+, jj, libgit2; ④ Workers CPU budget measured on a 500MB-repo fixture, documented with headroom or chunked fallback.

**D3 · push path v0 (self-hosted flag)** — `L · opus` — claims `crates/hugit-proto/write` — deps: D1, D2
Charter: receive-pack → CAS objects + event log. Raw pushes recorded as **opaque change-events** (never fabricated into intents).
Accept: ① push→clone round-trip is byte-identical; ② concurrent pushes to one ref: one wins, the other gets a correct stale rejection, log totally ordered; ③ a raw push appears in the ledger as `external-change` with full attribution; ④ feature-flagged off for any repo not tagged self-hosted-alpha.

**D4 · intents native + projection** — `M · opus` — claims `crates/hugit-refstore/intent` — deps: D1
Accept: ① landing an intent emits generated commits embedding intent_id, deterministically reproducible from the event log; ② `git log` (machine altitude) and intent log (human altitude) provably consistent on a 50-intent fixture; ③ sidecar corpus from B6 importable as native intents.

**D5 · ledger + watch (TUI)** — `M · sonnet` — claims `crates/hugit-ledger` — deps: D1, D4
Accept: ① `hugit ledger` renders asked→done→proven per campaign from the event stream; ② `hugit watch` live-updates <2s after an event; ③ every entry deep-links (intent → diff → check → diagnosis ids resolvable via CLI).

**D6 · policy engine v0** — `M · sonnet` — claims `crates/hugit-policy` — deps: contracts
Charter: declarative gates, fail-closed, locally testable. Test case #1: port our own gate museum (DCO, changelog, secrets-scan).
Accept: ① the 3 ported gates pass/fail identically local vs forge on golden fixtures; ② engine unreachable → landing blocks (fail-closed proven by kill-test); ③ policy change is itself an audited event.

**D7 · adversarial verdict panels** — `M · opus` — claims `crates/hugit-cli/verdict` — deps: B6
Charter: `verdict request --lens a,b,c` fans out independent reviewers (distinct prompts, models where available) against served ground truth (diff + affected graph + acceptance evidence). The change never defends itself.
Accept: ① 3 lenses run isolated (no shared context contamination — verified by prompt audit); ② output = valid `VerdictObject[]` with evidence refs; ③ planted-bug fixture: ≥1 lens catches a seeded logic bug that the author-agent's own tests miss (the anti-circularity smoke test).

**D8 · experiment harness** — `M · opus` — claims `crates/hugit-diag/experiment` — deps: B4, C4
Charter: instrument real fleet waves to measure (a) claim-disjointness rate of intent pairs, (b) regen honesty (regenerate D′ on moved base; own-acceptance pass vs independent verdict disagree).
Accept: ① every dogfood wave auto-contributes datapoints; ② dashboard: disjointness %, regen agree/disagree counts, n; ③ the gate report is generated, not hand-written (the data decides claims/regen promotion).

## 5. Work-packages — Phase E (Squad E, 6 agents)

**E1 · verified one-way mirror** — `L · opus` — claims `crates/hugit-mirror/out` — deps: D2
Accept: ① every alpha-repo landing appears on GitHub <60s, content-hash-verified per push; ② divergence → alarm + auto-repair + incident event (the soak instrument); ③ 72h soak on the hugit repo: 100% verified syncs, count published.

**E2 · GitHub import** — `M · sonnet` — claims `crates/hugit-mirror/import` — deps: D4
Accept: ① import of a 1k-commit public repo: history byte-identical (re-clone proof); ② PRs/issues land as proposed-state intents with provenance; ③ idempotent re-import (no dupes).

**E3 · status/badge compat** — `S · sonnet` — claims `crates/hugit-mirror/status` — deps: B1
Accept: ① our checks appear as GitHub commit statuses (badge/bot/ecosystem-readable); ② shields.io renders a live badge off it.

**E4 · Actions-YAML shim v0** — `M · sonnet` — claims `crates/hugit-runner/shim` — deps: C2
Accept: ① a simple real-world workflow (checkout+setup+test matrix) runs unmodified on our runner with correct env/secrets-broker mapping; ② unsupported features fail with an explicit actionable report, never silent skips.

**E5 · export guarantee** — `S · sonnet` — claims `crates/hugit-cli/export` — deps: D1
Accept: ① one command dumps git-everything + all hugit objects as documented JSON; ② restore-from-export on a fresh system reproduces refs + intents + events (round-trip proof); ③ documented as the contractual exit promise.

---

## 6. The dependency DAG

```
Day 0: contracts ──┬─→ B2 B3 B4 B5 │ C2 C4 │ D1 D6
                   └─→ scaffold, App registration, Hetzner box

B1 ─→ B6 B7 E3          C2 ─→ C3 C5 E4         D1 ─→ D2 D3 D4 D5 E5
B2 ─→ B5 C6             B4 ─→ C7 D8(+C4)       D2 ─→ E1
B6 ─→ D7                B1..B7 ─→ B8           D4 ─→ E2 D5

Critical path: contracts → B2 → B4 → B8 (sprint 1) ║ contracts → D1 → D2 → E1 (sprint 2)
Sprint barrier D5→D6 is a CHECKPOINT, not a dependency: D1/D6 may start
against frozen contracts as soon as squad capacity frees.
```

**Conflict map:** every WP claims a disjoint crate/path (tables above) — zero
overlapping write-claims by construction; shared types live only in
`hugit-contracts` (frozen, orchestrator-owned). The two integration WPs
(B8, and D10's final SEAL) are the only cross-claim writers, and they run at
barriers.

## 7. Routing & verification law

- `opus` WPs (B2 B4 C2 C5 D1 D2 D3 D4 D7 D8 E1, C1): ambiguity or
  security-critical — design judgment required.
- `sonnet` WPs (the rest): contracts make them deterministic builds.
- **Every WP:** failing acceptance suite written and committed BEFORE
  implementation; SEAL with evidence; cold verification by a non-author
  agent against the suite + charter; security review at both SEALs (D5, D10)
  with C5/D3 receiving dedicated red-team passes.
- Merge order follows the DAG; the orchestrator is the only merger.
```
