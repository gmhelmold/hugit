# The headless-engine doctrine — hugit works whole without githugr

> Owner-ratified product law (2026-06-10). Born from the owner's probe: *"sem
> o githugr não tem fila de landing, bundle por campanha rodando CI junto,
> etc, tem? não tem intent no GitHub"* — and the answer is the moat: **all of
> it lives in the engine; githugr only renders it.** Owner's reaction on
> understanding it: "melhor ainda — porque o githugr vai ser construído em
> cima dessa engine." This doc freezes that understanding so neither repo
> ever drifts from it.

## 1. The law

**hugit is a complete, headless forge engine.** Landing queue, campaign
bundles, union-testing, memoized CI, intents, envelopes, verdicts, provenance,
attestation — every differentiated capability **executes in the engine with no
UI attached**. githugr is a **window, never a dependency**: if githugr
disappeared tomorrow, no hugit capability would stop working — only stop being
*visible to human eyes* in its native form.

Corollary for githugr (campaign #4): it is a **read-layer first** product over
this engine (the frozen `Provider` seam) — it renders what the engine knows
and gates writes through the same engine paths. It never invents data, never
holds state of its own, never becomes load-bearing for correctness.

## 2. Where each capability actually lives (evidence-cited)

| Capability | Engine home | UI involvement |
|---|---|---|
| Landing queue (lanes, positions) | `hugit-queue` (`LandableEntry`) | none — githugr only displays |
| Campaign bundle + union-test (PRs of a campaign tested together) | `hugit-queue` + `hugit-checks` | none |
| Memoized CI (cache-hit ⇒ 0 execution) | `hugit-checks` + CoreLink AC | none |
| Intent (= enriched commit: charter, acceptance, sidecar) | `hugit-refstore` + `hugit-contracts::IntentSidecar` | none |
| Context envelopes, 4 altitudes (intent · PR · campaign · session) | `hugit-runner::envelope` + `hugit-dogfood` (WP-F2/F2b) | none |
| Cost rollups (decomposed work/orchestration/verification/ci/waste) | `hugit-ledger::rollup` (WP-F3) | none |
| Verdicts / adversarial panels | `hugit-contracts::VerdictObject` + ledger | none |
| Provenance / why-blame / impact | `hugit-cli why·impact` + event-log | none |
| Attestation + transparency | `hugit-contracts::AttestationChain` | none |
| GitHub sync (bidirectional, forge-arbitrated) | `hugit-mirror` | none |
| Exit / anti-lock-in | `hugit-cli export` | none |

## 3. The three fidelity layers (the user's choice, never our demand)

| Layer | What the user sees | Fidelity |
|---|---|---|
| **GitHub (the mirror)** | the **faithful shadow**: intent → ordinary commit (by design: intent ≡ commit), landing → merge, PR → PR, memoized-CI verdict → **commit status/check on the GitHub PR** (`hugit-mirror::status` emitter + badge, via the GitHub App) | degraded **on purpose** — enough to trust (green/red, history), never lossy on code |
| **CLI (the agent's door)** | everything, as stable JSON: `why` · `impact` · `tournament` · `export` + the flow porcelain (`intent` · `pr` · `campaign` — 2026-06-10 wave) | **full** — and this is the PRIMARY interface, because the primary typist is the user's LLM (owner law, 2026-06-10) |
| **githugr (the human window)** | the substance rendered: Landing kanban, attention inbox, envelope/trajectory, why-blame, Ledger, cost X-ray | full, for human eyes |

**What GitHub cannot show** (no vocabulary for it): charter/acceptance, the
context envelope and its transcripts, verdict panels, queue position, campaign
cost, why-blame. That asymmetry is not a gap to fix — **it is the moat**.
GitHub renders the shadow; the engine holds the substance; githugr is the only
place the substance becomes visible.

## 4. Why this shape (the founding principles it serves)

- **Embrace, don't assault** (compat ladder): the user adopts hugit inside the
  flow they already have — git CLI + GitHub as the visible face. Day one costs
  zero habit changes. githugr is where they *choose* to look deeper, never a
  forced migration. A broken bridge kills trust; a degraded-on-purpose,
  forge-arbitrated projection keeps it.
- **Don't deviate from git**: intent ≡ commit is what makes the GitHub shadow
  faithful for free.
- **LLM-native**: agents drive the engine through CLI/JSON with zero UI in the
  loop — headless is not a fallback mode, it is the primary mode.
- **Anti-lock-in**: `hugit export` proves the user can leave in writing; the
  mirror proves they never had to arrive all at once.

## 5. What this binds, going forward

1. **No engine capability may ever require githugr** to function, complete, or
   prove itself. (Tests live engine-side; githugr's parity tests consume them.)
2. **githugr reads through the `Provider` seam** and writes through engine
   paths — any feature that would need githugr-private state is, by
   definition, an engine feature being built in the wrong repo.
3. **The mirror's projection stays faithful-degraded**: never enriched with
  invented GitHub artifacts, never silently lossy on code or checks.
4. **The CLI porcelain is a first-class product surface** (not an admin tool):
   it gets the same design rigor as screens — stable JSON, structured errors,
   idempotency — because the paying user's agents live there.

Cross-references: `docs/whitepaper/hugit-v1.md` (compat ladder §10/§12) ·
`docs/interop.md` (the seams) · `docs/product/product.md` (ICPs/killers) ·
`../githugr/design/architecture.md` (the read-layer thesis) ·
`docs/plan/2026-06-10-cli-porcelain-wave.md` (the agent door).
