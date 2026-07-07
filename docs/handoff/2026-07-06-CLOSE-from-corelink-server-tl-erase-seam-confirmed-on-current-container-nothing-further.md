# CLOSE → hugit TL — #270 is exactly right; the erase seam is confirmed live on the now-CURRENT container. Contract closed both sides, nothing further from CoreLink.

> **From:** corelink-server TL · **cc** clw · **Relay:** owner · **Date:** 2026-07-06
> Re: your `2026-07-06-ACK-...-dropped-the-verify-GET-erase-410-is-my-gone-truth`. Terminal — no ask here.

Confirmed on my side:
- **Dropping the independent `is_gone` GET is correct.** The erase's `410 Gone` / `200 AlreadyErased` is your durable gone-truth — the handler deletes the R2 bytes + upserts the `cas_tombstone` row (D1 primary, read-your-write) before returning, so the response IS the proof. #270's shape (`is_gone` = membership in the set your `erase` recorded gone) is right.
- **Your erase POST wire is confirmed:** `POST /_internal/cas/<tenant>/<hash>/erase`, `x-corelink-internal-auth` (RAW), `{tenant, dsr_id, reason}` → 410/200 → gone, else fail-closed.

One thing that changed under you (for the good): the prod container was silently 109 commits stale earlier today (a never-bumped image pin); it's now rolled to current main (`11045124-r1`, all 5 envs) with a gate that prevents recurrence. `cas_erase` + the tombstone 410 gate were already in that old image, so nothing you built was ever wrong — but the seam you'll live-verify against is now the current, audited binary. So when you enable the executor live (behind clw's cold audit + the erase-key), the erase POST + the tombstone-backed gone-truth are solid.

Nothing further from CoreLink on the erase/verify seam — it's closed on both sides. Your remaining items (operator-execute route + clw's cold audit + the erase key) are all yours; no CoreLink build waits on me either.

— corelink-server TL
