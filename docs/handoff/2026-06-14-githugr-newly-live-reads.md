# Handoff → githugr TL: more `/v1` reads are now LIVE (flip the hybrid provider)

**From:** hugit (engine) · **Date:** 2026-06-14 · **Re:** close-the-product reads

Your hybrid provider runs "live on what we serve, honest fixture on the rest"
(`2026-06-13-hugit-phase2-close-the-product.md` §2). The engine now serves **9
more reads with REAL log-backed data** — switch these from fixture to live in the
provider's live-list (or, if the provider auto-detects via try-live-then-fallback,
nothing to do — they will light up on next deploy of `engine.githugr.com`).

## Now LIVE (real, not fixture)

**Wave-3 (write-backed):**
- `GET /v1/repos/{repo}/prs/{n}/review` → `ReviewVm` (verdicts + comment timeline; 404 if no such PR)
- `GET /v1/repos/{repo}/issues` → `IssuesVm` (latest `issue.transition` per id, 4 tabs)
- `GET /v1/repos/{repo}/security` → `SecurityVm` (house rules from `policy.set` + approved `erasure.decided`)

**Wave-4 (real log projections):**
- `GET /v1/repos/{repo}/settings` → `RepoSettingsVm` (rules + operator overrides)
- `GET /v1/repos/{repo}/releases` → `ReleasesVm` (`pr.landed` history, newest-first, `latest` pill)
- `GET /v1/repos/{repo}/search?q=<query>` → `SearchVm` (over prs/intents/issues/campaigns)
- `GET /v1/repos/{repo}/viewer-can` → `ViewerCanVm` (the real D14 authz capability matrix)
- `GET /v1/me/dashboard` → `DashboardVm`
- `GET /v1/me/attention` → `AttentionVm`

These join the already-live set (home · landing · checks · commits · pr_detail ·
chrome · branches · commit_detail · intent_detail · insights · campaign). **Total
now ~20 reads live** + the 9 Wave-2 writes.

## Two notes

1. **`/v1/me/*` scope.** `dashboard`/`attention` are identity-scoped. Until the P2
   Clerk seam resolves a per-principal repo set, the engine binds the **launch
   repo** (`hugit`) with the dev principal — so the data is REAL forge activity for
   that one repo; cross-repo aggregation fields are honest-default. When `/v1/token`
   (below) lands, the principal→repo resolution becomes real.

2. **Still honest-default fixture (by decision, not gaps):** the git-layer reads
   (`blob` · `compare` · `edit`) and identity reads (`account` · `profile` · `login`
   · `org` · `import` · `github_app` · `new-pr` · `knowledge`) stay fixture — they
   need the P2 git/CAS or Clerk seams. Keep them on the fixture side of the hybrid.

## Production seams — status

- **R2 read source:** DONE (live, `corelink-githugr-engine` bucket).
- **SSE `GET /v1/repos/{repo}/events?since=`:** designed + drafted (replay-then-close;
  true live-tail is the P2 architecture seam — the sync `tiny_http` loop can't hold a
  stream open). Lands next.
- **`POST /v1/token` (RFC-8693 Clerk exchange):** designed + drafted; security-critical,
  landing after a crypto-audit against authoritative vectors. Until then, the dev-token
  stub + dev-principal hold.

No action needed from you beyond the live-list flip (if your provider is not
auto-detecting). Ping if any VM shape mismatches a parity assert — the engine is
held to your frozen contract.
