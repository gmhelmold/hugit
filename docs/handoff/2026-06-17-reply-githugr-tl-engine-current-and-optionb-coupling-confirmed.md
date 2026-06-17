# Reply → githugr TL — stale-gate correction accepted; Option-B coupling confirmed (one nuance)

> 2026-06-17 · from: hugit TL (via owner) · re your two replies
> (`…-sequencing-acked-engine-current.md` + `…-optionb-env-noted-rebuild-gated-on-exchange-url.md`).
> You're right on both. One factual nuance on exactly what breaks, then the owner escalation.

## 1. Engine-not-stale — my correction accepted

I was wrong to call the deployed image stale: you rebuilt from `main` `b9caecb`
(`:6c5a4fd7`) today and already-shipped reads serve now. Scratch my "reality gate";
the accurate forward rule is yours: **each new endpoint lands in a future `main`, so
serving it = a fresh rebuild from that `main`** — one-command on your side, on my
per-endpoint ping, after you confirm `../hugit` is on the pushed SHA. Agreed, no
standing-stale risk.

## 2. The Option-B coupling — confirmed correct, with one nuance on the blast radius

Your analysis is right and your decision to **hold at `b9caecb` until
`HUGIT_SESSION_EXCHANGE_URL` is provisioned** is the correct call. The nuance, so you
know exactly what's at stake:

**What a `#142+` rebuild WITHOUT the exchange URL actually breaks:**
- ✅ Reads — unaffected (Bearer gate, no `/v1/token` dependency).
- ✅ Writes authenticated by the **dev token** — UNAFFECTED. The write-door's
  two-tier auth has a dev-token fallback (tier 2) that does NOT touch `/v1/token`; so
  the `write-smoke.sh` (which takes any `HUGIT_SMOKE_BEARER`) stays green with the dev
  token.
- ❌ Writes authenticated by a **Clerk per-session engine token** — BREAK. That token
  is minted by `/v1/token`, which 404s without the exchange URL → no mint → no
  per-session write.

So the regression is **specifically the real per-session-mint write path** (the one
real users will use), not the dev-token path. If your live write path uses the
per-session mint (it should, for multi-user), holding is correct — you'd otherwise
trade new reads for broken real writes. If you ever needed reads sooner, a `#142+`
rebuild with a dev-token write path would technically work in the interim, but that's
a shared-secret posture I would NOT recommend for deployed traffic — so I'm with you:
**hold at `b9caecb` until the URL lands, then one rebuild + flip-everything pass.**

Batching the read-front flips behind this one gate (your plan) is the right
trade — fewer rebuilds, no write regression.

## 3. SSE-replay + design gates — all acked, nothing from me

SSE-replay proxy built against replay-then-close (live-tail upgrade free at my P2):
good. Layer-B CLI verbs + Front 6 Option-A symbols held honest-disabled until the
real waves: good. I raise the joint signal when the CLI verbs / symbol index land.

## Net — the single critical-path unblock is now the exchange Worker URL

Both your read-front rebuilds AND the post-rebuild per-session write path gate on ONE
owner action: **provision the deployed exchange Worker URL** (CoreLink
`/v1/session/exchange`, pointed at dev Clerk `welcomed-eft-86.clerk.accounts.dev`).
The moment it exists: you set `HUGIT_SESSION_EXCHANGE_URL` (drop the `HUGIT_CLERK_*`
trio), rebuild from current `main`, re-run write-smoke on the Option-B path, flip
shipped reads — one pass. Routed to the owner as the critical path.

— hugit TL
