# DISPATCH → hugit TL — Go-Live engine track (private clone + self-service seam + value-prop)

> **From:** clw TL (cross-team go-live coordinator) · **To:** hugit TL · **Date:** 2026-06-30
> **Master:** `corelink-workspaces/docs/GO-LIVE-STACK-ROADMAP-2026-06-30.md` (dual-audited). Builds on your
> `docs/roadmap/2026-06-30-kungfu-pilot-readiness-roadmap.md`. Reply in this `docs/handoff/` folder; I sweep it.

## ⛔ Scope (owner override, 2026-06-30) — NO tradeoffs / gambiarras
The user is an **arbitrary real user**; deliver **the complete product, exactly as promised, 100% working**.
Every item is launch-gating and ships done-properly.

## Your track
1. **B4 [P0 — UNCONDITIONAL] — authed clone of a private repo.** `git-upload-pack` passes an empty principal —
   **zero Bearer extraction on the clone route** (`crates/hugit-serve/src/git.rs:280`, accuracy-audit-verified),
   so a private repo is uncloneable by anyone, incl. its owner. Specced + owner-ratified, not built. Private
   repos are a **promised** feature and an arbitrary user WILL make one, so "if private" is gone — build it.
   Exit: `git clone` of a private repo works with a Bearer; an unauthorized clone is denied (fail-closed).
2. **B3-engine [P0] (with githugr) — self-service repo ingest seam.** On-demand `POST /v1/repos` ingest must seed
   the `<tenant>/<repo>.json` event-log the read-gate needs (today an ingested repo 404s; githugr can't create
   one). Exit: a freshly-ingested repo serves for its tenant.
3. **value-ci [P1 — still gating] — memoized-CI / union-tested landing queue LIVE.** The CheckDef→CI value prop
   (algorithm built; needs the live runner + AC substrate, which Track A/C are standing up). Exit: a real PR runs
   memoized checks + lands via the queue.
4. **Per-session `/v1/me/*` READ token (the seam githugr B1 needs).** githugr depends on this to scope reads to a
   real user's tenant. Exit: githugr can request a per-session read token; reads are tenant-scoped.

## Coordination note
The check-host (W6 / `clw hydrate --manifest-digest`) is a **fast-follow, not a launch gate** — it's default-off
and orthogonal; the runner path runs CI without it. The toolchain_ref=snapshot-digest contract you ratified
stands; the operational snapshot is downstream of the runner fabric going live.

## What I need back
Per-item status + your honest estimate on B4 (the authed-clone build) and the per-session read-token seam, since
githugr is blocked on the latter.
