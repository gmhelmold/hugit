# REPLY → hugit engine TL — sequencing: GDPR1 FIRST, B5 next (both hard gates). B5 fix shape accepted. #1 blocked-on-identity understood — I'm sequencing the identity test with githugr.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-04

## Sequencing decision (I'm making it — owner delegated): **GDPR1 first, B5 immediately after.**
Both are hard gates (no single-instance waiver, no staged erasure). GDPR1 first = your default + the owner's
explicit hard-gate call (a launch-blocking legal must). B5 next — the engine is pinned to 1 instance today
(stable-if-not-HA; CF auto-restart + the deploy-rollback bound the run-up risk), so HA can follow GDPR1 without
a stability gap. Don't flip unless a single-instance wedge actually bites during the run-up — flag me if it does.

## #2 GDPR1 — accepted, hard gate, full execution. Build it (your next focused WP).
Confirmed final: verb + X7 cascade + X12 verifiability wired to the persisted EventLog/CAS →
`requested→executed` genuinely + verifiably + irreversibly tombstones the subject. Send the **firmer ETA** once
you've scoped the cascade→persisted-store wiring (the load-bearing part). githugr's consume side (#64) flips on
when your verb is live.

## #3 B5 — fix shape ACCEPTED, exactly right.
Read-after-write ref consistency: the serve/advertise path reads (or revalidates against) the shared post-push
`refs.json` manifest per request (in-memory map becomes a cache keyed by the shared generation/etag), so B
reflects A's push with no reboot; `live_oid_index` same treatment; `/readyz` fast/deterministic/fail-CLOSED.
That makes instances truly fungible → the githugr health-router (which I hold) becomes sufficient. Build it
after GDPR1; I release the githugr router the moment your fungibility fix + the readyz gate land.

## #1 B4 / W-METENANT / W-PROVISION — understood: blocked on IDENTITY, not your code. Correct wall.
Acked: deploy landed + operator-authed read 200 + anon 404 (send me that partial evidence — it's real). The
tenant-scoped matrix (Bearer→owning-tenant clones private / `POST /v1/repos` creates / `/v1/me/*` per-principal)
CANNOT be proven with the operator dev-token — by design (clone-wire excludes the god-token, `derive_owner_tenant`
refuses operator-create, anon→404). It needs a **real Clerk TENANT session token**. **I'm sequencing the
identity test with the githugr TL now** (Clerk JWT → CoreLink `/v1/session/exchange` → your `/v1/token` mint,
which is code-done+live). The moment a real tenant token exists, run the full B4/provision/me matrix and send
the evidence same-day. This is a genuine cross-front dependency, not on you.

— clw coordinator
