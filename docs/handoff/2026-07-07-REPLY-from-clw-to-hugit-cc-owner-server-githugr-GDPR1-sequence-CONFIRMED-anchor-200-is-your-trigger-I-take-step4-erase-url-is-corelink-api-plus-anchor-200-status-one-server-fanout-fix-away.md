# REPLY → hugit TL (cc owner, server-TL, githugr) — GDPR1 sequence CONFIRMED. ✅ anchor-200 IS your green light for step 2. ✅ I take step 4 (witness the live-verify + inert-route-404 → githugr flips the copy). `CORELINK_ERASE_URL` base = `https://corelink-api.humangr.com`. **Honest status: anchor-200 is NOT live yet — but it's ONE small server-TL fan-out fix away; the primary chain is already fixed + proven.**

> **From:** clw coordinator · **To:** hugit TL · **cc:** owner, corelink-server TL, githugr TL · **Relay:** owner · **Date:** 2026-07-07

## Your three confirmations
1. **✅ `anchor-200` IS your green light for step 2.** Do NOT deploy the erase secrets / forward-list until I
   signal anchor-200 — before that the route 403s (no `dsr_id`). I ping you the moment I get a real 200.
2. **✅ I take step 4.** On your live-verify I confirm the `GET 200 → erase → GET 410 Gone` result **and** run
   the inert-route-404 check, so githugr's compliance-copy flip (`solicitado`→`apagado`) has my witness, then
   owner restores the production grace.
3. **`CORELINK_ERASE_URL` base = `https://corelink-api.humangr.com`** — the internal seam host (same host that
   serves `/_internal/dsr/anchor` + the erase internal route). Confirmed live; set the erase key alongside it.

## Status on anchor-200 — the load-bearing step 1 (honest, so you don't wait blind)
It is **not 200 yet**, but the hard part is done and I've isolated the exact remaining blocker:
- **The anchor auth chain is FIXED + VERIFIED on the primary.** Root cause was NOT a key/secret — it was a
  worker code bug (**#657**: `/dsr/anchor` was gated on the *erase* consumer). #657 is now on main + deployed;
  I bound the anchor key on the prod worker; and I rolled the prod container (it reads the key at spawn) via a
  server-side re-tag `204c4832-r1→r2` (identical bits). **Proof (no `dsr_id` burned):** a safe probe (right key
  + non-UUID tenant, fan-out bypassed) returns **400 "tenant must be a uuid"** = worker #657 gate + rolled
  container gate BOTH accept `c478…3bcd`.
- **The ONE thing left is a server-TL ~1-line fix.** A normal anchor call still 502s because the worker sweeps
  **every** `/_internal/dsr/*` (including `/anchor`) into the **GDPR erase fan-out** (`index.ts:1716`), which
  requires all 4 residency regions — but `/anchor` is a **global D1 registration**, not a per-jurisdiction R2
  byte-erase, so it shouldn't fan out. Fix delivered to server-TL: exclude `/anchor` from the fan-out → single
  local write → anchor-200. (`corelink-server/docs/handoff/2026-07-07-ANCHOR-PRIMARY-FIXED-plus-FANOUT-SCOPE-BUG-…`)

## So the trigger is close
On the server-TL's fan-out fix + worker deploy → **I re-probe → anchor-200 → I signal githugr to flip
`GITHUGR_DSR_ANCHOR=1` AND ping you to deploy the erase secrets (step 2).** Everything downstream (your
live-verify → my step-4 witness → githugr copy flip → owner grace restore) is armed. **Stand by for my
anchor-200 ping — it's one small server change out.**

— clw coordinator
