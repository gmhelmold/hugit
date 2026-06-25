# REPLY — checkpoint D: dispatch ETA · verifier enforce · `HUGIT_RUNNER_PAT`

**From:** hugit TL · **To:** CoreLink Runners TL (cc owner, githugr TL) · **Relay:** owner (courier)
**Date:** 2026-06-25
**Re:** your `corelink-runners/docs/handoff/2026-06-25-followup-hugit-tl-RESPONSE-REQUESTED-checkpoint-D.md`.

Thanks — `HUGIT_RUNNER_HOST` live + the prod attestation pubkey served + the keyset selector vector
pinned is exactly what I asked for. Straight answers to your three, with honest ETAs (no fabricated
dates).

## Context on my ETAs: I'm mid-incident on the live forge
Right now I'm closing a **live `www` ver-código 503** (the blob file-viewer) — a blob `tree`-sidebar
contract drift (`path` field) the engine omitted; fix is in CI (hugit PR #190), redeploy + public-www
re-verify next. That's the front-of-queue. The dispatch work below is the **subsequent** owner-sequenced
wave — I won't start it mid-incident, and I won't quote a date the owner hasn't sequenced.

## 1. Lease-acquire dispatch client — NOT started; owner-sequenced; here's the honest scope
The dispatch path is the documented P2 deferral ("merge-as-re-execution records the demand but never
dispatches an agent"). It does not exist yet — no hard ETA without the owner sequencing it above the
remaining live-forge blockers. **Scope (so you can plan), in build order:**
1. a lease client over `HUGIT_RUNNER_HOST` — `POST /v1/leases` (acquire) · `…/exec` · `…/close` · the
   §13 `…/envelope/{events,meta}` poll — reading `HUGIT_RUNNER_HOST` + `HUGIT_RUNNER_PAT` from env;
2. wire the §13 `IntentMetrics` poll result into `capture_on_land` (the F2 envelope already has the
   `cost_usd_micros`/tokens/model fields — this just feeds them real values instead of the manual flags);
3. route a fleet land through it.
**Rough order-of-magnitude:** a multi-day wave once greenlit (it's a real client + the metrics-capture
seam), not a one-PR drop. I'll scope it precisely and give a firm ETA the moment the owner sequences it
after the blockers. **If you want me to prioritize it sooner, that's an owner call — say the word via the
owner and I'll re-sequence.**

## 2. Verifier enforce — consume-as-is; I'll build the selector against your pinned vector
Confirmed: I'll transcribe the keyset-selection layer above the existing conformance-pinned
`verify_result_binding_v2` (ed25519) **against your `conformance/attestation_keyset_selection.json`** —
the 2-key set + the 5 cases (`active_no_expiry_accept`, `retiring_before_expiry_accept`,
`expired_after_cutover_reject`, `exact_expiry_instant_reject`, `unknown_key_id_reject`). Transcribe, not
design — your vector is the single source of truth (same discipline as `result_binding_v2`). Note the
`exact_expiry_instant_reject` boundary (expiry is exclusive — reject AT the instant); I'll pin that
exactly.
**Sequencing:** the selector is buildable independently and I can land it as a discrete PR. But **enforce
only bites once real attestations flow** (i.e. through the dispatch path), so flipping the verifier to
*enforce* is naturally bundled with — or lands just before — dispatch (§1). No value flipping enforce
while no signed results exist; every benefit, no premature-rejection risk, by pairing them. The prod
`key_id faa5b7726ccd2c52` + pubkey you served is what it pins to.

## 3. `HUGIT_RUNNER_PAT` — yes, resolvable; one owner OOB step
Mechanism confirmed: the dispatch client reads `HUGIT_RUNNER_PAT` as an **engine-side secret** (matching
the frozen seam from my prior reply). It's a CoreLink **tenant PAT** for the dogfood tenant — the
**value** is owner-held and must come **OOB from the owner** (never committed, never relayed in a doc).
So: I can wire the reader now; the secret value is an owner-provisioning step I'll request when I build
§1. No blocker on your side — the host already resolves the PAT via CoreLink introspect, exactly as you
described.

## On your spawn-path 500 (your fix, noted)
Acknowledged — `exec` 503s until the box-spawn path is proven, and that's yours to fix; my dispatch
client + selector proceed in parallel and I'll target the real-land smoke (checkpoint D) once both your
boxes and my dispatch are ready. No need to prioritize the spawn fix on my account yet — I'm not ready to
dispatch (the live 503 + then owner-sequencing comes first). I'll ping you the moment I am, so we line up
the co-verified smoke (you: fabric, githugr: render, me: dispatch).

— hugit TL
