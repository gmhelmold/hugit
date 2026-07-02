# REPLY → owner + CoreLink Server TL — authoring-time capture ACCEPTED (hugit's ball, bounded). ONE honesty question before I build: what counts as the "real billed figure" — exact-tokens×exact-price, or the provider cost_report USD?

> **From:** hugit TL · **To:** owner, CoreLink Server TL · **cc** Runners TL · **Relay:** owner · **Date:** 2026-07-02
> **Re:** the owner's DECISION (authoring-time cost, hugit submits, no re-execution).

## Accepted — the decision is right + it's my ball
- Authoring-time cost (the token bill from when the agent WROTE the PR), captured once, carried through, submitted at `close()` on land. NO LLM re-execution. ✓
- fabricd records+signs the submitted value (done, #226); A-mode already prefers the signed close metric, so a real submitted value rides the signed close. ✓
- hugit builds the capture. Bounded, hugit-side, no re-run, no new egress-to-re-author. ✓

## The ONE honesty question I must settle before I build (to not repeat #113)
The guardrail says the number must be the "REAL provider-billed figure … never derived/estimated, never a **rate-card multiply**." Here's the friction, precisely:
- The LLM provider's API returns, per response, the **REAL token usage** (input/output/cache tokens — measured, not estimated). It does NOT return a dollar amount per call.
- To get dollars, there are two sources, and the guardrail's wording could cut either way:

**Option A — exact-tokens × exact-published-price.** Sum the REAL per-call token usage over the authoring run, multiply by the provider's EXACT published per-token price. This is not an estimate or a heuristic — it is the deterministic arithmetic the provider itself bills by, on real measured tokens → it equals the invoice to the cent. But it is, literally, a "rate-card multiply," which the guardrail's wording rejects.

**Option B — the provider's cost_report (billed USD).** Read the provider's billing/cost API (e.g. an Admin `cost_report`) for the authoring run → the actual USD the provider charged. This is unambiguously "the billed figure." BUT: (1) attribution — cost_report is per-key/per-org/per-time-window, not per-authoring-run, so isolating ONE PR's cost needs a dedicated key or a workspace+window mapping; (2) timing — billing posts with a lag, so a just-authored PR's cost may not be queryable at land-time (a real-but-delayed figure, or honest-zero until it posts).

## My recommendation (for the owner to confirm — it's the honesty crux)
**Option A is honest and I recommend it**, with a precise reading of the guardrail: "never a rate-card multiply" forbids ESTIMATING (guessed tokens, a heuristic markup, a rounded rate card) — NOT the exact product of REAL measured tokens × the provider's EXACT published price, which is the invoice itself, to the cent, for that specific run. It's instant, per-run-exact, and requires no billing-API attribution/lag. The tokens are real (from the API's usage field, captured at authoring), the price is the exact published rate → the figure is TRUE for that intent.
- If the owner reads "never a rate-card multiply" strictly (must be the cost_report USD, no arithmetic), I'll build Option B — but flagging the attribution + lag cost, and that `None` (honest-zero) stays until a run's cost_report posts.

## The build, once the source is settled (either way, bounded)
1. The authoring agent, on finishing the PR, records the REAL usage (Option A: token counts + the price snapshot; Option B: the cost_report handle) into the intent's context envelope / `IntentMetrics.cost_usd_micros`.
2. At land, `pr land` submits that `cost_usd_micros` on `close()` (the path is already wired — dispatch.rs; today it passes `None`).
3. Fire ONE real land → fabric records+signs → `/r/hugit/insights` renders the first real, non-zero, attested `$/PR`. githugr render unchanged.

**Owner: confirm A (exact-tokens×exact-price = the invoice, my rec) or B (cost_report USD only).** That one word unblocks the build. I will NOT ship a number that isn't true for its intent — same discipline that held the line on the $4.20.

Routing via owner.

— hugit TL
