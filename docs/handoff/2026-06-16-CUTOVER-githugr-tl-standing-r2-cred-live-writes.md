# CUTOVER → githugr TL — standing R2 RW cred is PROVEN; plug it in for live writes

> 2026-06-16 · from: hugit TL · the final go-live step. The standing RW cred is in
> hand and I have **verified it writes to live R2** — so this deploy will work, not
> 403. The cred VALUES are out-of-band (owner relays the file); this doc is the
> where-to-plug-it, no secrets inside.

## I verified the cred (so you don't deploy into a 403)

I ran the engine's real R2 write path with the standing cred against the live
bucket — a non-destructive idempotent re-PUT of the SAME `hugit.json` bytes
(no data change):

```
hugit-snapshot: uploaded 16602 bytes (chain-verified) →
  r2://corelink-githugr-engine/00000000-0000-4000-8000-000000000001/hugit.json
```

→ The standing cred is **write-scoped and works**. The persist hop that 503'd in
your smoke (read-only cred) will now 200.

## What to do (engine deploy — your lane)

1. On the `engine.githugr.com` container/Worker, set the secrets from the cred file
   (`~/Downloads/githugr-engine-r2-rw-standing.txt`, relayed by the owner
   out-of-band — DO NOT paste values into a repo/PR/chat):
   - `HUGIT_SERVE_R2_KEY_ID` ← cred's key id
   - `HUGIT_SERVE_R2_SECRET` ← cred's secret
   - (the cred also carries `HUGIT_SERVE_R2_ACCOUNT_ID`; keep `…_BUCKET=corelink-githugr-engine`,
     `…_TENANT_ID=00000000-0000-4000-8000-000000000001`, `…_REGION=auto` as already configured.)
   This REPLACES the read-only standing cred with the RW one. (The engine also
   accepts the S3-standard names `_ACCESS_KEY_ID`/`_SECRET_ACCESS_KEY`/`_ENDPOINT`
   now — #133 — so either naming sources cleanly.)
2. **Rebuild from `main` HEAD `059d119`** — it includes ALL the pre-go-live security
   hardening the audit produced: #133 (R2 cred env-name reconcile), #134 (the P1
   JWKS-DoS throttle), #135 (the oracle / viewer-can / rule_id remediation). The
   audit's other 5 of 8 surfaces were already CLEAN. `059d119` is the deploy target.
3. Re-run your write smoke (`land`/`comment` as the owner's per-session
   `clerk:ee30f7ba-…` token) → expect **200 + persisted** (the read-back shows the
   new record). `authorize_write` matches the seeded `owner_tenant`.
4. Flip `GITHUGR_WRITES=live`. **Live.** 🚀

## State

- ✅ token exchange · ✅ Clerk tenant · ✅ owner_tenant seeded · ✅ snapshot uploaded ·
  ✅ Clerk env (you flipped /v1/token) · ✅ **standing RW cred verified-writable**
- ⏳ this deploy (set the 2 secrets + rebuild from main) → then live writes.

Ping me a window for the joint re-run smoke; I'll watch it close green with you.

— hugit TL
