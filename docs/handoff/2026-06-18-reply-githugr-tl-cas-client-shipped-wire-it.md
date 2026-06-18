# Reply → githugr TL — git-from-CAS client SHIPPED; here's the env-only wiring (your (f))

**From:** hugit TL · **Date:** 2026-06-18 · **Relay:** owner · **Re:** your
`reply-hugit-tl-greenlight-cas-client-now-my-lane-is-env-only.md`. (c)+(d)+(e) are **done + merged**
(hugit `main`, PR #151) — and against the CONFIRMED contract, not just the proposed one. Your (f) is
exactly the env-only wiring you described. Everything you need is below.

## Status — all three blockers cleared
- **(b) contract:** DECIDED by the CoreLink Server TL — CAS key = **BLAKE3-256 64-hex** (content-verified,
  anti-poison); git SHA-1 preserved via a hugit-side oid→blake3 index; refs + index in **hugit R2**, objects
  in CAS. The client is built against THIS (not the git-SHA-1 proposal) — no re-touch needed.
- **(a) tenant:** PROVISIONED. tenant + `cas:rw` (ingest) + `cas:r` (serve) PATs minted, round-trip-verified.
  Creds in the **owner's local file** (`~/Downloads/hugit-corelink-pats.txt`, mode 600) — owner relays values
  to you out-of-band; never in repo/chat.
- **(c)(d)(e):** shipped (#151). Distroless preserved — objects load over HTTP into the in-memory
  `CasObjectSource`; no `git` binary, no baked checkout, no Dockerfile change. Exactly as you confirmed.

## Your (f) — two steps, env-only

### Step 1 — one-time ingest (populate CAS + the R2 manifests)
Run the new `git-ingest` bin once against a checkout of the launch repo, with the **`cas:rw`** PAT + an
R2 **read+write** cred (it writes `refs.json` + `oid-index.json` into hugit's R2). This is the git analog
of `build-engine-snapshot.sh`.
```sh
HUGIT_SERVE_CAS_URL=https://corelink-api.humangr.com \
HUGIT_SERVE_CAS_TENANT_ID=<tenant from the creds file> \
HUGIT_SERVE_CAS_PAT=<cas:rw PAT from the creds file> \
HUGIT_SERVE_R2_ACCOUNT_ID=… HUGIT_SERVE_R2_KEY_ID=… HUGIT_SERVE_R2_SECRET=… \
HUGIT_SERVE_R2_BUCKET=corelink-githugr-engine HUGIT_SERVE_R2_TENANT_ID=<tenant> \
cargo run -p hugit-serve --release --bin git-ingest -- <path-to-hugit-checkout> hugit
```
Expect: `ingested N objects → CAS (<tenant>/hugit), wrote refs.json + oid-index.json`. Re-run it whenever
the launch repo's refs move (it's idempotent — CAS dedups, manifests overwrite). The R2 RW grant is one-shot
for ingest; the serve cred stays read-only.

### Step 2 — wire the serve env quartet + rebuild (the R2-pattern forward)
Forward this quartet from Worker secrets into the engine container (`engine-worker/index.js` `envVars`,
all-or-nothing like the R2 quartet), using the **`cas:r`** PAT:
```
HUGIT_SERVE_CAS_URL        = https://corelink-api.humangr.com
HUGIT_SERVE_CAS_TENANT_ID  = <tenant from the creds file>
HUGIT_SERVE_CAS_PAT        = <cas:r (read-only) PAT from the creds file>   # set as a Worker secret
HUGIT_SERVE_CAS_REPO       = hugit
```
The existing `HUGIT_SERVE_R2_*` cred already on the engine covers the manifest reads (same bucket). Then
bump `ENGINE_CACHE_BUST` + `npx wrangler deploy -c engine.wrangler.jsonc`.

**Selection precedence (state.rs `from_env`):** `HUGIT_SERVE_CAS_URL` set → CAS mode (git objects from
CAS); else `HUGIT_SERVE_GIT_DIR`; else blob/edit/clone 404. So setting the quartet flips the engine onto
CAS with no other change.

### Smoke (proves it live)
```sh
git clone https://engine.githugr.com/hugit /tmp/hugit-clone   # → succeeds, real history
curl -sS 'https://engine.githugr.com/v1/repos/hugit/blob/README.md' -H "Authorization: Bearer <engine-token>" | head
```
`git clone` succeeding = blob/edit + the upload-pack wire all serving from CAS, with BLAKE3 + git-SHA-1
double-verify on every object. `git push` stays 404 by design (later wave).

## Integrity you inherit for free
Every object is CoreLink-content-verified (BLAKE3) AND re-verified against its git SHA-1 on load (the loader
fails closed on any mismatch — the engine refuses to start rather than serve a poisoned object). Cross-tenant
dedup + GDPR 410-erase come from the CoreLink CAS.

— hugit TL · routed via owner. Ping me if the ingest or the smoke shows anything off.
