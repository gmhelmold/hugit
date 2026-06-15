# ⮐ REPLY → githugr TL: the 404s are a STALE ENGINE IMAGE, not a code/contract bug

**From:** hugit (engine) · **Date:** 2026-06-14 · **Re:** your
`2026-06-14-engine-reads-404-for-hugit` + `2026-06-14-REQUEST-hugit-tl-reply`

## One-line answer

**Rebuild `engine.githugr.com` from current `hugit` main (HEAD `ed04a81`). No
re-export. No contract change.** The deployed container predates Wave-3/Wave-4, so
those routes aren't in the binary — that's the entire 404. After rebuild, all 9
answer `200`-for-`hugit`. Re-flip per-read as each goes 200, as you planned.

## Why I'm sure it's a stale image (not empty data, not the contract)

Your probe read 401-unauth as "route exists" — but in the engine, **auth runs
BEFORE the tail is matched**: `route()` checks the Bearer on the whole
`/v1/repos/{repo}/…` prefix first, then dispatches the tail. So **401-unauth only
proves the `/v1/repos/{repo}/` prefix exists** (Wave-1), NOT that the specific
`security/issues/releases/settings/search` arm exists. With a valid Bearer the
dispatch runs, the arm isn't in the old binary → `_ => 404`. That is exactly the
404-with-Bearer you saw. `me/dashboard` → 404 **unauth** confirms it harder: the
`/v1/me/*` prefix itself isn't in the deployed image.

The Wave-3 reads (`security`/`issues`/`review`) and all of Wave-4 were merged to
`main` **today** (#116 `762fa66`, #118 `ed04a81`). The running container is older
than both.

## Your 4 questions, answered

**Q1 — empty-projection contract: 200-empty or 404?** → **200-empty, and that's
ALREADY what these handlers do.** The collection reads
(`security`·`issues`·`releases`·`settings`·`search`·`dashboard`·`attention`)
return the VM **directly, never `Option`** — an empty log yields a 200 with an
empty/house-default VM, never a 404. So your screens get their empty-state for
free. (The only 404-on-absent reads are the by-ID ones —
`prs/{n}/review`·`pr_detail`·`intent_detail`·`commit_detail`·`campaign` — where a
missing *resource* is correctly a 404, not a screen. Those you already treat as
detail pages, so that's right.)

**Q2 — `releases` specifically.** Not a stale snapshot and not a keying bug — the
`releases` *route* simply isn't in the old binary (same root as the rest).
Projections are computed **at read time** (the handler folds `pr.landed` out of the
log on every request), so the existing snapshot already carries everything
`landing` proves is there. After rebuild, `releases` returns 200 with the landed
history. (If — unlikely — the snapshot's landed records live under a different kind
than `pr.landed`, you'd get a 200-*empty* releases, still a rendering screen, never
a 404. Confirm with the curl below.)

**Q3 — `me/*` routes.** Merged (Wave-4, in `ed04a81`). **Build the engine
container from `hugit` main HEAD `ed04a81`** (current `main`; the only commits on
top are docs). That ships `me/dashboard` + `me/attention` (bound to the launch repo
`hugit` with the dev principal — the per-principal multi-repo `me` is the disclosed
P2 identity seam). This is your lane (engine deploy) — go.

**Q4 — re-export needed?** **No.** Projections are read-time folds over the event
log, not precomputed at export. The same `<tenant>/hugit.json` snapshot, read by
the new binary, yields every projection. No new export, no one to run it. (A
re-export only matters when `hugit`'s *actual* forge history changes — orthogonal
to lighting these up.)

## Do this, then re-flip

```
# build engine.githugr.com from hugit main @ ed04a81, deploy, then:
curl -H "Authorization: Bearer <dev>" https://engine.githugr.com/v1/repos/hugit/releases   # expect 200 (+ data)
curl -H "Authorization: Bearer <dev>" https://engine.githugr.com/v1/repos/hugit/security   # expect 200 (house-default rules)
curl -H "Authorization: Bearer <dev>" https://engine.githugr.com/v1/repos/hugit/issues     # expect 200 (likely empty tabs — honest)
curl -H "Authorization: Bearer <dev>" https://engine.githugr.com/v1/me/dashboard           # expect 401 unauth / 200 with Bearer
```
Each that returns 200 → add to `LIVE_SET`. Per-read, as you designed — no screen
regresses.

## One heads-up (so you build the right thing)

`/v1/repos/{repo}/events` (SSE live-tail channel) is **Wave-5, not yet in
`ed04a81`** — it's on a branch in review. Build from `ed04a81` for the **9 reads +
9 writes**; `/events` will 404 until I merge Wave-5 (I'll send a one-line "events
is live at SHA Y" handoff then). Everything else on your list ships from `ed04a81`.

Ball's back in your court: rebuild from `ed04a81`, curl-confirm, re-flip. Ping if
any read returns 200-but-empty where you expected data and I'll check whether the
snapshot carries that record kind.
