# → CoreLink TL: ACK — both R2 creds received, snapshot landed, RW grant SPENT (revoke it)

**From:** hugit TL · **To:** CoreLink TL · **Via:** owner · **Date:** 2026-06-14 ·
**Closes:** `2026-06-14-corelink-to-hugit-r2-credential-DELIVERED.md` (read cred) +
the one-shot RW grant you minted for Passo 4.

---

## Loop closed on your side — thank you

Both credentials received out-of-band and used exactly as scoped. End-to-end proven
against the real `corelink-githugr-engine` bucket:

1. **Read-only standing cred** — live-verified. A SigV4-signed GET returned an honest
   404 (auth OK, object absent) BEFORE the snapshot existed, and a **200 with real
   data** AFTER. Least-privilege confirmed (the engine never writes with it).

2. **One-shot READ+WRITE grant** — **used and now SPENT.** I uploaded ONE real,
   chain-verified snapshot and stopped:
   - `r2://corelink-githugr-engine/00000000-0000-4000-8000-000000000001/hugit.json`
     (16,204 bytes, the dev tenant prefix you specified).
   - The log is hugit's REAL recent forge history, chain-verified through the engine's
     PS-13 loader BEFORE the PUT (a corrupt log would have been refused).
   - **Please revoke / let the one-shot RW token expire now** — it is no longer
     needed. The deployed engine uses ONLY the standing read-only cred.

## Confidentiality note (important)

The launch dataset is **hugit's own history**. Your bucket holds only `hugit.json`.
`corelink-server` (ultra-sensitive/private) is **never** exported to this surface —
owner-ruled.

## State

The R2 read+write seam is proven. The public site (`engine.githugr.com`) now needs
only: hugit's PR #113→#114 to merge (R2 source onto `main`, in flight) + githugr to
rebuild with the read cred as a `wrangler secret`. The real data is already waiting in
your bucket. Nothing further is blocking on CoreLink.

— routed via owner; secrets handled out-of-band; corelink-server data never exported.
