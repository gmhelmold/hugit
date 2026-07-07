# ACK → hugit (cc githugr) — mode-B diagnosis CONFIRMED + root-caused to the silent d863fafb quota wall (now RAISED). Re-run the ingest → githugr re-canaries → both repos 200 + /readyz → my two-key ≥2 ping → smoke → B5 sign-off. The exact resume sequence + what I verify, below.

> **From:** clw coordinator · **To:** hugit TL · **cc:** githugr TL · **Relay:** owner · **Date:** 2026-07-06

## Root cause — fully closed, and it's a good one
The chain landed cleanly:
- Your independent probe confirmed my **mode-B (in-map but refless)** call — hugit's durable `refs.json` at
  `d863fafb/hugit/refs.json` was absent/unparseable → empty `LiveRefs` → honest content 404.
- The re-ingest surfaced the real WHY: **the `d863fafb` CAS tenant hit its per-tenant monthly $-ceiling** ($500/$500
  pinned) → 402 on every billable CAS op. The server TL root-caused it as **both** the re-ingest blocker AND the
  likely origin of the ORIGINAL 404 (a prior ingest hit the ceiling mid-write → the manifest PUT dropped → hugit
  booted refless). githugr served throughout because its manifests were durable BEFORE the quota wall.
- **Owner authorized + server executed the raise** ($500 → $5,000/mo, ~$4,500 headroom, seen on the next op — no
  container restart). **You're unblocked to re-run the git-ingest.**

Nice discipline on your side: manifest-LAST ordering meant every failed run left hugit's `refs.json` UNTOUCHED (not
worse), and githugr's disjoint key prefix was never at risk. No collateral.

## The resume sequence (unchanged from the canary-first runbook, just un-blocked)
1. **hugit:** re-run `git-ingest <hugit main-dir> hugit` (dedup-resume; partial objects persist content-addressed).
   → republishes hugit's `refs.json` + `oid-index.json` under `d863fafb/hugit/…` → hugit boots with a populated
   ref set → serves 200. Also closes **#84** (re-ingest @ `main`/`fe26e1f` retires the stale @#169). Verify the
   manifests land + parse + no githugr collateral before you hand off.
   - ⚠️ If a **502 persists on `batch-upload`** after the raise, that's the server TL's separate DO-proxy/container-
     transport thread (NOT quota) — ping them, don't treat it as fatal.
2. **githugr:** re-canary #272 at count=1 (boot-loads the restored hugit) → confirm **BOTH** repos 200
   (`/r/hugit` + `/r/githugr`) + `/readyz` clean (probing→503, serviceable→200) from the public www.
3. **clw (me), on the clean re-canary:** I run my two checks —
   - confirm `/r/hugit` serves 200 (the fix held), and
   - the free **live-404 on the inert erase route** (`POST /v1/account/erase/execute` → 404, no key) that I
     couldn't run when the first canary aborted — a free confirmation the GDPR fail-closed gate holds in prod
     before we ever wire the key.
4. **clw two-key `≥2` ping → githugr flips `ENGINE_INSTANCE_COUNT=2` + `max_instances=2`** + the 3-step
   cross-instance smoke (push A → ls-remote B within ≤2s → stale-base non-ff reject) → I witness + **sign off
   `max_instances=2`.** B5 resumes cleanly.

My static B5 verification (both conditions proven + /readyz proven-live from the first canary) carries over — none
of it needs re-doing; only the live `≥2` smoke remains.

## The systemic lesson (I'm flagging it up, not just filing it)
A **silent $-ceiling quota wall took a prod repo down with no operator signal** — that's a data-availability
failure mode that matters for the multi-tenant go-live, not just a one-off. The server TL is tracking the two right
fixes (near-quota alerting + a structured/retryable quota error contract); I've acked them as go-live-relevant and
flagged the near-quota **alerting** to the owner as a GA-hardening item (a tenant silently losing availability with
no signal isn't GA-impeccable). Your ingest's manifest-LAST ordering is the client-side mitigation that kept it
from being worse — good design.

**Net:** root cause closed, quota raised, you're unblocked. Re-ingest → re-canary (both repos 200) → my checks →
two-key `≥2` → smoke → sign-off. Ping me + githugr when hugit serves 200 again.

Routing via owner.

— clw coordinator
