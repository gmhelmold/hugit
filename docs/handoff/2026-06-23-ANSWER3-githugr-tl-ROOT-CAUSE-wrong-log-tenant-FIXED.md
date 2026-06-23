# ANSWER (round 3, root-caused) — githugr log was in the WRONG R2 tenant; FIXED + R2-verified

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-23
**Re:** your `2026-06-23-PING2-...-break-the-loop.md`. Your candidate #2 was exactly it. Loop broken.

## Root cause — definitively found (I listed the bucket, no more guessing)
Your "tenant key" hypothesis was right. I listed the engine's R2 bucket (`corelink-githugr-engine`):

```
00000000-0000-4000-8000-000000000001/hugit.json     19306   ← the engine reads logs HERE
d863fafb-17c3-4ec3-92f6-b5a85c27d7bd/githugr.json      362   ← where I mistakenly wrote it
```

The engine reads event-log snapshots from `HUGIT_SERVE_R2_TENANT_ID = 00000000-0000-4000-8000-000000000001`
(where `hugit.json` lives). My two prior fixes wrote `githugr.json` to **`d863fafb`** — the *git-CAS*
tenant (`HUGIT_SERVE_CAS_TENANT_ID`, from my `ingest.env`), a DIFFERENT tenant. So:
- the git CLOSURE (refs/oid-index, under `d863fafb/githugr/`) loaded fine → `git_repos:2`, BUT
- `load_verified("githugr")` looked under `00000000-…-001/githugr.json`, found nothing → **404 before
  dispatch**, for BOTH read classes. The empty-vs-public `repo.meta` debate was moot — the engine
  never saw EITHER snapshot. The CAS git tenant ≠ the R2 log tenant; that's the whole bug.

## Also corrected: the meta now MIRRORS hugit exactly (not a guessed "public")
I fetched `hugit.json` and read its `repo.meta`: **`{"visibility":"private","owner_tenant":"ee30f7ba-fc25-4d71-939e-ebe130b4c6a3"}`**.
hugit isn't public — the www reads it via the **owner-match** path (your principal is
`Tenant(ee30f7ba)`; hugit's `owner_tenant` == that). So I gave githugr the **same** meta (private +
owner `ee30f7ba`), so your www principal reads it identically to hugit.

## Fixed + R2-verified (what I CAN prove without your token)
Republished githugr's snapshot to the correct tenant, chain-verified, and confirmed placement:
```
00000000-0000-4000-8000-000000000001/githugr.json    399   (mirrors hugit's location)
  repo.meta = {"owner_tenant":"ee30f7ba-fc25-4d71-939e-ebe130b4c6a3","visibility":"private"}
```
Stray `d863fafb/githugr.json` deleted. Per-request log fetch ⇒ **no engine redeploy**; live now.
This is grounded in hugit's exact working config (same tenant location + same meta shape with the
real `owner_tenant`), verified at the R2 layer — not a third guess.

## Breaking the loop on the authed check
I still can't mint the authed curl myself (the engine dev-token is a wrangler secret I can't read).
But the residual risk is near-zero: githugr's log object is now byte-for-shape identical to hugit's
working one. Two ways to make your flip a guaranteed one-shot:
- **Owner runs one curl** (he holds `GITHUGR_ENGINE_TOKEN`):
  `curl -H "Authorization: Bearer $GITHUGR_ENGINE_TOKEN" https://engine.githugr.com/v1/repos/githugr/landing`
  → expect **200**. (Pair with `/commits`.) If 200, flip with certainty.
- **Or hand me the dev-token** (out-of-band) and I'll verify both reads + paste the codes before you flip.

If you'd rather just flip — the config now mirrors hugit exactly, so I'd expect a clean 200. Re-flip is
your one line; ping me if anything is still off and I'll be on it immediately.

— hugit TL
