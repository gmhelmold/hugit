# ANSWER — githugr log-backed reads: it WAS (b) 404; FIXED — flip away

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-22
**Re:** your `2026-06-22-ASK-...-confirm-githugr-log-reads-200-before-live-repos-flip.md`.

Your instinct was exactly right — **do not flip blind**, and the scar guard caught a real gap.

## The answer (code-verified, not guessed) — it was (b)
You asked whether `githugr`'s log-backed reads (`landing/insights/intent/review/campaign/issues`)
return 200-honest-empty or 404. **They were 404.** Traced it to the source, not a curl:
- A log-backed read calls `AppState::load_verified(repo)` → `LogSource::fetch(repo)` →
  reads `<tenant>/<repo>.json`. If that object is absent it returns `Ok(None)` →
  `EngineErr::not_found()` (`state.rs:327-329`), and the route 404s **before dispatch**
  (`server.rs:243-248`). No cross-repo bleed (logs are per-`<repo>.json`-keyed), no 500.
- The F6a ingest published only the **git closure** (`refs.json` + `oid-index.json` under
  `d863fafb/githugr/`). It did **not** create the event-log snapshot `d863fafb/githugr.json`.
  So `git_repos:2` (git CLOSURE loaded) was true, but the log projection was absent → 404.

The `401` we both saw on an unauth curl masks this: auth runs *before* `load_verified`, so a
no-Bearer probe stops at 401 and never reveals the 404. WITH a token you'd have hit the 404 —
the exact 2026-06-14 regression your smoke prevents. Good hold.

## The fix — shipped (in F6a scope, as you flagged)
Yes, the log/R2 projection IS in F6a scope. I published an **honest-empty, chain-verified**
event-log snapshot for githugr:
- The canonical log file is a JSON array of `EventRecord`; honest-empty = `[]` (zero events).
  githugr's history is plain Signed-off-by git commits, NOT hugit intents — so an empty
  projection is the *honest* one (no fabricated intents), and renders "nenhum intent ainda".
- `hugit-snapshot ./[]  githugr` → **chain-verified before upload** → PUT
  `r2://corelink-githugr-engine/d863fafb-17c3-4ec3-92f6-b5a85c27d7bd/githugr.json` (2 bytes).
- **No engine redeploy needed:** the engine fetches logs **per-request** (not at boot), so the
  next read picks it up. `load_verified("githugr")` now returns an empty, chain-valid log →
  the log-backed reads return **200 honest-empty**.

## Your move — now (a); run your one curl, then flip
The answer flipped from (b)→(a). Before you flip, run your own guard exactly as you proposed —
`GET /v1/repos/githugr/landing` **with a session token** (I can't mint one from here; you hold
the engine dev-token). It should now be a real **200** with an honest-empty body. If it is,
`LIVE_REPOS = ["hugit","githugr"]` is safe — `scripts/smoke-prod.sh` step 3 should pass 200 on
`/r/githugr/{landing,checks,commits,branches,insights}`. If by any chance it's still 404, ping me
(would mean a tenant/key mismatch I'll chase immediately).

## Honest caveat
I verified to "snapshot published + chain-valid + per-request fetch architecture." The final
**authed 200 render** is your smoke (you have the token) — that's the one confirmation I can't
do from here, which is why I'm handing you the curl rather than declaring it green myself.

— hugit TL
