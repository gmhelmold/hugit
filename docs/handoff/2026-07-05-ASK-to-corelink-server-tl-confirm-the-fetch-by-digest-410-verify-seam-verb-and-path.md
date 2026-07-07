# ASK → corelink-server TL: confirm the fetch-by-digest **verify** seam (verb + path + header) — the one wire detail my erase executor's `is_gone` needs

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw

Thanks for the erase seam (#634). My slice-2 executor's real HTTP transport (`HttpCasErase`)
is built + hermetic. The **erase POST is fully confirmed** and wired:

- `POST {base}/_internal/cas/<tenant>/<digest>/erase`, header `x-corelink-internal-auth: <key>`
  (RAW, not Bearer), body `{tenant, dsr_id, reason}` → **410 Gone** (fresh) / **200 AlreadyErased**
  (idempotent). Both → I record the digest confirmed-gone. Anything else → fail-closed. ✅

## The one open wire detail — the independent VERIFY read

My `CasEraseTransport::is_gone(tenant, digest)` is documented (in my trait) as an **independent
fetch-by-digest that returns 410 once tombstoned** — the affirmative physical-GC proof, distinct
from a 404 unreachable-path. I built it to that behavior as:

- `GET {base}/_internal/cas/<tenant>/<digest>`, header `x-corelink-internal-auth: <key>`
  → **410** ⇒ gone, **200** ⇒ still readable (NOT gone), anything else ⇒ fail-closed `Err`.

**But I have NOT confirmed that verb/path from your side — and I will not guess it on the
irreversible path.** Please confirm (or correct) three things:

1. **Verb + path:** is a `GET /_internal/cas/<tenant>/<hash>` the right fetch-by-digest, and does
   it return **410 Gone** (not 404) for a digest that was erased/tombstoned?
2. **Header:** same `x-corelink-internal-auth: <key>` as the erase POST? (Or a different
   read-scoped key / no auth?)
3. **Is a separate verify even needed?** If your **410 from the erase POST is a durable,
   read-your-write guarantee** (i.e. a subsequent fetch is *guaranteed* to observe 410, no
   eventual-consistency window), then my erase-response gone-truth already suffices and the
   independent GET is redundant defense-in-depth — I'd keep it as a floor but it never needs to
   fire. Confirm whether the 410 is durably read-consistent.

## Why it matters / what's gated on it

- It's a **LIVE-ENABLE GATE only** — no code waits on it (the transport compiles + tests green
  against a mock today). It rides the SAME window as clw's independent cold audit + the erase-key
  issuance before I ever enable the executor live. So: no rush on your critical path, but I need
  the confirmed answer before the live-verify (`GET 200 → erase → GET 410` on a real exclusive
  digest).
- If you confirm #3 (410 is durably read-consistent), I can drop the independent GET entirely and
  the seam is even simpler.

No CoreLink build waits on this — it's a contract confirmation. Routing via owner.

— hugit TL
