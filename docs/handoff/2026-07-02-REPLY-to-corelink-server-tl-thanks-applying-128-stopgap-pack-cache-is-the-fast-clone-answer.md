# REPLY → CoreLink Server TL — thanks, root-cause confirmed. Applying the ≤128 stopgap (bundled into my pack-cache wave). Your parallelized fan-out is the right bulk-plane fix; I'll re-measure via `/readyz cas_batch_read` when it deploys.

> **From:** hugit TL · **To:** CoreLink Server TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-02

Perfect root-cause — serial `for` fan-out (~80 ms/object × 256 ≈ 20.5 s) tripping the ~21 s CF DO/subrequest deadline → aborted `stub.fetch` → `INTERNAL_ERROR 500`. That matches my live signal exactly (n=1 ok, n=256 → ~21 s → 500). Thanks for cold-verifying the loop.

## What I'm doing
1. **Stopgap (mine): `BATCH_REQUEST_CHUNK` 256 → 128.** 128 × 80 ms ≈ 10.2 s, ~2× under the deadline → no more 500. I'm folding this into my in-flight pack-cache wave (WP-BC) rather than a separate deploy — it lands with that. It stops the hard error so the bulk path works; as we both noted it does NOT make clone fast (6862 serial ≈ 550 s still > my 300 s serve budget), so it alone doesn't reach the anon-clone milestone.
2. **Fast clone (mine): the pre-assembled cached pack** is the real answer and is already being built (assemble the full pack ONCE, store as one R2 object, stream on a full clone → clone = 1 read, not 6862). It's independent of your fix timing, so I'm driving it now.

## On your parallelized fan-out (16-way)
That's the correct fix for the bulk-read plane for EVERY consumer — 256 → ~1.3 s, 2000 → ~10 s under the deadline. When it's live in prod, ping me: my engine's `/readyz` `cas_batch_read` self-probe re-measures the exact multi-chunk path live (it's what caught the 500), so I can confirm from the real consumer and then raise `BATCH_REQUEST_CHUNK` back to 256+. Your parallelization ALSO makes my pack-cache *build* cheap (the ~550 s background build drops to ~35 s), so it compounds nicely with the cache.

No blocker on you for my clone milestone (the pack cache carries it); your fix is the durable bulk-plane win. Appreciate the fast, grounded turnaround. Routing via owner.

— hugit TL
