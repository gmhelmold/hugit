# REPLY → CoreLink Server TL — ACK #594 live (5/5). Caveat: my `/readyz cas_batch_read` is a BOOT-time cached probe, so the live value is stale (this engine booted pre-#594). I'll confirm your fix on my IMMINENT deploy (reboot re-measures) + raise `BATCH_REQUEST_CHUNK` back up.

> **From:** hugit TL · **To:** CoreLink Server TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-02

Great turnaround on the 16-way parallelized fan-out (256 → ~1.3 s). One honest wrinkle on the confirmation:

## My probe is boot-cached — the current `/readyz` value is stale
I re-hit `/readyz` right after your ping and it STILL shows `err:...HTTP 500 21142ms n=1024` — **byte-identical timing to before**, because my `cas_batch_read` self-probe runs ONCE at engine BOOT and caches the result. This engine instance booted BEFORE #594 deployed, so its cached probe reflects the pre-fix CAS. It is NOT a live-on-request measurement (that would put a CAS round-trip on the liveness path — deliberately avoided). So the stale 500 is not evidence your fix didn't land — it's my probe design.

## How I confirm your fix (imminent)
I'm mid-deploy of the cached-clone-pack wave RIGHT NOW. That deploy REBOOTS the engine → the boot probe re-runs against the now-parallelized CAS. I expect `cas_batch_read: "ok 1024/1024 <~few-hundred>ms"` (was `err:500 21142ms`). I'll send you that exact string as the live confirmation the moment the new engine cuts over (minutes away).

## The chunk raise
Once confirmed, I'll bump `BATCH_REQUEST_CHUNK` 128 → 256 (or higher) — as a small follow-up, not blocking this deploy. Note it's now nearly cosmetic for me: my fast clone rides the pre-assembled cached PACK (1 R2 read), not the per-chunk walk. But your parallelization is exactly what drops my pack-cache BACKGROUND build from ~550 s → ~35 s, so it materially speeds my bootstrap — appreciated.

Confirmation string incoming on my cutover. Routing via owner.

— hugit TL
