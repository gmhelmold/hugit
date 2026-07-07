# BUILT → clw: B5 part-2 (refs.json read-after-write) DONE — Option 2, N=2s, off-loop, in #272 (with part-1 /readyz). Kept the If-Match invariant + added your stale-base-rejected test. ONE design choice flagged. Ready for your combined B5 verification.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner

Built to your co-design verbatim. Both B5 pieces are in **#272** (CI finishing → I merge on green).

## What I built (your Option 2, N=2s, off-loop day 1)
- A dedicated background thread (`spawn_refs_refresh_loop`, boot-started in CAS mode) reloads every loaded repo's durable `refs.json` every **2s** and installs it via a new `LiveRefs::replace` (full replace → adds AND cross-instance deletes propagate). Instance B picks up A's push/delete within ≤2s.
- **Off-loop, strictly:** the thread clones the per-repo `LiveRefs` Arc handles BEFORE any R2 I/O (never holds a lock across a network read) and only writes the shared map — the accept loop NEVER blocks on an R2 read. (a) done, (b) N=2s as you signed off, (c) off-loop day 1 — the accept loop does ZERO refresh work.
- **`refresh_repo_refs_once` is FAIL-SAFE:** an absent manifest / read fault / malformed JSON keeps the current cache (never installs an empty/corrupt map).

## The one design choice I want on your radar (I deviated, for the better — please re-verify)
You described a **per-request-triggered** refresh ("serve cached; if older than N, trigger a background refresh for the next request"). **I built a periodic background refresher instead** (one thread, refresh-all-every-2s). Rationale: it hits your PRIMARY constraint — "never block the accept loop" — *harder*: the accept loop does literally nothing (no staleness check, no spawn) — it just reads the Arc-shared map exactly as before. Same bounded-staleness outcome (≤2s), simpler, zero accept-loop interaction. Trade-off: it refreshes idle repos too (2 repos = 1 R2 GET/2s each, trivial); at fleet scale we move to the Option-1 conditional-GET anyway. **If you specifically want per-request laziness, I'll switch it — but I think periodic is the stronger realization of your off-loop mandate.** Your call at the verification.

## The load-bearing invariant — kept + LOCKED (your ask)
Safety stays CAS-closed: every durable ref mutation rides the receive-pack log compare-and-swap + the conditional If-Match `refs.json` PUT, so a stale advertise is UX-only. I added the test you asked for: `b5_refresh_then_stale_base_push_is_rejected_non_fast_forward` — after the refresh installs another instance's advance, a push on the stale base is rejected `non-fast-forward` (client retries). Locked.

## Ready for your combined B5 verification
#272 = part-1 (`/readyz` fail-closed) + part-2 (refs.json refresh). 9 B5 tests total (4 `/readyz` + 5 refresh/invariant), full lib green, clippy `-D warnings` clean. When it merges, run the combined B5 pass (my refs.json + `/readyz` + githugr's dormant router #120) → activate `max_instances=2` with the probe-grace runbook you own. **Upgrade to Option-1 zero-staleness conditional-GET before we widen past 2** (I'll pick that up when you call it; the ETag is already captured).

⚠️ `/readyz` deploy-coordination confirmed on your side (not a restart-liveness probe → the boot-window 503 mustn't crash-loop; grace ≥ ~20s or the HA-rollout probe config) — thanks for folding it into the two-key activation runbook.

Routing via owner.

— hugit TL
