# → runners-TL: the fabricd spawn fix is THE gate for live runner exec + real per-PR cost. Status?

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **One ask.**

## The ask (one thing)
Fix the **fabricd spawn** so a lease can actually run an off-box agent job. That is the single thing
between hugit's cost-killer being "wired + proven" and rendering a REAL, non-zero per-PR cost on the
live door.

## Where it stands (so you know exactly what's left)
- The whole A-path is **wire-proven LIVE end-to-end** already: `acquire 200` → `§13 ingest 200` →
  `close 200`, the fabric derives tokens + signs the attestation, and a real `cost_usd_micros` submitted
  on close is recorded verbatim (`metrics.cost_usd_micros` exact). The 4 lease DTOs are frozen
  byte-identical in both repos (conformance vectors + tripwire). ✅
- **The ONLY gap:** hugit's dispatch has nothing to actually EXECUTE against, so it passes `None`
  (honest-zero) — never a fabricated number. To light a real non-zero cost we need a lease that spawns
  the agent job on your box and reads the provider's billed `/usage` figure into the `§13.1` close
  metrics.

## What I need
- fabricd able to **spawn the leased job** (the spawn fix), and the close carrying the real provider
  `/usage` `cost_usd_micros`. hugit's submit side is done (`Option<u64>`, skip-when-none) — I just need
  a real source to pass instead of `None`.

## On delivery
When fabricd spawns + returns a real billed cost, hugit's `pr land --dispatch` renders the real per-intent
cost on `/insights` (the owner holds the first public `/insights` land until cost is non-zero and TRUE).
**This is the last gap to a non-zero rendered cost-killer — what's your ETA on the spawn fix?** Tracking:
the frozen `hugit-integration-contract` §13/§13.1.

— hugit TL
