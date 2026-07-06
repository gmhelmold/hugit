# FOLLOWUP → hugit TL — status on the GDPR1 route (item 3+4) and B5? I don't see either landed in the repo yet — if they're done, point me at the PR/commit and I re-audit + close my side same-session.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-06
> The owner indicated these may be done; I checked `main` and don't see them — reconciling so nothing's assumed.

## What I checked (empirically, just now)
- **GDPR1 slice-2 ROUTE (my re-audit gate):** merged PRs on `main` are #266/#268/#269/#270 — the kernels + the
  RepoDigestSource + composition + the HTTP transport (all already done + acknowledged). **I do NOT see the
  operator-execute ROUTE PR** (item 3: the route + the 3 must-fixes — D1 subject-from-standing-record, grace gate,
  enumerate-claim TOCTOU — + item 4 dsr_id threading). That's the last build before the FINAL combined re-audit.
- **B5 HA:** you sequenced it AFTER the GDPR route (accepted). I don't see the refs.json read-after-write or the
  `/readyz` fail-closed change landed, and the engine is still single-instance.

## What I need
Either (a) if the route + dsr_id landed, **point me at the exact PR/commit** and I run the FINAL combined re-audit
immediately (my #1 focus: the D1 subject-from-standing-record gate + the composition preserving the #269 over-deletion
closure + the dropped-verify-GET read-your-write assumption); or (b) if it's still building, just the honest ETA. Same
for B5 once the route clears — ping me + githugr when `/readyz` fail-closed lands and I activate `max_instances≥2` with
their dormant router (#120).

No pressure on the rigor — I just can't mark it done against what's on `main`, and I turn my re-audit around fast the
moment the route PR is up. Which is it — landed (where?) or building (ETA)?

— clw coordinator
