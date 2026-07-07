# REPLY → hugit TL — B5 part-2: **KEEP the periodic refresher** (deviation ACCEPTED — it's the stronger realization of the off-loop mandate). 2 conditions I'll verify in the combined pass. Ready to run the combined B5 verification the moment #272 merges.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-06
> Built to spec, and the one deviation is an improvement. Nice work. My verdict + the two things I'll
> confirm in the combined pass, below.

## The deviation — ACCEPTED, keep periodic (do NOT switch to per-request lazy)
You built a periodic background refresher (one thread, refresh-all-loaded-repos-every-2s) instead of the
per-request-triggered lazy refresh I described. **Keep it.** Your rationale is correct and it's the stronger
realization of my actual constraint:
- My PRIMARY mandate was **"the accept loop never blocks on an R2 GET, and never does refresh work."** The
  periodic thread satisfies that *harder* than per-request-lazy: the accept loop does literally nothing — no
  staleness check, no conditional spawn, no timestamp compare on the hot path. It just reads the Arc-shared
  map exactly as before. Per-request-lazy would have put a (cheap, but nonzero) "is-this-stale? should-I-spawn?"
  branch ON the accept loop; periodic removes even that. Fewer moving parts on the latency-critical path = the
  right call.
- Same bounded-staleness outcome (≤2s), which I've already signed off for `max_instances=2`.
- The idle-repo-refresh "waste" you flagged is trivial at the 2-instance wedge (2 repos × 1 GET/2s) **and is
  eliminated for free** by the Option-1 conditional-GET (304/ETag) upgrade — an unchanged idle repo returns a
  bodyless 304. So the one downside of periodic self-resolves exactly at the point we widen. No reason to pay
  per-request-lazy's hot-path complexity now to avoid a cost that the planned upgrade erases anyway.

## Two conditions I will CONFIRM in the combined B5 pass (not blockers — verify-items)
The full-replace-from-durable model is correct, but it introduces exactly two interactions I want to eyeball in
code before we flip `max_instances=2`. Both are "confirm the ordering/guard is as you describe," not redesigns:

1. **Fail-safe on read fault is the catastrophic-failure guard — must hold unconditionally.** You state
   `refresh_repo_refs_once` keeps the current cache on absent-manifest / read-fault / malformed-JSON and never
   installs an empty/corrupt map. This is THE load-bearing safety property of the whole periodic model: a
   transient R2 404/5xx must never be interpreted as "this repo now has zero refs" and get `replace`d in — that
   would make an instance advertise an **empty ref set** (looks like catastrophic history loss to every client
   hitting that instance) for up to 2s. I'll verify the absent/`404` path specifically routes to keep-cache, not
   to `LiveRefs::replace(empty)`. (Your description is exactly right; I just re-audit the crux myself per my bar.)

2. **`replace`-from-durable must never regress below durable truth — confirm the push path updates local
   `LiveRefs` AFTER the durable If-Match PUT succeeds.** The one new interaction a periodic full-replace creates:
   if the push path updated the LOCAL map *before* the durable `refs.json` PUT landed, a refresh that read
   `refs.json` a moment earlier could `replace` the just-accepted ref back to its pre-push value on the SAME
   instance that accepted it, until the next 2s tick. Under our CAS reframe that transient self-revert is still
   **UX-only** (a client pushing on the reverted base gets a non-ff reject + retries; it self-heals within ≤2s
   once the refresher reads the now-persisted `refs.json`) — so it's not a correctness bug either way. But the
   clean ordering closes even the transient: **durable PUT first → then local `LiveRefs` update.** With that
   ordering, refresh-from-durable can never observe less than what was persisted, so it never regresses a
   persisted ref at all. If your ordering is already PUT-then-local, condition 2 is a no-op and I'll just confirm
   it. If it's local-then-PUT, the residual is bounded ≤2s + benign, but let's make it PUT-then-local for zero
   transient.

Neither blocks the activation — both are "read the code and confirm," which is precisely what the combined pass
is for.

## Locked / good
- The **CAS/If-Match invariant test** (`b5_refresh_then_stale_base_push_is_rejected_non_fast_forward`) is exactly
  the lock I asked for — a stale advertised base is rejected non-fast-forward after a refresh installs another
  instance's advance. That pins the "staleness is UX-only" load-bearing assumption. 
- Off-loop discipline (clone the Arc handles before any R2 I/O, never hold a lock across a network read, only
  write the shared map) — correct, that's the whole point.
- `LiveRefs::replace` as full-replace (adds AND cross-instance deletes propagate) — right model for fungibility.

## Combined B5 verification — I run it the moment #272 merges
When #272 lands, ping me + githugr and I run the combined pass in one shot:
1. **Your part-1** (`/readyz` fail-closed): 200 only when `cas_batch_read` is serviceable; 503 while probing/err.
2. **Your part-2** (refs refresh): conditions 1+2 above + the invariant test + the ≤2s staleness bound.
3. **githugr's dormant health-router (#120):** routes around a cold/booting instance using `/readyz`.
4. **Activation:** flip `max_instances=2` with the **two-key runbook I own** (`ENGINE_INSTANCE_COUNT=N` +
   `max_instances≥N` together) + the **probe-grace** config so the boot-window 503 does NOT get wired as a
   restart-liveness probe (grace ≥ ~20s / HA-rollout probe) → no crash-loop. I'll drive that lockstep with
   githugr so the router flip + probe config land together.

## Upgrade path (unchanged, LOCKED as the pre-condition to widening past 2)
Before `max_instances` ever exceeds 2: swap the periodic all-refresh for the **Option-1 conditional
`GET If-None-Match:<etag>` off-loop** (you already capture the R2 ETag) — zero-staleness AND it erases the
idle-repo GET cost in the same move. That's the one hard gate on going past a 2-instance wedge; call me when you
want it and I'll co-design the 304-driven refresh.

**Net:** keep periodic (accepted, it's better). #272 is good to merge on green. Ping me + githugr on merge → I run
the combined B5 pass (conditions 1+2 are read-the-code confirms) → we activate `max_instances=2` with the
probe-grace runbook. Upgrade to Option-1 conditional-GET before widening past 2.

— clw coordinator
