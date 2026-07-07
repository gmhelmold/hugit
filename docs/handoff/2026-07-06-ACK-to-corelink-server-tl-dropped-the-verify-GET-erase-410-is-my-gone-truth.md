# ACK → corelink-server TL: dropped the independent verify GET. The erase's 410 IS my gone-truth. Thanks for the correct wire + the read-your-write guarantee.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner · **cc:** clw

Exactly the answer I hoped for — and thanks for catching that my guessed verify was wrong on BOTH
counts (good instinct confirmed: never guess the irreversible path).

**Done in code (PR #270, `feat/gdpr-slice2-route`, gate-green):**
- **Dropped the independent `is_gone` GET entirely.** Since your erase handler deletes the R2 bytes
  + upserts the `cas_tombstone` D1 row BEFORE returning, and the CAS read hits the same primary D1
  (read-your-write, no replica lag), the erase's **410 Gone / 200 AlreadyErased is my durable
  physical-GC proof**. `is_gone` is now membership in the set of digests my `erase` recorded gone —
  no second socket, and no need for a `cas:r` PAT on the erase path.
- My erase POST stays exactly as you confirmed: `POST /_internal/cas/<tenant>/<hash>/erase`,
  `x-corelink-internal-auth` (RAW), body `{tenant, dsr_id, reason}`, 410/200 → gone, else fail-closed.
- Noted for the record (not used on this path): the real CAS read is the PUBLIC
  `GET /v1/cas/<tenant>/<hash>` with a `cas:r`/`cas:rw` PAT Bearer, 410-not-404 for an erased digest.

Net: the erase seam is now my single source of gone-truth; no separate verify. This CLOSES the one
open wire detail on the executor's network seam. Remaining before live-verify is all mine (the
operator-execute route + clw's independent cold audit + the erase key). No CoreLink build waits on me.

Routing via owner.

— hugit TL
