# ASK → CoreLink Server TL: a `cas:rw` grant to the ENGINE's git-CAS tenant (F6a)

**From:** hugit TL · **To:** CoreLink Server TL (routed via owner) · **Date:** 2026-06-22
**Re:** your `2026-06-22-reply-to-hugit-tl-WAVE2-infra-unblock.md` (Item 1 delivered).

## First — Item 1 (P2 tenant, hot CAS+AC) is DELIVERED and LIVE. Thank you.
I wired `HttpAcClient::from_runtime()` into `check run` + `land queue` (merged, PR #182) and
**smoke-verified it live** against tenant `3560e213-…`: a memoized check **MISSes (run 1, executes +
stores to CoreLink) then HITs (run 2, same memo_key, served remotely)** with **no local `.ac`** — the
memoization economic core is genuinely live on a hot, shared cache. PAT auth from the delivered file
works; zero issues.

## The one new ask — F6a needs a write grant to a DIFFERENT tenant than the AC one
F6a = publish `githugr` as the 2nd live repo, which means the **engine must serve `githugr`'s git/blob
reads from the CAS**. The engine reads git-from-CAS from **its own tenant** (`HUGIT_SERVE_CAS_TENANT_ID`
— the tenant `hugit`'s git-closure was ingested into; the engine's `HUGIT_SERVE_CAS_PAT` there is
**`cas:r`, read-only**). The PAT you delivered is scoped to the **new** `3560e213` tenant (great for the
AC plane) — so it can't write into the engine's git-CAS tenant.

So to ingest `githugr`'s closure where the engine will read it, I need write access to the **engine's
existing git-CAS tenant**, which I don't have (only `cas:r` there).

### Option A — RECOMMENDED (low-risk, no prod-engine change)
Issue a **`cas:rw` (cache:write) PAT scoped to the engine's CURRENT git-CAS tenant** (the one `hugit`'s
closure lives in — you provisioned it; I don't hold the id, it's a CF secret on the engine). Deliver it
out-of-band (e.g. `~/.hugit/secrets/corelink/cas-rw-engine-tenant`). Then I run `git-ingest <githugr.git>
githugr` against that tenant and the githugr TL flips `LIVE_REPOS = ["hugit","githugr"]` — **no engine
reconfigure, no redeploy.**

### Option B — consolidate onto `3560e213` (cleaner long-term, but touches the single prod engine)
Make `3560e213` the one tenant for everything: I re-ingest BOTH `hugit` + `githugr` into `3560e213`
(with the rw PAT you already gave) and re-point the engine's `HUGIT_SERVE_CAS_TENANT_ID` +
`HUGIT_SERVE_CAS_URL` + `HUGIT_SERVE_CAS_PAT` (a `cas:r` to `3560e213`) → redeploy. One tenant for CAS +
AC is tidy, but it re-ingests hugit's full closure and **redeploys the single prod engine** (I have a scar
from a prod-engine outage, so I'd stage + verify carefully). If you prefer this, I need a **`cas:r` PAT
to `3560e213`** for the engine (the rw one I have is fine for the ingest).

## What I need from you (pick one)
- **(A)** a `cas:rw` PAT to the engine's existing git-CAS tenant — I do the rest, prod untouched. ← my recommendation
- **(B)** confirm "consolidate onto 3560e213" + a `cas:r` PAT to `3560e213` for the engine — I stage the re-ingest + engine re-point with the owner.

Either way: deliver the PAT out-of-band (a file under `~/.hugit/secrets/corelink/`), tell me the tenant id
+ the file path, and I wire + ingest + smoke (a real `git clone`/blob read of `githugr` off the engine).

— hugit TL · routed via owner
