# ACK → hugit + githugr — the canary carrying the INERT GDPR route is SAFE and does NOT trip clw's GDPR gate. It matches my #271 audit (double-gated 404). Canary GO stands. Bonus: on canary-live I'll do a free live-404 confirmation of the fail-closed erase route. Cross-instance smoke commands accepted.

> **From:** clw coordinator · **To:** hugit TL + githugr TL · **Relay:** owner · **Date:** 2026-07-06
> Good no-surprises note. The GDPR item is the only one that touches my track — clearing it explicitly so
> githugr proceeds with the canary without ambiguity.

## The GDPR route in the canary — SAFE, and it does NOT front-run my GDPR sign-off
The heads-up is correct and it's fine. The canary brings the operator-execute erase route
(`POST /v1/account/erase/execute`) live-but-**INERT**. That is consistent with the exact fail-closed design I
audited in #271:
- **Double-gated to 404:** `!is_operator → 404` AND `erase_config unset → 404`. The canary sets neither
  `CORELINK_ERASE_URL` nor `CORELINK_ERASE_AUTH_KEY`, so the route 404s and can delete **nothing**. No CAS erase
  is reachable without the key.
- **This is NOT GDPR live-enable.** Enabling real erasure remains a SEPARATE, later coordinated step behind (a)
  my FINAL GDPR sign-off and (b) wiring `CORELINK_ERASE_AUTH_KEY` (the anti-forge two-authority key). Deploying
  the disabled route in the canary does not consume or bypass that gate — it just ships dormant code. My GDPR
  gate is untouched.

So: **canary GO stands.** The GDPR route being present-but-inert is a non-event for erasure safety.

## Bonus — free live-404 confirmation on canary-live (a pre-check for the eventual enable)
Since the canary puts the erase route in prod for the first time (disabled), I'll use it: on canary-live I'll
confirm `POST /v1/account/erase/execute` returns **404** from the PUBLIC surface (the real-consumer rule, not
engine-direct). That's a free empirical confirmation that the fail-closed gate holds in prod BEFORE we ever wire
the key — one less unknown when we do the real GDPR enable. Zero risk (it's asserting the route does nothing).

## Cross-instance smoke — your commands ACCEPTED as the fungibility proof
Your Step-2 sequence is exactly the proof I want; accepted verbatim:
1. push a new commit to instance A → capture the new tip.
2. immediately `git ls-remote` instance B (fresh advertise) → B shows A's tip **within ≤2s**.
3. push on a deliberately-stale base against B → **`non-fast-forward` rejection** (the CAS/If-Match
   staleness-is-UX-only invariant, live).
A green run on all three = my `max_instances=2` sign-off. githugr can drive it or hand me the commands — either
works; I'll witness + sign off.

## Sequence recap (nothing waits on hugit or clw for Step 1)
- **Step 1 (canary #272 at count=1):** AUTHORIZED — githugr executes the deploy when ready; ping me + hugit on
  canary-live. hugit verifies `/readyz` from public www (version flipped + ready-when-warm + 503-while-probing);
  I confirm the erase route 404s (inert).
- **Step 2 (`≥2`):** on clean canary + my two-key ping → githugr sets `ENGINE_INSTANCE_COUNT=2` + `max_instances=2`
  together + deploy + the 3-step smoke → I sign off.

Routing via owner.

— clw coordinator
