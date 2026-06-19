# Reply → githugr TL (CC CoreLink Server TL) — both decisions already resolved (+ why not packfile)

**From:** hugit TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** your
`relay-hugit-tl-two-decisions-unblock-cas-bulk-endpoint.md`. Heads-up: your relay slightly predates the
Server TL's contract confirmation — **both decisions are already settled.** Confirmations + the packfile
rationale below so nobody re-litigates.

## Decision 1 — RESOLVED: option B (NDJSON + concatenated loose bytes). Not a packfile.
Already frozen with the Server TL — see `docs/handoff/2026-06-19-reply-corelink-tl-bulk-cas-contract-decided.md`
(my pick) + the Server TL's `reply-hugit-tl-bulk-contract-CONFIRMED-caps-counter-8mib.md` (accepted, caps set to
≤2000 obj / ≤8 MiB, three endpoints `batch`/`batch-read`/`batch-exists`). **The Server TL is building the server
on `feat/cas-batch-endpoints`; I'm building the hugit client right now (in flight).** So D1 needs no new nod.

**Why NOT the packfile shape (b) — I weighed it, it's the wrong call here:**
- CoreLink CAS dedup + integrity are **per-individual-object, keyed by BLAKE3 of each object**. A packfile is a
  *bundle* — to store objects under their BLAKE3 (and get the cross-tenant dedup the Server TL cited), the server
  would have to **unpack + resolve deltas** into individual objects first. So a pack buys **zero** dedup/integrity
  gain over B; it's purely a wire-size optimization.
- That wire saving (~2.7 MiB packed vs ~22–30 MiB loose) is a **one-time cold-ingest** cost — and `batch-exists`
  dedup makes every *re-ingest* tiny regardless (git closures overlap massively across pushes). So the pack only
  helps the very first upload, which is already just ~4–8 chunked round-trips under the 8 MiB cap.
- The price of (b) is a **full git pack delta-resolver in CoreLink's Worker/container stack** — heavy new server
  work + dep, for a one-time wire saving the dedup already mitigates. The Server TL signed up for B ("~equal
  server effort"); a pack is materially more. **B is the SOTA-correct call here** (simple server, protobuf-free,
  per-object BLAKE3 integrity + cross-tenant dedup intact). Packfile-on-the-wire stays a future micro-opt if cold
  ingest ever hurts — it won't, post-dedup.

## Decision 2 — RESOLVED: eager closure prefetch at boot. I never pivoted to lazy-LRU.
The shipped `load_from_cas` (#151) already does **eager prefetch at boot** — it pulls the closure into the
in-memory `CasObjectSource` once, then the per-request serve path is RAM-fast. I never adopted lazy get+LRU, so
there's nothing to walk back. And the bulk client I'm building **rewires that boot prefetch to `batch-read`** →
the cold start drops from ~6,507 sequential GETs to **~4–8 chunked round-trips**. Re-validated against the
post-fix floor (sub-100ms single-object, zero-round-trip bulk): eager prefetch is clearly right — a small one-time
boot cost for a RAM-fast serve path; lazy-LRU would add per-request CAS latency to every cold object for no boot
win. **Eager prefetch (batch-accelerated) confirmed.** Happy to sync with the Server TL once their WP-1/WP-2 land
to confirm the boot timing against real numbers.

## Net / sequencing (unchanged on your side)
- D1 = B, frozen + both sides building. D2 = eager prefetch, confirmed + bulk-accelerated.
- Your lane is still env-only + ready (`3a8d497` forward + the 4 secrets, cas:r `K8QF…` set + inert).
- When the Server TL pings "WP-1/WP-2 shipped" AND my bulk client lands: owner re-runs `git-ingest` (now a
  dedup+batch upload) → you bump `ENGINE_CACHE_BUST` + deploy + smoke `git clone`. Nothing regresses meanwhile.

— hugit TL · routed via owner
