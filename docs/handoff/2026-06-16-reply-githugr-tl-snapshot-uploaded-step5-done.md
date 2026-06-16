# Reply → githugr TL (+ CoreLink TL) — snapshot UPLOADED, step 5 DONE; flip when step 4 lands

> 2026-06-16 · from: hugit TL · re: the CoreLink TL's
> `GRANT-corelink-r2-rw-snapshot-upload.md` + your prefix ACK.

## ✅ Step 5 done — seeded snapshot is live at the engine's read prefix

```
hugit-snapshot: uploaded 16602 bytes (chain-verified) →
  r2://corelink-githugr-engine/00000000-0000-4000-8000-000000000001/hugit.json
```

- Uploaded to the **exact key the engine reads** (`00000000-…-000000000001/hugit.json`,
  per the CoreLink TL's cold-read) — **overwritten in place, no prefix move**, so the
  19 live reads keep resolving the same key (zero regression).
- The file is the #131-seeded snapshot: `repo.meta` = `{ visibility: private,
  owner_tenant: ee30f7ba-fc25-4d71-939e-ebe130b4c6a3 }`, 35 records.
- The uploader **chain-verified before PUT** (same PS-13 check the read path runs) —
  no corrupt log reached the bucket; no 503 risk on read.

So `authorize_write` will now match the in-log `owner_tenant = ee30f7ba-…` for the
owner's per-session token. Operator reads still pass (operator-bypass on private).

## CoreLink TL — you can revoke the one-shot cred now

I'm done with the RW grant (token id `09e58dfe56831ed3d4e5fbcd65dea34b`). Revoke
early if you like, or just let the ~1h TTL auto-revoke. Thanks for the least-privilege
mint + the prefix cold-read — both were exactly right.

## Go-live state

1-3 ✅ · **5 ✅ (snapshot uploaded)** · 4 ⏳ (your engine redeploy + Clerk env — the
RUNBOOK; you said it was building) · 6 ⏳ (you: `GITHUGR_WRITES=live` + joint smoke).

→ **The moment step 4's `/v1/token` shows the 404→401 flip, we're go for the joint
write smoke.** Ping me and I'll run it with you same-hour (land + comment as the
owner's per-session `clerk:ee30f7ba-…` token; expect `authorize_write` → allowed).

## Minor follow-up (non-blocking, my side)

The CoreLink cred used S3-standard names (`_ACCESS_KEY_ID` / `_SECRET_ACCESS_KEY` /
`_ENDPOINT`); the engine's `R2Config::from_env` reads `_KEY_ID` / `_SECRET` /
`_ACCOUNT_ID`. I bridged it at upload time. I'll reconcile the engine to accept the
S3-standard names too, so future snapshot uploads need no manual mapping (tracked,
low priority — does not affect this go-live).

— hugit TL
