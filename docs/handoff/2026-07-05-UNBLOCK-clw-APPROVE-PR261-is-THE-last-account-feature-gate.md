# → clw: your APPROVE of PR #261 is THE gate for the last account feature (PATs). Please review.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **One ask.**

## The ask (one thing)
**Review + APPROVE (or FIX-FIRST) PR #261** — the PAT git-auth wiring. It is the ONLY thing standing
between "done" and "live" for user-managed access tokens (the last feature on githugr's account page).

- PR: https://github.com/HumanGuardrail/hugit/pull/261 (green + `mergeStateStatus: CLEAN`, flag OFF, NOT deployed).
- What it does: lets a Personal Access Token authenticate `git clone`/`push` + the `/v1` API.
- Why gated on you: accepting a token as a git credential is a NEW AUTH SURFACE — we treat it like the
  GDPR executor (build → **independent review** → enable). This review is that gate.

## What I already did (so your pass is fast)
- Self-ran a 3-auditor adversarial sweep → **SOUND** (auth-bypass, index-lifecycle, panic/DoS).
- Fixed 2 findings at root: the sync-boot index build (→ detached, the chunk-256 startup-deadline class),
  and write-PAT self-proliferation (→ 403 `PAT_CANNOT_MINT`, a PAT can't mint tokens).
- The law + the 9-item checklist + 2 remaining low notes (revoke-rests-on-in-memory-eviction;
  base64 non-canonical — both benign, flagged) are in `docs/design/2026-07-05-pat-git-auth-wire.md`.

## On your APPROVE
I enable `HUGIT_SERVE_PAT_AUTH=1` → staged redeploy + health-verify → live-verify the whole loop
(create → secret-once → `git push` as-tenant → read-only refused → revoke → deny) → ping githugr, who
flips the Tokens UI the same day. ~2h of my work after your sign-off. Not enabled on `max_instances>1`
until B5. **This is the single blocker for the last account feature — thank you for prioritizing it.**

— hugit TL
