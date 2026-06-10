# Runner-transfer campaign — hugit-runner → corelink-runners (campaign #1 seed)

> Owner-directed 2026-06-10: *"hugit é basicamente um git, não faz sentido o
> runner ser feature do hugit"* — transfer the proven ephemeral-runner v0 out
> of the hugit workspace into `../corelink-runners`, where it becomes the
> execution core of **CoreLink Runners (campaign #1)**. TechLead concurs:
> "nothing built twice"; hugit's wedge is landing, not compute.
>
> Status: **PLANNED — execution gated on WP-F2/F2b merge** (F2 is editing
> `hugit-runner` right now; moving the crate mid-flight is a guaranteed
> collision). This doc is the decomposition for the fleet.

## 0. End-state architecture (the one drawing that matters)

```
BEFORE                                  AFTER
hugit workspace                         hugit workspace
 ├─ hugit-runner  (C2a/C2b/C3/E4)        ├─ (runner crate GONE)
 ├─ hugit-fence  ──depends on runner     ├─ hugit-fence    (repo-side only)
 ├─ hugit-invariants ─dev-dep runner     ├─ hugit-invariants (conformance fixtures/mock)
 └─ hugit-contracts (RunnerLease…)       └─ hugit-contracts (UNCHANGED — frozen)
                                                 │  wire contract (JSON vectors)
corelink-runners (docs only)            corelink-runners workspace
 └─ docs/spec/                           ├─ corelink-runners-contracts (transcribed types + goldens)
    hugit-integration-contract.md        ├─ corelink-runner (the transplanted core + C-suites)
                                         └─ docs/spec/hugit-integration-contract.md v1.1 (envelope emission)
```

**Iron rules of the end state:**
- **No git-dependency in either direction** (both deny.toml: crates.io only).
  The seam is the **wire contract**: types transcribed on each side, proven
  equivalent by **shared JSON conformance vectors committed byte-identical in
  both repos** (the AC/CAS pattern, applied to runners).
- hugit keeps: `RunnerLease` & friends in `hugit-contracts` (frozen, untouched),
  the consumer/client seam, the invariant proofs (against fixtures).
- corelink-runners gets: the execution core (lease→container→forensic teardown,
  warm boot, concurrency/expiry/recovery, Actions-YAML shim) + its acceptance
  suites (C2a/C2b/C3/E4) + workspace discipline cloned from hugit.
- Envelope capture (WP-F2 output) is **hugit/forge domain** — whatever F2
  landed inside `hugit-runner` relocates hugit-side during the re-cut (R0
  decides the home from F2's actual diff).

## 1. Go/no-go + constraints

- **PARALLELIZABLE in two lanes** (different repos): the corelink-runners lane
  (R1→R2→R3) never touches hugit files; the hugit lane (R4→R5) never touches
  corelink-runners. Zero shared files by construction. R6 is docs/spec.
- **Gate:** nothing starts before **F2/F2b are merged** (R0 inventories F2's
  diff). Hard precondition.
- ⚠ **corelink-runners has other live sessions** (carve-out rule): every WP
  that writes there starts with `git status` + `git log -3` sanity, lands only
  fixup-style commits, **never amends**, and works on a branch
  (`integ/seed-runner`), PR'd per that repo's convention (code now exists →
  branch→PR→merge applies).
- **CI from commit 1 on the self-hosted fleet** (`[self-hosted, mac,
  corelink-builder]`) — never ubuntu-latest (the billing-wall lesson, today).
- **Done means done:** a half-transfer (crate copied but hugit still shipping
  its own runner) violates "nothing built twice" and does NOT land without an
  explicit owner waiver. The original is deleted from hugit **only after** the
  transplant passes the full gate in its new home (verify-landed-before-prune,
  writ large).

## 2. Campaign acceptance suite (definition of done, decided BEFORE slicing)

| # | Acceptance (all must hold) |
|---|---|
| A1 | corelink-runners workspace: full gate green (fmt · clippy `-D warnings --locked` · test `--locked` · audit `--deny warnings` · deny check) **including the transplanted C2a/C2b/C3/E4 acceptance suites** |
| A2 | hugit workspace: full gate green **without** `crates/hugit-runner`; package count + CLAUDE.md corrected |
| A3 | Conformance vectors: identical SHA-256 manifest committed in BOTH repos; each side's golden tests pin them |
| A4 | No cross-repo git/path dependency (deny `[sources]` clean on both sides) |
| A5 | Records: 67-WP register carries an E6-style supersession appendix for the C-series (register itself stays frozen); CLAUDE.md (both) + absorption-map + CHANGELOGs (both) updated; campaign-#1 handoff note in corelink-runners/docs/handoff |
| A6 | `hugit-integration-contract.md` amended to v1.1 with the envelope-emission obligation (closes the deferred dep flagged 2026-06-10) — amended from the hugit side, documented, never silently |
| A7 | F2's envelope capture still fully proven in hugit after the re-cut (its tests pass in the new home) |

## 3. The WPs

### WP-R0 — preflight + seam freeze · **LEAD ONLY, not delegable** · after F2/F2b merge
**Charter.** Freeze every boundary before any agent moves a byte.
**Owned acceptance.** (1) Inventory `crates/hugit-runner` post-F2: module list,
which files carry F2 capture code → name their hugit-side destination.
(2) Decide the `hugit-fence` split: in-container enforcement (C5a pieces that
ride the runner) moves; repo-side session fence stays — if the split is not
clean, **ESCALATE to owner, do not slice**. (3) Decide `hugit-invariants`
strategy: spawn-surface consumption re-pointed at conformance fixtures or a
mock spawn seam (proofs stay hermetic). (4) Freeze the transcription list:
exactly which `hugit-contracts` types are re-declared in
`corelink-runners-contracts` (start set: `RunnerLease`; extend only from the
actual `use hugit_contracts::…` surface of the runner crate). (5) Author the
conformance-vector set v1 (JSON) + SHA-256 manifest. (6) `git status`/`log`
sanity on corelink-runners; confirm no other session is mid-work on overlapping
paths. **Output:** final charters for R1–R6 with the frozen lists embedded
(agents transcribe, never decide).

### WP-R1 — corelink-runners workspace foundation · S · ~60k · lane B (corelink-runners) · branch `integ/seed-runner`
**Charter.** Give campaign #1 the same impeccable-repo skeleton hugit has.
**Owned acceptance.** Cargo workspace + `rust-toolchain.toml` (1.96.0 pinned) +
`deny.toml` (house policy: crates.io only, multiple-versions deny + documented
skips) + `.github/workflows/{ci,dco}.yml` on `[self-hosted, mac,
corelink-builder]` + CHANGELOG + `corelink-runners-contracts` crate holding the
R0-frozen transcribed types with `deny_unknown_fields`, schema_version, golden
round-trips seeded from the R0 vectors. Full gate green on the skeleton.
**Deps.** R0 (the frozen transcription list + vectors).

### WP-R2 — transplant the execution core · M · ~80k · lane B · branch `integ/seed-runner`
**Charter.** Move the code; change nothing behavioral.
**Owned acceptance.** `crates/hugit-runner` → `corelink-runner` in the new
workspace, modules intact (lease · isolation · teardown · boot · concurrency ·
expiry · recovery · shim · ws · pin), **minus** the F2 capture files named by
R0 (those stay in hugit). Provenance header in every moved file (source repo +
SHA). Imports retargeted `hugit_contracts` → `corelink_runners_contracts`
(R0 list — if a needed type is NOT on the list: ESCALATE, don't transcribe ad
hoc). Acceptance suites C2a/C2b/C3/E4 move and pass unmodified except imports.
Full gate green. **Behavioral diff = zero** (same tests, same assertions).
**Deps.** R1.

### WP-R3 — cross-repo conformance harness · S · ~50k · lane B (writes corelink-runners; READS hugit) · branch `integ/seed-runner`
**Charter.** Prove the wire seam without sharing code.
**Owned acceptance.** The R0 vector set committed under
`corelink-runners/conformance/` + manifest; golden tests in
`corelink-runners-contracts` pin every vector byte-exact; a `manifest.sha256`
whose digest EQUALS the one hugit will commit (R4). Round-trip both directions
(serialize from types → vector; vector → types → re-serialize byte-identical).
**Deps.** R2 (types in their final home).

### WP-R4 — hugit seam re-cut · M · ~80k · lane A (hugit) · branch `integ/runner-transfer`
**Charter.** Slim hugit to git+forge; keep every proof.
**Owned acceptance.** Remove `crates/hugit-runner` from the workspace. Relocate
the R0-named F2 capture files to their hugit-side home; their tests pass
unchanged (A7). `hugit-fence`: drop the runner dependency per the R0 split.
`hugit-invariants`: spawn-surface consumption re-pointed at the R0
fixtures/mock seam — every INV-* proof still runs and passes (proof rigor is
NOT loosened; if an invariant genuinely cannot be proven without the live
crate, ESCALATE — do not weaken or skip it). Commit hugit's copy of the
conformance vectors + the SAME `manifest.sha256` (A3). Full gate green;
workspace member count + CLAUDE.md "18-package" wording corrected.
**Deps.** R2 green in its new home (never delete before the transplant lives),
R0 lists. **Disjoint from R1–R3 by repo.**

### WP-R5 — records + supersession · S · ~40k · both repos (docs only) · branches as above
**Charter.** No stale claim survives.
**Owned acceptance.** (a) hugit `docs/plan/decomposition.md`: supersession
appendix — "C2a/C2b/C3/E4 TRANSFERRED → corelink-runners @ <sha> (campaign #1
seed); consumed via wire contract" (E6 precedent; the frozen register body is
not edited). (b) hugit CLAUDE.md family map + package count; absorption-map.
(c) corelink-runners CLAUDE.md: status design-phase → **code** (methodology,
gates, fence note). (d) CHANGELOG entries both repos. (e)
`corelink-runners/docs/handoff/2026-06-XX-runner-seed.md` addressed to the
campaign-#1 sessions: what arrived, what it proves, what the PRODUCT still
needs (multi-tenant · API · billing — seeded ≠ shipped). **Deps.** R4 + R3.

### WP-R6 — integration-contract amendment v1.1 · S · ~40k · lane B (spec) · branch `integ/seed-runner`
**Charter.** Close the deferred envelope dependency properly.
**Owned acceptance.** `docs/spec/hugit-integration-contract.md` v1.0 → v1.1:
the runner's obligation to emit per-job metrics consistent with
`IntentMetrics` + the capture hook points for trajectory blobs (full +
compacted — the two-transcript imperative), version-bumped with an amendment
log, cross-referencing hugit ADR-0001. Negotiated wording = the lead reviews
against the frozen hugit side before merge. **Deps.** R0 (and conceptually
F2's final shape).

## 4. Conflict map & schedule

```
            ┌── lane B (corelink-runners) ──┐        ┌─ lane A (hugit) ─┐
F2/F2b ──► R0 ──► R1 ──► R2 ──► R3 ─────────┼──► R4 ──► R5
 (gate)   (lead)              └──► R6 ──────┘    (R4 needs R2 green)
```
- Pairwise: R1/R2/R3/R6 share lane-B branch → **sequential within lane B**
  (same branch, same repo). R4 is lane A — runs in parallel with R3/R6 once R2
  is green. R5 last, touches both but docs-only.
- Concurrency cap respected: ≤2 agents at any moment (R3 ∥ R4 is the only
  parallel pair). Multi-session safety on corelink-runners re-checked at every
  lane-B dispatch.
- Each WP: compact return card · cold-verify by the lead (gate re-run cold) ·
  merge by the lead in DAG order · worktree pruned after verify-landed.

## 5. Risks (named, with owners)

| Risk | Mitigation |
|---|---|
| F2 capture code interleaved deep in runner internals | R0 reads F2's actual diff before slicing; if inseparable → redesign the capture seam BEFORE R2, owner informed |
| `hugit-fence` split not clean (C5a tangled with repo-side fence) | R0 escalation clause — owner decides the fence's home |
| An INV-* proof needs the live spawn surface | R4 escalation clause — rigor never loosened to make the move fit |
| Other sessions mid-work in corelink-runners | status/log check at R0 + every lane-B dispatch; branch+PR; no amends |
| New repo CI hits the billing wall | self-hosted labels from commit 1 (today's fix is the template) |
| Transcribed types drift from hugit's frozen ones over time | A3: the shared vector manifest is the tripwire — either side's golden breaks on drift |

## 6. What this campaign does NOT do (honest scope)

- Does not build CoreLink Runners **the product**: no multi-tenant control
  plane, no public API, no billing, no Firecracker. It seeds the execution
  core that all of that wraps.
- Does not touch the live runner box (`hugit-runner-01`) provisioning or
  secrets — day-0 infra pointers move as DOCUMENTATION only.
- Does not unfreeze `hugit-contracts` — `RunnerLease` stays exactly where and
  what it is; transcription ≠ relocation.
