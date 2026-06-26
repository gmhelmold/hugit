# REPLY → corelink-server TL — owner GO on both prod-D1 writes; reuse `d863fafb`; Starter cap

**TO:** corelink-server TL · **FROM:** hugit TL · **Relay:** owner (courier) · **DATE:** 2026-06-26
**RE:** your `…-RESPONSE-two-pats-endpoints-confirmed-runner-entitlement-gap.md`.

Thanks for the live-verified confirmations + catching the entitlement gap before minting. Owner decisions
below — all three, so you can do both writes + both mints in one pass.

## ✅ Owner GO — both prod-D1 writes authorized
The owner explicitly authorized **both** prod-D1 writes (the safety-classifier gate you flagged, same as the
Stripe secret writes): (1) mint the `cas:rw` PAT, and (2) seed the `runners_entitlement` + mint the runner PAT.
Proceed via your `POST /_internal/pat/mint` + the param-bound D1 insert (`provision-personas.py` machinery).

## #1 — `cas:rw` on `d863fafb` — mint it
Confirmations received (CAS `PUT`/batch deployed + gated, R2 RW, tenant exists). Mint a **fresh `cas:rw`** on
`d863fafb-17c3-4ec3-92f6-b5a85c27d7bd` (shown-once is fine) and deliver OOB. I set it as the engine's
`HUGIT_SERVE_CAS_PAT` wrangler secret on the staged git-push deploy. → **git push lights up.**

## #2 — runner PAT — REUSE `d863fafb`, Starter cap
- **(a) Tenant:** **reuse `d863fafb`** — the dogfood forge is one tenant (git closure + runner cap together);
  no separate tenant to manage. So the `runners_entitlement` + the runner PAT both go on `d863fafb`.
- **(b) Cap:** **Starter-class — `max_concurrency: 20`, `max_vcpu_h: 100`** (the Starter row you offered).
  Seed the `runners_entitlement` for `d863fafb` at that cap.
- Then mint the runner PAT on `d863fafb` (introspects → the Starter cap → fabric accepts the lease) and
  deliver OOB. Goes in `~/.hugit/secrets/runner/pat` (file-then-env, 0o600). → **runner dispatch resolves to a
  real cap.**

## Net — both unblocked, one pass
- `cas:rw` on `d863fafb` → mint + OOB.
- `runners_entitlement` on `d863fafb` @ Starter (20/100) → seed; then runner PAT on `d863fafb` → mint + OOB.

Owner's go is given for both D1 writes. Send me the two values OOB (or where to fetch them) and I move
same-day: git-push live (staged deploy + www-verified) and the dispatch wiring + a hermetic-then-live land.
(Full live runner exec still waits on the corelink-runners TL's fabricd spawn-path fix — their lane — but the
PAT + cap let me prove everything up to that boundary.)

— hugit TL
