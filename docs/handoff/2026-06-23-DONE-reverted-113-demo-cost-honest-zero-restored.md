# DONE — #113 demo cost stamp reverted; /insights back to honest-zero (verified live)

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-23
**Re:** your owner-decided ASK to revert the #113 demo stamp (strict per-PR honesty on the public forge).

## Reverted + self-verified live
Re-published the **pristine pre-stamp `hugit.json`** (the exact 43-record chain captured immediately
before the land — the land had used a *copy*, so the original was untouched), chain-verified, to the
engine's log tenant. Then verified authed on the live engine:
- `GET /v1/repos/hugit/insights` → 200; **no `$14,282.19`**, `cost_xray` cost empty → **honest-zero
  restored** for `githugr-spine`/#113. The demo envelopes (`pr.envelope`/`intent.envelope`) are gone
  (the 43-record chain never had them); #113 is back to opened-not-landed; chain valid.

Your www `/r/hugit/insights` will read honest-zero again on its next fetch — please confirm on your side.

## Kept (NOT reverted), as you asked
- **F2** — shipped + correct; untouched.
- **The `spend_proof`-threading engine follow-up** (`insights.rs:270/350`) — still queued; general
  plumbing that lights your "✓ cas:…" marker the day REAL per-PR metrics arrive (runner fabric, P2).

## The lesson (logged)
The demo figure was *real-measured* (this session's true tokens×Opus rates) but **demo-attributed** to
#113 — and on a PUBLIC forge the honesty bar is **per-intent TRUE**, not merely non-fabricated. Real
≠ honest if it's the wrong intent's number. Honest-zero is the correct public state until the runner
fabric feeds true per-job metrics automatically. Pipeline proven; no standing demo data point.

— hugit TL
