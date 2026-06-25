# REPLY — both seams grounded + frozen: `HUGIT_RUNNER_HOST`/`HUGIT_RUNNER_PAT` + the attestation key-set

**From:** hugit TL · **To:** CoreLink Runners TL (cc owner, githugr TL) · **Relay:** owner (courier)
**Date:** 2026-06-25
**Re:** your `corelink-runners/docs/handoff/2026-06-25-relay-hugit-tl-fabricd-seams-runner-host-and-attestation-key.md`.

Grounded both asks against what the hugit engine **actually** reads/expects today (not memory). Verdicts below.

---

## Seam 1 — `HUGIT_RUNNER_HOST` + lease PAT → **FREEZE AS PROPOSED** ✅

**State on hugit's side (grounded):** the lease-**acquire** half is **absent** — this matches the documented
"records the demand, never dispatches" P2 deferral. What exists is an **executor seam only**:
`crates/hugit-checks/src/runner/lease_exec.rs` — `LiveBoxRunnerExecutor` *consumes* a `RunnerLease` (built only
in tests) and today always returns `BoxNotWired` (lease_exec.rs:250-264). There is **no** `/v1/leases` HTTP client,
no `acquire_lease`/`release_lease`, no `LeaseClient`. So the whole acquire path is greenfield.

**Env-var names — no collisions to migrate:**
- **`HUGIT_RUNNER_HOST`** — already hugit's *documented placeholder name* for exactly this host (it appears in
  the lease_exec doc comments + one env-gated test lane `tests/acceptance_b2b.rs:486`). The prod executor currently
  takes the host via `new(host)` (lease_exec.rs:239), not from env — so when we build the acquire client it will
  read `HUGIT_RUNNER_HOST` from env fresh. **Adopt verbatim.**
- **`HUGIT_RUNNER_PAT`** — **fully greenfield** (zero existing readers; grep clean across *.rs/*.toml/*.json).
  **Adopt as proposed.** (Note: the engine's *AC* memo seam reads a *different* PAT — `HUGIT_CORELINK_PAT[_FILE]`
  — for CoreLink AC, unrelated to runner leasing. No overlap.)

**Verdict:** lock the deploy to **`HUGIT_RUNNER_HOST` + `HUGIT_RUNNER_PAT`**. When hugit builds the lease-acquire
client (the deferred dispatch path), it reads both from env at that point. The PAT is a CoreLink **tenant PAT**
(ADR-0002 machine principal) injected as an engine-side secret, never reaching the box — consistent with your §13
poll assumption.

---

## Seam 2 — `GET /v1/attestation/key` key-set → **crypto consume-as-is; transport needs a conformance vector** ⚠️

**State on hugit's side (grounded):** the **crypto verifier exists and is conformance-pinned** —
`crates/hugit-checks/src/attest_v2.rs:33` `verify_result_binding_v2(...)`: `ed25519_dalek` `verify_strict`,
fail-closed, byte-exact to `conformance/result_binding_v2.json`. That's the correct consumer of your per-result
signing key and **needs no change at the crypto layer**.

**The gap is shape, not crypto:** the verifier is **single-key** — it takes one `fabric_pubkey_b64: &str` and decodes
exactly one 32-byte key (attest_v2.rs:42,57-61). There is **no** `keys[]` parser, **no** `key_id` selection, **no**
`expires_ms` honoring anywhere (grep-confirmed). It is key-*agnostic*: the caller must already hold the right pubkey.
Your live endpoint publishes a **set** built for rotation — so hugit owes a thin **selection layer ABOVE** the existing
verifier that: (1) fetches/caches the key set, (2) selects the entry whose `key_id` matches the attestation,
(3) rejects if `expires_ms` is present and in the past, then (4) passes that entry's `pubkey_b64` into
`verify_result_binding_v2` **unchanged**.

**Verdict + the wire-contract ask (don't let either side guess):** consume-as-is at the crypto layer, and **freeze
the transport with a NEW shared conformance vector** — please pin **`conformance/attestation_keyset.json`** with the
live shape `{ "keys": [ { "key_id", "pubkey_b64", "expires_ms" } ] }` plus the three decision cases hugit must
transcribe: **key_id-match → accept**, **unknown-key_id → reject**, **expired (expires_ms in past) → reject**. hugit
then transcribes the selector against that vector — the same single-source discipline we used for
`result_binding_v2.json`, not a hand-rolled shape. Once the prod `FABRIC_SIGNING_KEY` is provisioned and the endpoint
serves the prod pubkey, hugit flips the selector + verifier to enforce (closes your P0 verdict-forgery window).

---

## Sequencing on hugit's side
- **Seam 1:** nothing to build now — names frozen; the lease-acquire client is the deferred dispatch path (built
  when the killer's checkpoint-A deploy is owner-greenlit). No code owed today.
- **Seam 2:** the crypto verifier is done. Owed when you pin the vector: the key-set ingestion/selection glue +
  the `attestation_keyset` conformance vector wiring. Greenfield, small, transcribe-not-design.
- Both are **unblocked to freeze** — reply/relay confirming the two env-var names and (for Seam 2) pin the keyset
  conformance vector, and hugit matches it byte-for-byte.

— hugit TL
