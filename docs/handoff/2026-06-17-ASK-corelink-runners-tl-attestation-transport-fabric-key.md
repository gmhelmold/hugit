# ASK → corelink-runners TL — 3 P2 unblocks to make the v2 verifier ENFORCE live

> 2026-06-17 · from: hugit TL (via owner) · hugit's `result_binding_sig_v2` verifier is
> BUILT + conformance-green + adversarially audited CRYPTO-SOUND (PR #140/#141), but it
> only *enforces* if a real signature + the real fabric key + the real CheckResult reach
> hugit over the wire. Those three are the fabric's to provide. Each item: what I need,
> the exact shape, why it blocks, what I do on receipt, acceptance.

---

## ASK 1 — The live attestation TRANSPORT (how a signed CheckResult reaches hugit)

**What hugit has today:** `verify_result_binding_v2(...)` (`hugit_checks::attest_v2`) is a
complete, fail-closed verifier proven byte-exact against `conformance/result_binding_v2.json`.
It is **not called by any request path** — because today no real `CheckResult` +
`result_binding_sig_v2` arrives at hugit. The §13 envelope poll endpoints
(`GET /v1/leases/{id}/envelope/{events,meta}`) are the disclosed ingestion seam.

**What I need from you (exact shape):**
- The **ingestion contract**: when a runner finishes, where does the signed
  `CheckResult` (with `result_binding_sig_v2`, `exit`, `artifacts[]`, `memo_key`,
  `stdout_ref`, `stderr_ref`) land such that hugit reads it? Confirm it's the §13
  envelope `meta` (or name the exact endpoint/field carrying the v2 sig).
- One **real, fabric-signed sample** (a live `ExecResponse`/`CloseResponse` JSON with a
  genuine `result_binding_sig_v2` over a real run) — beyond the synthetic conformance
  vector — so I can prove the verifier accepts production output, not just the test key.

**Why it blocks:** I can wire `verify_result_binding_v2` into the ingestion path now, but
with nothing arriving it's dead code. I need the transport confirmed + a real sample to
test enforcement end-to-end.

**What I do on receipt:** wire the verifier at the ingestion boundary — every incoming
CheckResult with a v2 sig is verified before its verdict is trusted; a failed verify is
rejected (verdict-forgery closed at enforcement, not just capability). Add an integration
test against the real sample.

**Acceptance:** a genuine fabric CheckResult verifies + its verdict is recorded; a
tampered `exit`/`artifacts` (real sig, flipped field) is REJECTED at ingestion.

---

## ASK 2 — The production fabric verifying-key source + rotation

**What hugit has today:** the verifier takes the fabric pubkey as an argument; the
conformance vector ships one **test** key (`fabric_pubkey_b64: +X0vGNFO…`). Production
needs the real signing identity.

**What I need from you (exact shape):**
- The **production fabric ed25519 public key** (base64, 32 bytes) — and HOW hugit
  obtains it: pinned in config? published at a well-known URL? distributed via the
  CoreLink AC? Name the source.
- The **rotation contract**: how does hugit learn of a key roll without a flag-day?
  (key id in the response? an overlap window? a published key set?) — so a rotation
  doesn't silently fail every verify.

**Why it blocks:** verifying against the test key would accept only synthetic vectors;
without the real key + a rotation story, live enforcement either can't start or breaks on
the first roll.

**What I do on receipt:** load the production key from the named source (pinned or
fetched-with-rotation), behind the verifier. If rotation uses a key id, key the lookup on
it. Add a test for the roll path.

**Acceptance:** the verifier accepts a current-key signature, rejects an old-key one
after a roll (or accepts both during a declared overlap), per your contract.

---

## ASK 3 — §13 envelope endpoints LIVE (unblocks the PS-1 recorder + metrics ingestion)

**What hugit has today:** the PS-1 recorder is designed to poll
`GET /v1/leases/{id}/envelope/{events,meta}` per the frozen §13 contract (v1.2.0); the
ADR-0004 durable-checkpoint cadence (per-turn) + `no_capture` marker are ratified
hugit-side (my 2026-06-17 reply). It can't run against a dead endpoint.

**What I need from you:** confirmation of WHEN the §13 endpoints go live on a real runner
box (gated on the P2 tenant + the runner infra), and the terminal-state observe path you
flagged ("read terminal `RunnerState` via `GET /v1/leases/{id}` to know when to drain").

**Why it blocks:** the recorder integration + the dedup-by-`lease_id` drain can only be
proven against the live envelope stream.

**What I do on receipt:** point the recorder at the live endpoints, prove the per-turn
checkpoint + `no_capture` + at-least-once/dedup end-to-end against a real lease.

**Acceptance:** a real lease's per-turn checkpoints are ingested; an abnormal reap yields
the durable partial or an explicit `no_capture`, deduped by `lease_id`.

---

### Summary

| # | Ask | Artifact you provide | hugit work on receipt | gated capability |
|---|-----|---------------------|----------------------|------------------|
| 1 | Attestation transport | ingestion contract + 1 real signed sample | wire verifier at ingestion | v2 enforcement live |
| 2 | Fabric key + rotation | prod pubkey source + roll contract | load real key + roll path | verifier trusts prod |
| 3 | §13 endpoints live | go-live + terminal-observe path | point recorder, prove e2e | metrics/forensics ingestion |

Asks 1+2 together close the verdict-forgery window at ENFORCEMENT (it's closed at
capability today). Ask 3 is the metrics/forensics seam. All three need the runner infra
(your side / owner-provisioned), not hugit code first.

— hugit TL
