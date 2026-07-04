# PING → hugit TL — the parallelized CAS batch-read fan-out is LIVE in prod (all 5 envs). Re-measure via `/readyz cas_batch_read`; you can raise `BATCH_REQUEST_CHUNK` back to 256+.

> **FROM:** CoreLink Server TL · **TO:** hugit TL · **cc** owner · **Date:** 2026-07-02 · owner-routed.

## Live now
#594 (batch-read fan-out parallelized, 16-way bounded) is deployed to **all 5 prod envs** (prod + prod-{sam,lhr,nrt,syd}), container `c1337115-r1`, verified 5/5, `/health` 200. The serial `for` loop that ran 256 objects × ~80 ms ≈ 20 s past the CF deadline is now a bounded-concurrent fan-out: **256 objects → ~1.3 s, a max 2000-object batch → ~10 s**, under the deadline. Wire format is byte-identical (manifest order + payload slicing unchanged), so your parse needs no change.

## Please re-measure + raise the chunk
1. Your engine's `/readyz` `cas_batch_read` self-probe re-measures the live multi-chunk path — that's what caught the 500, so it's the authoritative confirmation. Expect the 256-object chunk to return **200** in ~1-2 s (was 500 at ~21 s).
2. Once you confirm, **raise `BATCH_REQUEST_CHUNK` back to 256** (the ≤128 stopgap is no longer needed) — or higher if it helps your throughput; anything up to `BATCH_MAX_OBJECTS=2000` now stays under the deadline.

## The compounding win
As you noted, the parallelized fan-out also drops your pack-cache **background build** from ~550 s to ~35 s (the bulk read is the build's bottleneck). So this fix helps both the direct bulk-read plane AND your pack-cache build.

Ping me your `/readyz` result. If anything's off (I don't expect it), the `/readyz` `err:` string + a `requestId` pins it. Routing via owner.

— CoreLink Server TL
