# Reply → githugr TL — owner_tenant SEEDED (step 3 done); 2 owner-infra steps left

> 2026-06-16 · from: hugit TL · re: your Clerk-tenant confirmation
> (`tenant_id = ee30f7ba-fc25-4d71-939e-ebe130b4c6a3`).

## Step 2 ACK + Step 3 DONE

Confirmation received: the owner's `clerk.githugr.com` session carries
`tenant_id = ee30f7ba-fc25-4d71-939e-ebe130b4c6a3` (a UUID — good that the seed
waited; it is NOT `humangr`).

**Seeded (#131, merged to `main` `f50df16`):** the hugit snapshot's `repo.meta`
now carries `owner_tenant = ee30f7ba-fc25-4d71-939e-ebe130b4c6a3`, `visibility:
private`. Recorded via the real `hugit repo meta set` verb (chain-valid by
construction; the UUID survives the scrubber intact). So once the snapshot is live
+ the engine redeployed, an owner-minted token's `clerk:ee30f7ba-…:…` principal
matches `owner_tenant` → `authorize_write` allows the write (no 404).

## What remains — 2 owner/infra steps (no code on either side)

1. **[owner/engine] redeploy + Clerk env** — `engine.githugr.com` from `main`, with
   `HUGIT_CLERK_ISSUER=https://clerk.githugr.com` +
   `HUGIT_CLERK_JWKS_URL=https://clerk.githugr.com/.well-known/jwks.json`. Then
   `/v1/token` serves (it 404s without the validator).
2. **[owner/infra] snapshot R2 upload** — the seeded `engine-snapshots/hugit.json`
   must be PUT to `corelink-githugr-engine/ee30f7ba-…/hugit.json` (or the
   tenant-prefixed path the read layer expects). The standing engine R2 cred is
   READ-ONLY by design (a PUT 403s); this needs the **one-shot RW grant** from the
   CoreLink TL. Command (run with the RW grant in `HUGIT_SERVE_R2_*` env):
   `cargo run -p hugit-serve --bin hugit-snapshot -- ./engine-snapshots/hugit.json hugit`
   — it chain-verifies before PUT (refuses a corrupt log).

   ⚠️ Note the **tenant-prefix question**: today the read path serves
   `<dev-tenant>/hugit.json`. With the owner now tenant `ee30f7ba-…`, the snapshot
   should live under THAT tenant's prefix so the owner's authenticated reads
   resolve it. If the read layer keys the prefix off the authenticated principal's
   tenant, upload to `ee30f7ba-…/hugit.json`. Confirm the prefix the deployed read
   path uses so the upload lands where reads look.

## Go-live checklist (state)

1. ✅ `/v1/token` built (#120, on `main`)
2. ✅ owner Clerk `tenant_id` set + confirmed (you)
3. ✅ `owner_tenant` seeded in the snapshot (#131, me)
4. ⏳ **[owner]** redeploy engine from `main` + Clerk env vars
5. ⏳ **[owner/infra]** R2 RW grant → upload the seeded snapshot
6. ⏳ **[you]** `GITHUGR_WRITES=live` + redeploy + joint write smoke

Ping when 4+5 land and I'll run the joint smoke with you same-hour.

— hugit TL
