# Product-refinement audit — 6 product-lens auditors (2026-06-21)

Constructive ("fit like a glove", not "trash it") review across 6 lenses: onboarding,
the wedge/daily-loop, CLI ergonomics, positioning/value, adoption/migration, trust/legibility.

## The honest core insight (where 3 lenses converged)
hugit is genuinely **BUILT** but not yet a daily **GLOVE**. Three independent auditors found the
same root cause, stated three ways:
- **The killer engine is unreachable** — the union-test + bisect + memoize engine
  (`hugit-queue/core/union.rs`) is REAL and tested, but **no operator-invokable verb runs it**;
  `pr land` deliberately does NOT run the union verdict, and the only wiring is an in-process dogfood
  harness. *The product's #1 promise can't be invoked.*
- **The human-review surface is blind** — the diff is a universal blank (`file_count:0`, empty hunks),
  transcripts are always `None`, `VerdictVm.reviewer` is always `""`. A human approving fleet output
  has *zero code-level evidence in the UI*.
- **The CLI makes the loop verbose** — every verb requires `--log <path>` (no `init`, no default), so
  every session feels like scripting, not tool use.

Plus: **the value isn't legible** — no one-sentence "hugit gives you X GitHub can't"; it's buried under
CoreLink/CAS infra framing. None of this says the idea is wrong — it's lapidation to make the built
thing **usable + legible + believable**.

## Top levers (the glove-fit core — each cross-validated)

1. **Make the wedge REAL & reachable** *(P0, M)* — wire `hugit land --queue <log>` to a real `MemoCheck`
   over `hugit-checks::run_memoized` + a file-backed AC (the exact seam `hugit check --store` uses), so a
   single-tenant operator can batch-land union-tested, memoized PRs **locally today**; the runner fabric
   drops in later behind the same trait. Surfaces `failing_pair` (the signature bisect output) for free.
   *The single move that turns "built" into "the daily loop works."*
2. **Kill the `--log` friction** *(P0, S)* — `hugit init` (writes `.hugit/log.json`) + default `--log` to
   `$HUGIT_LOG`/`.hugit/log.json` across all verbs. *The #1 daily-feel win; today the 2nd command a user
   types is a guaranteed "file not found".*
3. **Make agent work LEGIBLE to the human** *(P0, M)* — wire the real git diff (file list + hunks, via the
   existing `hugit_proto` tree-walk, `HUGIT_SERVE_GIT_DIR`-gated) and the transcript blob-fetch into the
   review/intent surfaces; set `VerdictVm.reviewer = vo.model` (one line) so approvals read
   "claude-opus-4 / correctness: APPROVE". Triage-sort the attention feed by the already-computed blast
   radius. *Turns the approve button from a trust leap into a review.*
4. **Make the value LEGIBLE & believable** *(P0, S–M)* — lead every surface with the pain→cure couplet
   (below); add honest **live / next / horizon** badges to each "killer" (stop selling the unshipped ones
   present-tense); ship the **public dogfood ledger** (real cache hit-rate + flat-vs-metered cost on our
   own fleet) — the one proof no competitor can fake. Pick **one beachhead ICP** (fleet operators = the
   dogfood persona), not five.
5. **Git-proximity CLI cleanup** *(P1, S–M)* — `check`/`checks` → one `check run|show|key`; `approve`/
   `reject` → `verdict approve|reject`; `pr land --settle` → `pr queue` + `pr land`; `journal note` →
   `note`; `diag` → `bisect`; strip internal jargon (`WP-*`, `Phase D`, the X5 rationale) from `--help`;
   `hugit` with no args → human help on a TTY (not raw JSON); converge `intent`'s `suggested_fix`→`fix`.

## Trust-breakers & hidden strengths (quick, high-ROI)
- **`git push` → silent 404** reads as "broken" → return **403 with a message** ("push not yet supported;
  use `hugit land`"). Add `git_serving:bool` to `/readyz`. *(P0/P1, S)*
- **Surface the real strengths that are invisible:** `hugit export` (the exit guarantee — a top adoption
  de-risker), `hugit import <github-url>` (the on-ramp; logic is built, the verb isn't), the **compat
  ladder** as a "where hugit is today" README table, and **safety/provenance** (undo, claim-fences,
  model-level attestation — built, under-sold) promoted to a top pillar.
- **`hugit symbol`** is the perfect standalone "first aha" (works with no server/log) — surface it first
  in the README + `--help`, not 23rd.
- **Set expectations gracefully** on honest-null/refused outputs (add a `note:` telling the user what to
  run to populate evidence).

## The sharpest "why hugit" (positioning auditor)
> *"Your agent fleet ships branches that are green alone and red together. hugit lands them on a main
> that's always green and re-runs zero CI it's already paid for — on your existing GitHub repos,
> migrating nothing."*

## Suggested sequencing
- **Wave 1 (days, mostly S — instant glove-feel):** `hugit init` + default `--log`; the CLI git-proximity
  cleanup; the value one-liner + honest badges; `git push` 403; surface export/import/symbol + compat
  ladder in the README.
- **Wave 2 (the product-existence wave, M):** wire `hugit land --queue` (the wedge becomes real) + the
  diff/transcript/reviewer legibility surface. These two make the daily loop both **work** and **be
  trustworthy**.
- **Wave 3 (proof & GTM):** the public dogfood ledger; beachhead-ICP messaging; deploy the public repo
  cloneable (HUGIT_SERVE_GIT_DIR) as rung-0.

Full per-lens findings: in the 6 auditor reports (this session).
