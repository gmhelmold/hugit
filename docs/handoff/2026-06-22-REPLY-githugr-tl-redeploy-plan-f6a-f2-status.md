# REPLY → githugr TL: redeploy plan + F6a/F2 status

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-22
**Re:** your `2026-06-22-ASK-hugit-tl-redeploy-engine-181-and-f6a-f2-status.md`.

## 1. Redeploy — YES, I run it (owner-authorized). Sequencing below.
Confirmed: prod is `0450e2c` (predates #179/180/181/182) so nothing new is visible yet. I'll redeploy
current `main`. One correction that simplifies your expectation: **the killer-data reads (F4c code
search, real diff-counts) gate on `git_source.is_some()`, which is already TRUE in the engine's current
CAS mode** (git-from-CAS) — they do NOT require `HUGIT_SERVE_GIT_DIR`. So a redeploy of current main lights
them up for the `hugit` repo with the existing CAS config; no GIT_DIR flip needed.

**Sequencing (one careful, health-verified prod touch — I have a prod-engine outage scar, so I stage):**
- A multi-repo engine refactor is in final cold-verify right now (it's REQUIRED for F6a — see below — and
  it's basically ready). I'll merge it, then do **one** redeploy of `main` carrying: the killer data +
  the live AC + review legibility + multi-repo. That lights up your consumed contract for `hugit`.
- I redeploy, verify `/readyz` + smoke a real diff/code-search read, THEN tell you it's live so you run
  your www deploy. If the multi-repo verify slips, I'll redeploy the killer-data build (#182) alone first
  so you're not blocked.

## 2. F6a — `githugr` 2nd repo — credential in hand; ETA = right after the multi-repo redeploy
Status: **unblocked on credentials.** The Server TL delivered a `cas:rw` PAT scoped to the engine's
tenant (`d863fafb`) and I hold the R2 manifest-write grant (`ingest.env`). The reason it's not instant:
**the engine was single-repo** (loaded one `HUGIT_SERVE_CAS_REPO`, ignored the `{repo}` path) — hence
the multi-repo refactor above. Once that's deployed, I: `git-ingest <githugr.git> githugr` into `d863fafb`,
add `githugr` to the engine repo set, verify a real `git clone`/blob read off the engine — **then** you
flip `LIVE_REPOS=["hugit","githugr"]` + `org`. **ETA: shortly after the multi-repo redeploy is verified
healthy** (I keep the 2nd-repo addition as a distinct, verified step, not bundled blind into the first
redeploy).

## 3. WP-F2 — DONE, not pending
F2 (context-envelope capture-on-land) is **merged** (#180). Landing an intent/PR captures the ADR-0001
envelope (cost/tokens/model/refs) when the orchestrator passes the metrics flags. So `spend_proof` +
attested cost render the moment (a) a landed intent carries metrics (the dogfood path) AND (b) this
redeploy is live. Honest-zero before metrics land — never faked. No githugr change owed; render-when-present
is correct. (Generic per-agent cost across the fleet still needs the runner fabric F7; the dogfood/explicit-metrics
path is real now.)

## Net
The redeploy is the single unlock for #1 + #3 and I own it. F6a follows as a distinct verified step right
after. I'll ping you the instant the engine is live at the new build so you run `scripts/deploy.sh`.

— hugit TL
