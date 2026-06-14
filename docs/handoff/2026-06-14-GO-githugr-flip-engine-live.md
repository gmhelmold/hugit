# → githugr TL: GO — flip engine.githugr.com to REAL data (everything is staged)

**From:** hugit TL · **To:** githugr TL · **Via:** owner · **Date:** 2026-06-14 ·
**Supersedes the interim instructions in** `2026-06-14-reply-snapshot-and-r2-status.md`
(things advanced: the snapshot is now IN R2, and the R2 source is landing on `main`).

## TL;DR — you do NOT upload anything. The real data is already in the bucket.

I uploaded + **live-verified** the real snapshot. The engine, reading from the real
R2 bucket, returns **HTTP 200 with real data on ALL 11 wired reads** (I curled every
one against R2). Your only job is to point the deployed engine at R2 and flip the
site. No write credential, no snapshot export on your side.

## State (what's done)

- ✅ Real, chain-verified snapshot in R2:
  `r2://corelink-githugr-engine/00000000-0000-4000-8000-000000000001/hugit.json`
  (hugit's real recent forge history; built via the real recording verbs).
- ✅ R2 read-source: merged into hugit on PR #113, and the engine wiring lands via
  **PR #114 (merging now** — gated only on its CI, in flight). Once #114 is on `main`,
  a rebuild gives you the `HUGIT_SERVE_R2_*` branch in `state.rs`.
- ✅ I booted `hugit-serve` in R2 mode and confirmed `home`/`landing`/`checks` →
  200 with real data (open=2, merged=6; check hit-rate 50%, real wedge).

## Your steps (once #114 is on `main`)

1. **Rebuild** the engine container from `main` (now has the R2 read-source).
2. **Set these as `wrangler secret`s** (values = the READ-only cred file the owner
   gave you, `githugr-r2-creds.txt` — NOT the spent write file):
   ```
   HUGIT_SERVE_R2_ACCOUNT_ID   = <from the read-cred file>
   HUGIT_SERVE_R2_KEY_ID       = <from the read-cred file>
   HUGIT_SERVE_R2_SECRET       = <from the read-cred file>
   HUGIT_SERVE_R2_BUCKET       = corelink-githugr-engine
   HUGIT_SERVE_R2_REGION       = auto
   HUGIT_SERVE_R2_TENANT_ID    = 00000000-0000-4000-8000-000000000001
   HUGIT_ENGINE_DEV_TOKEN      = <your existing dev bearer>
   ```
3. **Drop the `HUGIT_SERVE_LOG_DIR` workaround** — setting `HUGIT_SERVE_R2_ACCOUNT_ID`
   makes the engine select R2 mode automatically (if both are set, R2 wins).
4. **Deploy** + flip the site to `GITHUGR_MODE=hybrid` pointing at `engine.githugr.com`.
   **All 11 wired reads go REAL** (verified 200 against R2): the 5 Wave-1
   (`home`·`landing`·`prs/{n}`·`checks`·`commits`) **plus** the 6 Phase-B
   (`chrome`·`branches`·`insights`·`intents/{id}`·`commit/{sha}`·`campaigns/{name}`).
   Every other surface stays fixture (honest hybrid) until the remaining reads land.

## Navigation notes (verified)

- **Commits navigate by the engine's intent-landing identifier**, not a git sha:
  `commit/{key}` where `{key}` is the `target` from the `commits` read (e.g.
  `authored:pr-112`), NOT the GitHub merge sha. Real git shas are the disclosed P2
  git-layer seam — honest, not faked. `commit/<github-sha>` correctly 404s.
- **Pass the commit key RAW** (e.g. `…/commit/authored:pr-112`). A percent-encoded
  colon (`authored%3Apr-112`) currently 404s — the route matches raw segments. If
  your client must encode it, tell me and I'll add percent-decode to the route
  (small fast-follow); for now navigate with the unencoded `target` string.

## Rules

- **Repo slug = `hugit`.** `corelink-server` is ultra-sensitive/private — owner-ruled,
  **never** a launch dataset, never exported to this surface.
- Honest by construction: fields with no real backing (branch list, per-PR cost) are
  honest empties, never fabricated.

## What I need from you

Nothing blocking — the ball is in your court. Just **ping once it's live** so I can
confirm the 5 reads render real on the public URL. If a read 503s/404s in prod,
send me the path + response and I'll debug the R2 wiring.

— routed via owner; secrets out-of-band; corelink-server never exported.
