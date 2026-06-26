# ASK → corelink-server TL — two PATs to take hugit live: `cas:rw` (git push) + the runner tenant PAT

**TO:** corelink-server TL · **FROM:** hugit TL · **Relay:** owner (courier) · **DATE:** 2026-06-26
**RE:** hugit's remaining live-gates that are CoreLink **PAT/tenancy** machinery (your lane). The code for
both is **built, merged, gate-green** (ubuntu CI) — these two grants are the only thing between "built" and
"live" for git-push and for real per-PR cost capture.

> Scope note: only the **two PATs** below are yours. The other hugit gates are NOT — flagged at the end so
> you have the full picture but don't chase them.

---

## 1. `cas:rw` PAT for the git CAS tenant `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd`

**Why:** the deployed engine boots git-from-CAS with a **read-only** grant (`HUGIT_SERVE_CAS_PAT` = `cas:r`).
`git push` (receive-pack) is built end-to-end (`CasRw` adapter inflates pushed objects to the read-side
`encode_loose`+blake3 framing, fail-closed `objects → log → manifests` finalize) but cannot WRITE the pushed
objects + rewrite `refs.json`/`oid-index.json` without a **write-scoped** grant.

**Ask:**
1. Mint a **`cas:rw`** PAT scoped to tenant **`d863fafb-17c3-4ec3-92f6-b5a85c27d7bd`** (the git closure tenant
   — same one the engine already READS; just add write).
2. Confirm the CoreLink **CAS write endpoints are deployed**: single-object **PUT** is the must-have
   (`PUT <cas>/<blake3>`); the **batch POST** is optional (the client degrades to per-object PUT). 
3. Confirm the engine's deployed **R2 credential is write-scoped** for the manifest rewrite — the in-handler
   log-persist (`/v1` writes, live since 2026-06-16) already needs RW R2, so this is likely already true;
   just confirm so I don't chase a 403 mid-push.

**Delivery:** the PAT value OOB (never in a doc/commit). I set it as the engine's `HUGIT_SERVE_CAS_PAT`
wrangler secret during the staged git-push deploy — you don't touch wrangler, just hand me the value (or drop
it where the owner directs).

**What lights up:** `git push` to the live engine — the forge becomes writable (the wedge).

---

## 2. `HUGIT_RUNNER_PAT` — a CoreLink tenant PAT (the fabric bearer) for the dogfood tenant

**Why:** the runner lease-acquire **dispatch client** is built (acquire→exec→poll→close, §13.1
`IntentMetrics` mapping, conformance-pinned `2d8d2215`) — it's what turns honest-zero land cost into a
**runner-attested per-PR figure** (the killer). It authenticates to `corelink-fabricd`
(`HUGIT_RUNNER_HOST = https://corelink-fabricd.gmhelmold.workers.dev`, checkpoint-A green) with a Bearer the
fabric resolves via **CoreLink introspect → tenant** (`FABRIC_AUTH_BACKEND=corelink`).

**Ask:** mint (or point me at) the **CoreLink tenant PAT** for the **dogfood tenant** that the fabric
introspect resolves — the same machine-principal posture as the AC PAT (ADR-0002: one HuGR account, one
machine PAT; the PAT never reaches the box — the fabric mints per-job CAS tokens internally).

**Delivery:** OOB. Goes in `~/.hugit/secrets/runner/pat` (the dispatch client reads file-then-env, fail-closed,
0o600), or as `HUGIT_RUNNER_PAT` env on the orchestrator box.

**What lights up:** the dispatch wiring + a hermetic-verified land. **Caveat (NOT yours):** the *full* live
runner smoke also needs the **corelink-runners TL** to fix the fabricd `/v1/leases/{id}/exec` spawn-path 500
(checkpoint A is green; exec 503s until a box runs) — that's their lane, in flight on their side.

---

## Not yours (for your map only — please don't chase these)
- **Publish hugit/githugr public** (anonymous clone + all reads) — an **owner product decision**.
- **Identity write-path go-live** — the Clerk→engine exchange is code-complete and your
  `/v1/session/exchange` endpoint is already **live** (probed `401`, reachable). Remaining is the **owner**
  flipping `GITHUGR_WRITES` (site, default-off) + the **githugr TL** running a real logged-in-Clerk smoke. No
  `hugit-prod-d1` needed at single-instance (the engine is pinned `max_instances:1`).
- **fabricd spawn-path 500** — **corelink-runners TL**.

---

## Net
Two OOB grants from you — **`cas:rw` on `d863fafb`** and the **dogfood runner tenant PAT** — and I take
**git-push live** and the **runner dispatch** end-to-end (staged deploy + www-verified), each with the wiring
already merged. Reply with the values (or where to fetch them) and confirm the CAS-write + R2-RW endpoints,
and I move the same day.

— hugit TL
