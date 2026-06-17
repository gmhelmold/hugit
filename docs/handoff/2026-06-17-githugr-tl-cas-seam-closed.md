# Heads-up → githugr TL — the R2 CAS seam is CLOSED (write durability hardened)

> 2026-06-17 · from: hugit TL · FYI, not blocking. The last disclosed P2 engine seam
> on the write path — R2 compare-and-swap — is now implemented + merged to `main`
> (`9b8f6b8`, PR #137). Your deployed engine picks it up on the next rebuild from `main`.

## What changed

The write-door's `load → mutate → persist` cycle was last-writer-wins: a concurrent
writer (the snapshot uploader today; multi-instance scale tomorrow) could silently
clobber another request's records AND its idempotency-ledger entry. It is now a
**compare-and-swap**:

- `persist` does an R2 conditional PUT — `If-Match: <etag>` (the head captured at load)
  / `If-None-Match: *` for create. R2's **412 Precondition Failed** → a typed conflict →
  the write-door reloads + retries (bounded). Same-key race → one effect + replay;
  different-key race → both survive; sustained contention → honest transient 503.
- Proven against LIVE R2 (`tests/r2_cas_live.rs`): stale `If-Match`→412, correct→200,
  non-destructive. SigV4 untouched (the conditional header is sent unsigned, spec-valid).

## Does it affect you?

- **No behavior change for the happy path** — single-instance writes are unaffected; the
  guard only changes the outcome of a genuine concurrent-head collision (previously a
  silent drop, now a transparent retry or, at worst, a retryable 503).
- **No client/contract change** — same `/v1` shapes, same `Accepted`, same error envelope.
  `write-smoke.sh` passes unchanged.
- When you next rebuild `engine.githugr.com` from `main`, you get it for free. No action
  required; flagging it so you can mark the "R2 CAS/compare-and-swap" P2 seam CLOSED on
  your side.

Bonus in the same PR: a `fix(deps)` cleared two newly-published `git2` advisories
(RUSTSEC-2026-0183/0184) in a test-only dep, keeping the advisory gate green.

— hugit TL
