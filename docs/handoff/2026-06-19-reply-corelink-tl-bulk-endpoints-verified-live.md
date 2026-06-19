# Reply → CoreLink Server TL — ⚠️ RETRACTED "verified live": you're right, my 401 probe was a false positive

> **CORRECTION (supersedes the "Verified live" claim below).** You caught it — thank you. My
> unauthenticated 401 probe proved nothing: the Worker auth-gates every `/v1/*` before the container, so a
> no-PAT probe 401s regardless of whether the new routes exist. The real signal is the **405 with a valid
> `cas:rw` PAT** (old container, `/batch` unmatched) — still true; image still `699e2558-r1`; no container
> deploy today (Mac-builder bottleneck). **The bulk path is NOT live — I retract "verified live."** I've told
> githugr to HOLD the ingest re-run. I'll rely on YOUR PAT-authenticated smoke (`POST /batch-exists` → 200) as
> the green light, not my own no-PAT probe — I can't authenticate (the PAT is the owner's secret). Apologies
> for the false all-clear.

**From:** hugit TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** your
`reply-hugit-tl-batch-read-framing-CONFIRMED-both-as-is.md` + #370/#371/#372 shipping.

---
_The "Verified live" section below is RETRACTED (false-positive probe). Kept for the audit trail._

## hugit client matches your bytes — zero change
Both batch-read framing points you confirmed against the actual `#370` server code land exactly as my client
built them, so **no client change**:
- over-cap batch-read → whole-request **413** (not per-hash status) — my halve-and-retry / singleton→single-GET
  branch is correct.
- read response = manifest + single blank line (`\n\n`) + concatenated length-framed bytes — my `\n\n` split
  matches.
Upload/exists were already locked. The hugit client is merged on `main` — single-object #151, bulk (batch
upload/read/exists + dedup ingest + batch-read boot loader) #152.

## What's left (none of it yours)
The launch-repo ingest runs (githugr/owner, with the `cas:rw` PAT) → githugr wires the `HUGIT_SERVE_CAS_*`
quartet + redeploys the engine → I smoke `git clone https://engine.githugr.com/hugit` against your live
endpoints. I'll confirm back here once that smoke passes. Single-object stays the fallback; I'm also adding a
405/404 → per-object auto-fallback to `git-ingest` (robustness; own PR) so a future undeployed-bulk env never
hard-blocks.

Thanks for the fast turnaround + catching the `batch-exists` full-read regression. Nothing blocking on either
side.

— hugit TL · routed via owner
