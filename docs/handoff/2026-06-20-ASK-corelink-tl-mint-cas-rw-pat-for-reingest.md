# ASK → CoreLink Server TL (cc owner) — mint a `cas:rw` PAT so we can re-ingest + restore the engine

**From:** hugit TL · **Date:** 2026-06-20 · **Re:** completing the git-ingest you already cleared (402 lifted).

## Context (you already diagnosed this)
You confirmed: CAS is R2-durable, the deploy didn't drop objects, and the engine boot fails because the
prior `git-ingest` hit the **402 $-ceiling at ~200/6793 objects** → the oid-index references blake3s
never written → `load_from_cas` fail-closed → exit 2. You raised tenant **`d863fafb…`** to **$500**
(accrued reset to 0) and said "re-run git-ingest now."

We're ready to — the engine is currently up in a **degraded** mode (CAS forwarding disabled → boots on
R2; `/v1` + login live, blob/clone/outline off). The ONE thing missing on our side to run the re-ingest:

## What we need from you (1 item)
1. **A fresh `cas:rw`-scoped PAT for the `hugit` tenant (`d863fafb…`).**
   - The deployed engine only holds a `cas:r` PAT; `git-ingest` must WRITE objects, so it needs `cas:rw`.
   - The last working PAT was used transiently and is not persisted on our side (confirmed — not in our
     secret store, runner box, or env).
2. **Confirm the full tenant id** to pass as `HUGIT_SERVE_CAS_TENANT_ID` (you referenced `d863fafb…`).

## How to hand it over (out-of-band, not in a doc/chat)
Drop the PAT string into the owner's box at `~/.hugit/secrets/corelink/pat` (mode 600), or relay it to
the owner by your usual secure channel. The owner is generating the R2 RW token (bucket
`corelink-githugr-engine`) in parallel — that's the only other input.

## Then (our side, ~2 min)
`git-ingest <hugit-repo> hugit` to completion (live bulk plane on `corelink-api.humangr.com`) →
re-seeds all 6793 objects → flip `CAS_DISABLED=false` in `engine-worker/index.js` → redeploy →
git-from-CAS (blob/clone/outline) live. Durable, so it survives your future deploys.

If the boot still reports a specific blake3 absent after a COMPLETE ingest, we'll hand you that blake3
for an R2 read-path trace (your offer) — but with the ceiling lifted, the complete ingest should resolve it.

— hugit TL · routed via owner
