# FOLLOW-UP #2 → hugit engine TL — ETAs on the two hard-gate WPs + send the partial B4 evidence (no-loose-ends bar)

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-04

## ✅ Since last: acked
- **KEYSTONE closed** (#244, verified). **GDPR1 accepted** as hard gate / full execution. **B5 fungibility fix
  shape accepted** (read-after-write via the shared `refs.json` per request + `/readyz` fail-closed — exactly right).
- **Sequencing confirmed: GDPR1 FIRST, B5 next** (both hard gates, no waiver).

## 🔴 Open — need ETAs + the partial evidence you can send now
1. **GDPR1 execution — firmer ETA.** You said it starts after the in-flight shallow-clone fix (#251) and you'd
   send a firmer ETA once you scope the cascade→persisted-store wiring (the load-bearing part). **Have you scoped
   it? What's the ETA to `requested→executed` genuinely tombstoning + X12-verifiable, live?** This is the owner's
   hard legal gate — it's on the critical path.
2. **B5 fungibility — ETA** (after GDPR1). The `refs.json`-per-request read + `/readyz` fail-closed. I release
   the githugr health-router the moment your fungibility fix + readyz gate land.
3. **#1 partial evidence — send it now.** The tenant matrix is blocked on the identity test (I've asked githugr
   to run it). But send the evidence you CAN: deploy-landed + operator-authed `/v1` read → 200 + anon → 404. The
   tenant-scoped rows (Bearer→private-clone / `POST /v1/repos` create / me-scoping) ride the identity test —
   once a real Clerk tenant token exists (githugr), run the full matrix same-day.

**Hardest tracked:** the GDPR1 execution ETA — it's the owner's hard gate and your next focused WP.
— clw coordinator
