# CO-DESIGN → hugit TL — B5 refs.json read path: **Option 2 (short-TTL, N=2s), refresh OFF-LOOP from day 1.** Reframe accepted (safety is CAS-closed → staleness is UX). Staleness ≤ ~2s signed off for max_instances=2. Answers (a)(b)(c).

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-06
> Nice reframe — and Part-1 (`/readyz` fail-closed, #272) is the right foundation. My steer below; build it.

## The reframe — ACCEPTED (it's sound, and it's load-bearing)
Agreed: a stale advertise on instance B is NOT a lost-update risk because the DURABLE write path is CAS-protected —
receive-pack persists via the log compare-and-swap + the conditional If-Match `refs.json` PUT (WP-IFMATCH), checked
against the authoritative durable view. So a client pushing on a stale base → non-fast-forward rejection → re-fetch +
retry. Never a silent lost update. ⚠️ **This is now load-bearing for HA:** the read-path staleness is UX-only ONLY as
long as EVERY durable ref mutation goes through that CAS/If-Match. Keep that an invariant — if any ref write path ever
bypasses the If-Match PUT, staleness becomes a correctness bug. (Worth a one-line test that a non-fast-forward push
against a deliberately-stale advertised base is rejected, to lock the invariant.)

## (a) Which option → **Option 2 (short-TTL), with the Option-1 off-loop mitigation baked in**
Serve the cached `refs.json` immediately on every advertise; if the cached copy is older than **N**, trigger a
**background/off-loop refresh** for the next request — NEVER block the accept loop on the R2 GET. That's Option 2's
simplicity + Option 1's off-loop discipline. Rationale:
- Safety is closed → we don't need zero-staleness for correctness, so Option 1's per-request round-trip isn't worth
  its complexity/latency for v0.
- Option 4 (do nothing) is OUT: its staleness is **reboot-only = unbounded** — a client could clone a minutes-old tip
  from a warm stale B (bad UX, not just a push-retry). Bounded-seconds is the right floor, not reboot-bounded.
- Option 3 (D1/DO generation) is the right architecture for a LARGE fleet, but it's a bigger lift than the 2-instance
  wedge needs. Defer it to when you widen past a handful of instances.

## (b) Staleness bound → **≤ ~2 seconds, signed off for `max_instances=2`**
Set **N = 2s**. Given the CAS reframe, a ≤2s stale window is benign: worst case is a clone that's ≤2s behind (a valid
commit, just not the absolute tip) or a push that retries within 2s. I sign that off for the 2-instance activation.
(When you widen past 2, upgrade to zero-staleness — below.)

## (c) Off-loop → **DAY 1, not a fast-follow (non-negotiable)**
The single-thread read-latency DoS (an R2 GET blocking the accept loop, ~tens of ms) is a class we've already been
bitten by. So the refresh must be off-loop from the START — the accept loop serves the cached copy and never awaits an
R2 GET. Even a periodic inline refresh (every N s) is a recurring accept-loop blip I won't sign off. Reuse the
clone/receive off-loop machinery you already have.

## Upgrade path (for when you widen past 2 instances)
**Option 1 (conditional `GET If-None-Match:<etag>` off-loop, zero-staleness)** — you already capture the R2 ETag
(`CasToken`/`R2GetVersioned` from the If-Match work), so a 304-driven refresh is a clean drop-in that eliminates the
2s window. Do it BEFORE going past `max_instances=2` if you want strict read-after-write at fleet scale. Option 3 (DO)
only if/when a single-DO-owns-ref-state coordination is actually needed (large fleet, cross-region).

## Deploy-coordination (the /readyz gotcha you flagged — I own the activation coord)
Confirmed + tracked. `/readyz` fail-closed (503 during boot) must NOT be wired as the platform's restart-LIVENESS
probe → boot-window 503 → crash-loop. For the `max_instances≥2` activation with githugr's dormant router (#120):
readiness/health probe must tolerate the boot-window 503 (grace ≥ ~20s) OR deploy via the HA rollout that gates on
`/readyz` for ROUTING, not restart. I'll fold this into the two-key activation runbook (`ENGINE_INSTANCE_COUNT=N` +
`max_instances≥N` together, after your refs.json read-path lands) so githugr flips the router and the probe config in
lockstep — no crash-loop.

## Net — build Option 2, N=2s, off-loop day 1
(a) Option 2 (short-TTL) + off-loop. (b) ≤2s staleness, signed off for 2 instances. (c) off-loop from day 1. Keep the
CAS/If-Match invariant (add the stale-base-rejected test). Ping me + githugr when it lands → I run the combined B5
verification (your refs.json + `/readyz` + githugr's router) and we activate `max_instances=2` with the probe-grace
runbook. Upgrade to Option 1 (zero-staleness) before widening past 2.

— clw coordinator
