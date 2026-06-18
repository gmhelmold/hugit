# Deploy cutover — make the W3/W5 work LIVE (engine redeploy + git-serving image)

**From:** hugit techlead · **Date:** 2026-06-18 · **To:** owner (Cloudflare account) + githugr TL (engine image lane)
**Why:** 8 PRs landed on hugit `main` today (#145–#150 + docs). The deployed engine
(`engine.githugr.com`) still runs the **June-17 image** (`ENGINE_CACHE_BUST=2026-06-17-142-…`,
commit `5782168`) — **none of today's work is live yet.** This is the exact, chewed cutover.

## Probed live state (2026-06-18)
| Surface | Live now | Note |
|---|---|---|
| `POST /v1/token` (login exchange) | ✅ 401 (wired) | `HUGIT_SESSION_EXCHANGE_URL` already in `engine.wrangler.jsonc` `vars` |
| `/v1` reads+writes | ✅ 401 (auth-gated) | deployed, but at commit `5782168` (pre-Wave-A) |
| blob/edit reads | ❌ | old image (pre-#148) |
| `git clone` (`/hugit/info/refs`) | ❌ 404 | old image + container has no git dir |

---

## STEP 1 — Redeploy the engine from current `main` (OWNER, ~5 min, no new infra)

This rebuilds `hugit-serve` from the current `../hugit` workspace and rolls a fresh container.
Lights up **everything merged since `5782168`** that is log/R2-backed (Wave-A reads, issue/verdict
parity, etc.) and reconfirms real login. It does **NOT** light up blob/edit/`git clone` (those need
STEP 2 — a git dir in the container).

The engine deploy lives in **`../githugr`** (engine.Dockerfile + engine.wrangler.jsonc). The image
is content-cached, so a deploy that doesn't change the image bytes won't restart the container — you
**must** bump the cache-bust. Two edits + one command, all in `../githugr`:

1. Edit `engine.wrangler.jsonc` → `containers[0].image_vars.ENGINE_CACHE_BUST` to a new value, e.g.:
   ```jsonc
   "image_vars": { "ENGINE_CACHE_BUST": "2026-06-18-w3w5-main" }
   ```
2. From `../githugr` (with your Cloudflare login — `npx wrangler login` if needed):
   ```sh
   npx wrangler deploy -c engine.wrangler.jsonc
   ```
   This builds the image with `image_build_context: ".."` (so it picks up the current `../hugit`
   crates) and rolls a fresh container.

**Smoke (run after the deploy reports success):**
```sh
curl -sS https://engine.githugr.com/readyz            # → 200 ready
curl -sS -o /dev/null -w '%{http_code}\n' -X POST https://engine.githugr.com/v1/token   # → 401 (wired)
```
A real authenticated read smoke is `scripts/write-smoke.sh` in this repo (needs a real bearer).

> ⚠️ The build pins `Cargo.lock` (`--locked`) + `rust:1.96` by digest. No change needed — current
> `main`'s lockfile is committed. If the build fails on a new transitive dep, ping hugit.

---

## STEP 2 — Light up `git clone` + blob/edit (githugr TL: engine image change)

These three serve **file content from a git repo on the container** (`HUGIT_SERVE_GIT_DIR` →
`hugit-serve` shells `git rev-list/cat-file/for-each-ref`). The current runtime is
**`gcr.io/distroless/cc`** (no shell, no `git`) and the data source is **R2 logs** (no git dir). So
blob/edit/clone will 404 even on the STEP-1 image until the container gains a `git` binary **and** a
real repo checkout. This is a real `engine.Dockerfile` change (a base/packaging decision), not an env
flip — hence the githugr TL's lane. Required changes to `../githugr/engine.Dockerfile`:

1. **A `git` binary in the runtime stage.** `distroless/cc` has none. Options, cheapest first:
   - Switch the runtime base to `debian:bookworm-slim` + `RUN apt-get install -y --no-install-recommends git ca-certificates` (simplest; loses distroless's minimalism — a security/size call for the owner).
   - OR copy a static `git` + its template/exec-path into distroless (smaller attack surface, fiddlier).
2. **A real hugit repo in the image** at a fixed path, e.g. `/app/repo`. The build context is the
   parent `HuGR/`, so either `COPY hugit/.git /app/repo/.git` (+ `git -C /app/repo checkout`) — note
   `.git` must NOT be excluded by `engine.Dockerfile.dockerignore` — or `git clone` the repo at build.
   Bare or non-bare both work (`for-each-ref`/`rev-list`/`cat-file` operate on either).
3. **Set the env var + bump the cache-bust:**
   ```dockerfile
   ENV HUGIT_SERVE_GIT_DIR=/app/repo
   ARG ENGINE_CACHE_BUST=2026-06-18-git-serving
   ```
4. Redeploy as in STEP 1.

**Smoke (after STEP 2):**
```sh
git clone https://engine.githugr.com/hugit /tmp/hugit-clone   # → succeeds, real history
curl -sS 'https://engine.githugr.com/hugit/info/refs?service=git-upload-pack' | head -c 64  # → "# service=git-upload-pack" pkt-line
```
> `git clone` is gated on the repo being **publicly readable** (the same `authorize_read` predicate
> as `/v1` reads). The `hugit` launch repo's `repo.meta` visibility must be public (it is, for the
> read path). `git push` is intentionally 404 (a later wave).

> **Architectural note (for later, not blocking):** STEP 2 bakes the repo into the engine image — a
> pragmatic single-tenant bridge. The "full real" multi-tenant path serves git objects from the
> CoreLink CAS tenant (P2), not a baked checkout. Fine to bake for the launch repo now; revisit at P2.

---

## What this does NOT cover (still owner/infra, unchanged)
CoreLink P2 tenant (live CAS/AC) · runner fabric live · GitHub App + mirror · multi-tenant Clerk +
`hugit-prod-d1`. Per the standing critical path (`CLAUDE.md`, the honest audit). `symbol`/W6 and
`git push` are future hugit code waves.
