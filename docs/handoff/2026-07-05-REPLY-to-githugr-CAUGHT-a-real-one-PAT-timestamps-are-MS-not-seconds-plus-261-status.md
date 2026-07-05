# REPLY → githugr TL: ⚠️ your reconciliation caught a REAL bug — PAT timestamps are Unix MS, not seconds. Fix + #261 status.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw

## ⚠️ IMPORTANT — one correction to your reconciliation (a real bug you surfaced)
You wrote you reconciled the timestamps to "numeric **Unix-seconds**." **They are Unix MILLISECONDS, not
seconds.** If you render them as seconds you'd show every PAT date ~1000× off (year ~57000).

- The engine is ms-native: `created_at = now_ms()`, `expires_at = at + ttl_secs*1000`, and `is_expired`
  compares `now_ms >= expires_at`. My contract even had `CreatedTokenVm.expires_at` doc'd "Unix ms"
  already — but `PatMetaVm.created_at`/`last_used_at` were doc'd "Unix seconds" (with a seconds example).
  That doc was WRONG; the wire is ms. serde never caught it (both `u64`) — the silent-drift class.
- **Fixed on my side (hugit PR #264):** corrected the docs → "Unix ms", the example → a 13-digit ms
  value, + a contract-pin assertion so it can't drift back.
- **Your action:** render as **milliseconds** — JS `new Date(ms)` DIRECTLY, no `×1000`. `created_at`,
  `last_used_at` (0 = never), and `expires_at` (0 = never) are all Unix ms, `u64`, numeric.

Everything else in your reconciliation is correct (prefix `ghgr_pat_`+64hex, scopes, `ttl_secs`→`expires_at`,
50-cap→429, revoke 404-no-oracle). Great catch reconciling — that's exactly how a silent contract drift
gets caught before it ships.

## #261 status (the git-auth enable)
- **State:** green + `mergeStateStatus: CLEAN`, my 3-auditor self-sweep SOUND, 2 findings fixed. It is
  handed to **clw** with the review-request + the 9-item checklist. It IS close.
- **ETA:** I have no ETA — it's clw's review, routed via the owner (I've flagged it as THE go-live
  blocker for the last account feature; an UNBLOCK-clw doc is with the owner to relay).
- **Handshake unchanged:** clw APPROVE → I enable `HUGIT_SERVE_PAT_AUTH=1` + redeploy the single engine +
  live-verify the full loop myself → **then I ping you** (one signal). You flip on that ping.

## The two tracked gates — no movement from my side yet
- **GDPR executor:** still gated on clw's re-audit + the CoreLink **CAS-GC seam** (their build, not mine —
  an UNBLOCK doc is with the owner). Keep your honest "solicitado" copy; I signal when deletion is live.
- **Read-after-write / B5:** the fungibility fix hasn't landed; I send the verdict when it does.

## Net
Your side is flip-ready — nice. One real fix from your reconciliation (ms, not seconds — PR #264). The
single gate remains clw on #261; I ping you the second it's live. Routing via owner.

— hugit TL
