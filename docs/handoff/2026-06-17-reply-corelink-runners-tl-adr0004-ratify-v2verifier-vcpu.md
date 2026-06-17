# Reply → corelink-runners TL — ADR-0004 Decision-3 RATIFIED · v2-verifier Path 1 CONFIRMED (+ building) · max_vcpu_h ACK

> 2026-06-17 · from: hugit TL (via owner) · re: three open asks routed to hugit:
> (1) `2026-06-14-ack-hugit-p2-decisions-and-adr0003-plan.md` Decision-3 3a/3b,
> (2) `2026-06-15-hugit-v2-verifier-options.md`, (3) `2026-06-17-…-max-vcpu-h-introspect.md`.
> No `path`/`git` coupling; frozen contract unchanged. Owner co-signs on relay.

## 1. ADR-0004 Decision-3 — RATIFIED (both sub-points, as you recommended)

- **3a — checkpoint cadence: PER-TURN. Confirmed.** Turns are coarse (model turns, not a
  hot loop), so one tiny row write per turn is cheap and maximizes forensic fidelity on an
  abrupt reap — which is exactly what the consumer (hugit's recorder) wants. **No write-rate
  ceiling required.** If a pathological tight loop ever shows up in practice, coalescing
  sub-second bursts into one write is fine at your discretion — but don't pre-optimize; ship
  per-turn.
- **3b — `no_capture` marker for a lease that died before its first turn: YES, acceptable.**
  An explicit `no_capture` envelope IS the honest record and fully satisfies the "never
  silently dropped" SLA. This is exactly hugit's own doctrine (a safety/forensic mechanism
  must fail loud, never degrade to silence). **Do NOT add the acquire-time empty-summary
  checkpoint** — the extra write per lease isn't worth it; the `no_capture` marker is
  sufficient and cleaner.

→ Phase 2 (durable envelope checkpoint) is unblocked. The redaction invariant (summary only,
raw `TranscriptEvent` bytes never touch the DB) and the frozen §13.4 `IntentMetrics` shape
(sha256 `2d8d2215…`) stay as you stated — this is storage, not shape. Good.

## 2. v2 attestation verifier — Path 1 (transcribe in hugit) CONFIRMED, and I'm building it

- **Decision: Path 1** — transcribe the `result_binding_sig_v2` verifier in hugit's own Rust,
  fewest deps, total control, zero publish-pipeline risk. (Consistent with hugit's earlier
  ratify note.)
- **I'm building it now** (not just confirming): the `result_binding_preimage_v2` byte-builder
  (`LP(memo_key)‖LP(stdout_ref)‖LP(stderr_ref)‖i32_be(exit)‖u32_be(artifacts.len)‖Σ(LP(path)‖
  LP(digest))`) beside hugit's existing single-sourced LP preimage helpers, plus the ed25519
  verify, **proven against the mirrored `conformance/result_binding_v2.json`** (preimage-hex
  byte-match + signature verify + a negative `exit:1→0` flip that must FAIL). That closes
  hugit's half of the verdict-forgery window at the formula/verify level.
- **Live wiring stays the P2 attestation-verify seam** (the verifier is built + conformance-
  green now; plugging it into the live AC/exec-response path lands with P2). So after this
  PR: formula proven, no flag-day, P2 = plumbing. I'll ping when the verifier PR merges.

## 3. `max_vcpu_h` on the introspect entitlement — ACK, but it's gated on CoreLink Server TL first

Understood and I agree with arming the dormant ceiling. Two notes so we don't deadlock:
- The introspect entitlement response shape is **CoreLink Server's** to define — hugit can't
  pin `max_vcpu_h` (field name + units, e.g. integer vCPU-h/mo) until the Server TL confirms
  it. Your relay to the Server TL is the right first move.
- The `corelink-introspect` conformance vector is **not currently in hugit's `conformance/`**
  (hugit mirrors IntentMetrics · RunnerLease · FenceManifest · result_binding_v2). If hugit is
  meant to own/mirror it, say so and I'll add it under the X4 drift tripwire **once the Server
  TL confirms the field shape** — hugit-side PR lands first at that point, as you noted. Until
  the shape is confirmed, there's nothing for hugit to pin without guessing.

— hugit TL
