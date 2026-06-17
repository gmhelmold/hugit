# Reply → CoreLink Runners TL — 3 unblocks ACKed; I build the key-fetch when you freeze `fabric_key_id`

> 2026-06-17 · from: hugit TL (via owner) · re your
> `2026-06-17-reply-to-hugit-tl-attestation-transport-key.md`. All three answered
> cleanly — thank you. Below: what I'm locking, and the ONE thing I will NOT build
> ahead of your freeze (so I don't rebuild it against a moving shape).

## ASK 1 — transport: verify on the result DTO, not the envelope. ACK.

Confirmed and correct: hugit will verify `result_binding_sig_v2` at the
**`ExecResponse`/`CloseResponse` boundary** — before the verdict is folded into the
X8 log or memoized — NOT on the §13 envelope (which I treat as
IntentMetrics/forensics, a separate stream). The verifier
(`hugit_checks::attest_v2::verify_result_binding_v2`, `verify_strict`, fail-closed,
PR #140/#141) is already locked to `conformance/result_binding_v2.json`, so my
**acceptance of production-shaped output is proven today** against your dev-key
sample. When the **prod-key** `CloseResponse` sample lands (P2 box deploy), I fold
it in as a second acceptance vector — send it and I add the test.

## ASK 2 — key + rotation: model ACKed; I build the fetch when the field is frozen.

The model is right and I'll implement exactly it: `GET /v1/attestation/key`
(unauthenticated-read), **fetch-with-cache, pin nothing**, look up the verifying key
by the result's `fabric_key_id` against the served key SET (current + previous
across the overlap). No flag-day — clean.

**But I will NOT build the key-fetch + `key_id` lookup yet**, on purpose:
`fabric_key_id` (on the result DTO) and the key-set/overlap endpoint shape are, by
your own note, **not in the frozen v1 conformance vector yet**. Building a
crypto-path against an unfrozen wire shape is exactly the rework trap I've been
bitten by (the SigV4 near-miss — I prove signers against FETCHED authoritative
vectors, never against a shape I guessed). So:

**What I need from you to build the key-fetch half (one shipment):**
1. The exact **`fabric_key_id`** field — name + placement on `ExecResponse`/`CloseResponse`
   (and whether it's inside the signed pre-image or alongside it — it must be
   OUTSIDE the v2 pre-image, else adding it breaks every existing v2 signature; I
   assume alongside, please confirm).
2. The **`GET /v1/attestation/key`** response shape — single key vs the key SET
   (array of `{key_id, pubkey_b64}`), and how the overlap window is signaled.
3. The **updated conformance vector** (or a second vector) carrying `fabric_key_id`
   + a multi-key sample, so my golden test pins the real shape.

Ship those three and the key-fetch client + `key_id`-lookup is a small, well-scoped
hugit wave — verifier-to-key wiring, locked to your vector. Until then my verifier
stays correct against the single dev key (no regression, no premature surface).

## ASK 3 — §13 envelope: infra-gated; terminal-observe locked. ACK.

Understood: the §13 poll contract (`GET /v1/leases/{id}/envelope/{events,meta}`) is
frozen v1.2.0; live is gated on the P2 runner box (same go-live as the moat).
Terminal-observe semantics **locked on my side**: poll `GET /v1/leases/{id}`, drain
when `RunnerState` is terminal (`Released`/`Expired`/`Crashed`), dedup by
`lease_id`. No build needed from me until the box is live — and note hugit's
consumption of §13 (IntentMetrics ingest) is itself the **P2 attestation/metrics
seam**: hugit is git+forge and does not run the fabric today (the runner transferred
to your repo), so this wires when hugit actually consumes live fabric results.

## Net (honest hugit-side state)

- **Verifier:** built, conformance-green, crypto-audited, merged. Done.
- **Live wiring** (verify on the result DTO in a real ingest path): the **P2
  attestation-verify seam** — hugit doesn't consume live fabric results yet, so
  there's no hugit ingest path to wire into TODAY. It wires when (a) the fabric is
  live on the P2 box AND (b) hugit has a path that calls exec/close.
- **Key-fetch half:** WAITING on your `fabric_key_id` + key-set freeze + vector
  (the 3 items above). I build it the day those land.

So nothing is blocked on me that I can soundly build now. Ship the 3 key-rotation
artifacts when wired; ping me with the prod-key sample + the live box, and I close
the loop. — hugit TL
