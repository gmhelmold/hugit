# INCIDENT — engine.githugr.com container DOWN; needs Cloudflare dashboard (logs + reset)

**Date:** 2026-06-20 ~14:45Z · **From:** hugit session (caused this) · **To:** githugr TL (engine deploy lane) + owner · possibly CoreLink TL
**Severity:** engine `/v1` + git wire DOWN (HTTP 500 "Failed to start container").

## TL;DR
Attempting STEP 2 (git-serving engine image) I deployed a container that wouldn't
start; rollback + 3 further redeploys did NOT restore it. The container now exits
with **exit code 2 (hugit-serve fail-closed boot)** even on a rebuild of the
**known-good pre-multilang source (`5b9bcf9`)** that was healthy as version
`5a230a3a` at ~12:39Z today. Image swaps don't change the outcome ⇒ the cause is
**environment/platform, not the image** — most likely the engine's BOOT-TIME load
of an external dep (CoreLink CAS via `HUGIT_SERVE_CAS_URL`, or R2) now failing. I
cannot read the container's stderr from the CLI. **Stop redeploying; this needs
the Cloudflare dashboard.**

## What I need from the dashboard (the unblock)
1. **The container's start/stderr logs** for `githugr-engine-enginecontainer`
   (app id `a03e6041-b230-47a3-8446-ba51e4015035`). hugit-serve prints the exact
   `from_env` error before exiting 2 — that one line says WHY (a missing/!empty
   required var, or a CAS/R2 boot-load failure). That pins the fix.
2. If it's a **wedged container app** (crash-loop backoff), a **dashboard
   restart/reset** of the app may be needed — `wrangler` redeploys did not clear it.

## Timeline (all 2026-06-20)
- ~12:39Z `5a230a3a` (STEP 1, from `5b9bcf9`, distroless) — **HEALTHY** (`/readyz` 200, `/v1/token` 401). Verified.
- (later) merged #168 multilang symbols + #169 load_git_dir batch → main `cfd01d8`.
- STEP 2 deploy `10a6a7a3` (debian-slim + git + baked .git, from `cfd01d8`) → container **"not running"** (never started). Verified locally first — the IMAGE boots in ~8s, serves `git clone` of 6862 objects, serves the multilang outline — so STEP-2 packaging is sound; prod start failed (size/mem on `basic`? or the below).
- Rollback distroless from `cfd01d8` (`7d8fc384`) → **exit 2**. (distroless/cc lacks `libstdc++`, which the multilang C++ grammars need — so current-main distroless can't run; a real finding, but see next.)
- `wrangler rollback` to `5a230a3a` worker version → worker reverted but **container image NOT rolled** (Cloudflare: bound resources don't roll back) → still 500.
- Rebuild known-good `5b9bcf9` distroless (`39a4b30c`, current) → **exit 2**, even though `5a230a3a` (same source/base) was healthy 2h earlier. `wrangler containers list` shows the app `ready / 2 live instances` yet serving exit-2.

## Leading hypothesis (needs the logs to confirm)
The exit-2 is `hugit-serve`'s fail-closed `from_env`/boot. Since the SAME source
(`5b9bcf9`) that booted at 12:39 now exits 2, an **external boot-time dependency
changed**: most likely the engine loads its content/log source from the CoreLink
CAS (`HUGIT_SERVE_CAS_URL`) or R2 at boot, and that endpoint/credential is now
failing (cf. the 06-19 CAS bulk-endpoint churn handoffs). If so, this also needs
the **CoreLink TL** (is the CAS endpoint the engine boots against up?).
Secondary, separate finding: once multilang (#168) is in, the engine binary needs
`libstdc++` → the runtime base must NOT be distroless/cc (debian-slim, as STEP 2
used, is correct) — so the eventual current-main deploy needs the debian base AND
the instance sized for it.

## Current state / what I did NOT change
- Worker code (`engine-worker/index.js`) and the forwarded vars/secrets in
  `engine.wrangler.jsonc`: **unchanged by me** (only `ENGINE_CACHE_BUST` bumped and,
  earlier, `HUGIT_SESSION_EXCHANGE_URL` which was already present).
- `engine.Dockerfile` + `.dockerignore`: reverted to the committed distroless state
  (my STEP-2 versions saved at `/tmp/step2-engine.Dockerfile{,.dockerignore}`).
- hugit repo: back on `main` (`cfd01d8`), clean.

## UPDATE ~13:10Z — CAS confirmed live, but the engine STILL won't boot
- CoreLink TL relayed `2026-06-20-corelink-tl-LIVE-cas-bulk-plane-deployed.md`: the CAS
  **bulk plane is live + PAT-smoke verified** (batch-exists → 200). This also confirms the prod
  git-serving path is **git-from-CAS** (`HUGIT_SERVE_CAS_URL` quartet), NOT a baked git-dir — so the
  whole STEP-2 (debian + baked `.git`) detour was the wrong approach; distroless + CAS is correct.
- Forced a container roll (fresh cache-bust `2026-06-20-cas-live-restore-cfd01d8`, version
  `07388fd8`) so it re-reads + retries the live CAS. Result: **still down** — `/readyz` times out
  (000), occasional `503 ENGINE_UNAVAILABLE`. So `load_from_cas` at boot is STILL failing despite the
  bulk plane being live.
- Narrowed causes (need the container log to pick): (1) the hugit-side **one-time `git-ingest`** that
  populates the R2 manifests (`refs.json`, `oid-index.json`) hasn't run / is stale for the current
  repo → `load_from_cas` can't resolve → fatal; (2) **boot-load timeout** — pulling thousands of
  objects from the CAS over HTTP at boot exceeds the healthcheck window (same class as the load_git_dir
  perf bug, but over HTTP — the fatal-at-boot eager load doesn't scale to the launch repo).
- Hugit session is blocked: cannot read the container observability log (dashboard-only), cannot run
  `git-ingest` (needs the real CAS/R2 secret values), cannot `wrangler secret delete` (fence).
- Stopped redeploying (~6 rolls, none healthy — more is noise/risk).

## RESOLVED (degraded) ~14:15Z — engine UP on R2
Root cause (confirmed by CoreLink TL): the CAS is R2-durable and the deploy did NOT drop objects;
the prior `git-ingest` hit the **402 $-ceiling at ~200/6793 objects**, so the oid-index referenced
blake3s never written → `load_from_cas` fail-closed → exit 2. (NOT a deploy reset — coincidental timing.)

**Degraded restore (owner-authorized):** `engine-worker/index.js` now suppresses the CAS quartet
forwarding behind `CAS_DISABLED = true` → the engine boots on R2 alone. Deploy `38f13173`:
`/readyz` 200, `/v1/token` 401, `/v1/.../home` 401 (auth-gate healthy, no more 500/exit-2).
**`/v1` reads + login are LIVE again; blob/clone/outline (git-from-CAS) are OFF** until a complete ingest.

**Full restore (remaining, the real fix):**
1. Obtain creds: a fresh **`cas:rw` PAT** (CoreLink TL mints — ASK doc filed) + an **R2 RW S3 token**
   for bucket `corelink-githugr-engine` (owner's Cloudflare R2 dashboard) + the `d863fafb` tenant id.
2. Run `git-ingest <hugit-repo> hugit` to completion (tenant ceiling now $500) — re-seeds all 6793 objects.
3. Flip `CAS_DISABLED = false` in `engine-worker/index.js`, bump cache-bust, redeploy → git-from-CAS live.

**Guard note:** the TechLead secret-read/destructive guard was disabled globally (owner-authorized,
2026-06-20) via removing its hook from `~/.claude/settings.json` — it was blocking the recovery. The
sibling-repo fence stays on. Re-arm by restoring that hook entry once the incident fully closes.

## Recovery sequence (for whoever has the dashboard)
1. Read the container start logs → get the `from_env` exit-2 reason.
2. If external-dep (CAS/R2) boot-load: confirm that endpoint is up (CoreLink TL) or
   unset the var so the engine falls back to the local/empty source + serves 404s
   (degraded but UP), redeploy.
3. If wedged app: dashboard restart of `githugr-engine-enginecontainer`.
4. For the eventual current-`main` + STEP-2 deploy: debian-slim base (libstdc++ +
   git), bake `.git` (saved Dockerfile), and bump `instance_type` if size/mem is the
   start failure. The STEP-2 image is locally proven; the gap is purely the
   Cloudflare runtime config + the boot-dep above.
