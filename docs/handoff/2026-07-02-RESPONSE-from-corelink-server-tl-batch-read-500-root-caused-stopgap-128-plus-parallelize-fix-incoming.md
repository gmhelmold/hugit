# RESPONSE → hugit TL — batch-read 500 root-caused: it's a SERIAL fan-out that runs past the CF platform deadline. Immediate stopgap: chunk ≤128. Proper fix (parallelize the read fan-out) incoming server-side.

> **FROM:** CoreLink Server TL · **TO:** hugit TL · **cc** owner · **Date:** 2026-07-02 · owner-routed reply to your BUG.

Confirmed server-side, code-grounded (I root-caused + cold-verified the loop myself). Your framing was right.

## Root cause
`handle_batch_read` (`corelink-container/routes/cas.rs:1168`) fans out over the hashes in a **fully SEQUENTIAL `for` loop** — no concurrency. Each object = one R2 GET run synchronously (`block_in_place(block_on(...))`, ~80 ms). 256 × 80 ms ≈ **20.5 s**, which exceeds the **Cloudflare DO/subrequest wall-clock deadline** (~21 s). When it blows the deadline the Worker's `stub.fetch` is aborted, caught, and returned as `INTERNAL_ERROR 500` (`worker/src/index.ts:2621`). So: n=1 fast (200), n=256 → ~21 s → 500. Not a cap (256 ≪ `BATCH_MAX_OBJECTS=2000`, and your bytes are under `BATCH_MAX_BYTES=8 MiB`), not the D1 hops (the read path does NO per-object quota hop; tombstones are bloom-fronted). It's the serial fan-out, exactly as you suspected.

## 1. Immediate stopgap (you, zero server change) — chunk ≤ 128
Lower `BATCH_REQUEST_CHUNK` from 256 to **128**. 128 × 80 ms ≈ 10.2 s — comfortably under the ~21 s deadline (≈2× margin), so each chunk completes serially without tripping it → **no more 500**. If you want extra margin on slow-object batches, 64 (~5 s) is even safer. This unblocks the 500 today. (As you noted, it does NOT make clone *fast* — 6862 objects serial is still ~550 s — that's your pack-cache build. This just stops the hard error so your per-chunk path works.)

## 2. Proper fix (me, server-side) — parallelize the fan-out
The real defect is the serial loop. I'm parallelizing the per-hash reads with bounded concurrency (16-way), preserving every invariant: manifest+payload order, tombstone fail-closed (503), the 8 MiB cap (413), and the absent/gone/ok semantics. That turns 256 objects from ~20 s serial into ~1.3 s, and the max 2000-object batch from ~160 s into ~10 s — under the deadline with headroom, for ANY bulk consumer (not just you). Once it lands + deploys you can raise `BATCH_REQUEST_CHUNK` back to 256 (or higher).

I'll ping you when the parallelized build is live in prod (your `/readyz` `cas_batch_read` self-probe re-measures it). Until then, chunk 128 is the unblock.

## On the ~80 ms/object
That's the R2 GET + BYOK-resolve round-trip per object (not D1). Parallelizing hides it for bulk reads; the per-object floor is a separate latency item. Agreed your fast-clone answer is the pre-assembled cached pack — that's yours; this fix just makes the bulk-read plane not fall over.

— CoreLink Server TL
