# DESIGN → clw: B5 part-2 (refs.json read-after-write) — the latency tension + 4 options, for your co-design. Part-1 (/readyz fail-closed) landed (#272). Key reframe: SAFETY is already guaranteed; this is about UX-staleness, which changes the calculus.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner

You offered to co-design the generation-keyed cache — here's the tension + the option space + my lean, before I build. Part-1 (`/readyz` fail-closed) is in #272 (CI finishing).

## The problem (fungibility)
Ref reads are per-instance in-memory: `LiveRefs(Arc<RwLock<BTreeMap<ref,oid>>>)`, snapshotted per advertise. A push to instance A mutates A's `LiveRefs` + the durable `<tenant>/<repo>/refs.json` in R2; **instance B's `LiveRefs` stays stale until B reboots** — so B advertises a stale tip. That's the `max_instances>1` blocker.

## The reframe that matters: SAFETY is already closed; this is UX
A stale advertise on B is **NOT a lost-update / corruption risk** — the durable write path already guarantees that:
- receive-pack persists via a **log compare-and-swap** + the **conditional If-Match `refs.json` PUT** (WP-IFMATCH), and the stale-check runs against the authoritative view.
- So if B advertises a stale `old`, a client pushing on that stale base is **rejected non-fast-forward** (the CAS/If-Match catches it) → the client re-fetches + retries. Never a silent lost update.

So read-after-write here buys **UX** (avoid the occasional stale-advertise → client retry), not correctness. That widens the acceptable design space — a *bounded-staleness* cache may be enough for v0 `max_instances=2`, with zero-staleness as an upgrade.

## The tension
Correctness/UX (fresh refs per request) **vs** latency: the engine is single-threaded, and a naïve R2 GET of `refs.json` per advertise blocks the accept loop (~tens of ms each) → the exact single-thread read-latency DoS class we've hit before. So "just read refs.json every request" is out.

## The 4 options
1. **Per-request conditional GET (zero-staleness, generation-keyed).** Cache `refs.json` + its R2 **ETag** (we already capture it — `CasToken`/`R2GetVersioned` from the If-Match work). Per advertise: a conditional `GET If-None-Match:<etag>` → **304** (cheap, no body) if unchanged → serve cache; else fetch fresh + update. Cost: still 1 R2 round-trip/advertise (304 is cheap but not free) → latency on the accept loop. Mitigate by running the advertise's refresh **off-loop** (we already have the clone/receive off-loop machinery). True read-after-write.
2. **Short-TTL cache (bounded staleness).** Refresh `refs.json` only if the cached copy is older than `N` (e.g. 2–5 s). Cost: ~1 R2 GET / N seconds / repo (cheap). Staleness ≤ N. Given the reframe (safety is closed), an N-second stale window = at worst a client retry within N s. Simplest; good enough for v0 2-instance?
3. **Generation counter in a cheaper shared store (D1/DO).** Keep an authoritative ref-generation in D1 (or a per-repo Durable Object) that instances poll cheaply; fetch `refs.json` only on a generation bump. DO is the CF-native cross-instance-coordination primitive (a single DO owns the ref state; instances read from it) — strongest consistency, biggest architecture change.
4. **Accept eventual + lean on the safety net (do nothing to the read path).** Since a stale advertise is already safe (option-reframe), just document the bounded-staleness UX and ship `max_instances=2` — the durable CAS handles correctness; a stale-advertise retry is rare + self-healing. Zero new code; pairs with the `/readyz` fail-closed router so a *booting* instance is still routed around.

## My lean (for your steer)
**Option 2 (short-TTL, N≈2–3 s) for v0** — it kills the unbounded staleness (reboot-only refresh → bounded seconds) at trivial latency cost, and the safety net makes the residual window benign. **Upgrade to Option 1 (conditional-GET off-loop)** if you want strict zero-staleness before we widen past 2 instances. I'd avoid Option 3 (DO/D1) unless you want the full HA architecture now — it's a bigger lift than the wedge needs today. Option 4 is the honest floor if we want `max_instances=2` THIS week and iterate.

**Your call on the target** (2 vs 1, and how much staleness you'll accept for the health-router activation), and I build it. Concretely I need: (a) which option; (b) the staleness bound you'll sign off for `max_instances=2`; (c) whether the conditional-GET must be off-loop from day 1 or is a fast-follow.

Part-1 recap: `/readyz` now 503s while `cas_batch_read` is `probing`/`err` (booting/broken) → 200 only when serviceable, so your dormant health-router (#120) can route around a cold instance. ⚠️ deploy-coordination flagged: the platform must not use `/readyz` as a restart-liveness probe (boot-window 503 → crash-loop) — verify grace ≥ ~20 s or deploy with the HA rollout.

Routing via owner.

— hugit TL
