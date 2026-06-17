# INDEX — hugit's P2 unblocks: who owns what (owner relay map)

> 2026-06-17 · from: hugit TL · The complete map of what hugit needs to flip every
> remaining P2 seam to live. hugit's engine code is complete + audited; each item below
> is blocked on an ARTIFACT or DECISION owned by someone else. This index says who, and
> points to the chewed per-TL ask. Relay each ask to its TL; hugit moves the same day it
> lands.

## The two per-TL asks (full detail inside)

- **CoreLink Server TL** → `docs/handoff/2026-06-17-ASK-corelink-server-tl-clerk-tenant-store.md`
  1. Live Clerk JWKS URL + issuer + `azp` → real identity / `POST /v1/token`.
  2. P2 tenant id + org→tenant mapping → real multi-tenant isolation.
  3. Shared-store substrate decision (DO/D1/KV) OR "single-instance is P2" → horizontal scale.

- **corelink-runners TL** → `docs/handoff/2026-06-17-ASK-corelink-runners-tl-attestation-transport-fabric-key.md`
  1. Attestation ingestion transport + one real signed sample → v2 verifier ENFORCES live.
  2. Production fabric pubkey source + rotation contract → verifier trusts prod, survives rolls.
  3. §13 envelope endpoints live + terminal-observe path → metrics/forensics ingestion.

## Owner / infra (not a TL — provisioning)

| Item | What it unblocks | Note |
|------|------------------|------|
| Clerk instance provisioned | Server TL Ask 1 can be answered | the JWKS URL comes from a live Clerk |
| CoreLink P2 tenant + ceiling | Server TL Ask 2 + runners §13 | `docs/handoff/2026-06-08-…-p2-tenant-request.md`, `…-2026-06-11-…-ceiling-request.md` |
| Dedicated CI runner box | de-flakes CI (PS-12b); ends "local build starves CI" | today CI runs on the dev box (single self-hosted) |
| R2 write credential | ✅ DONE (live writes proven) | standing RW cred verified-writable |

## githugr TL — NOTHING blocking from hugit's side

Stated explicitly so no phantom ask is invented: hugit is the engine githugr **consumes**;
hugit depends on githugr for nothing. The only open thread is the **correction** already
sent (`docs/handoff/2026-06-17-correction-githugr-tl-verifier-not-live-cas-yes.md`): CAS +
#138/#139 are live on their rebuilt engine; the v2 verifier is built but NOT live
(enforcement = the runners-TL asks above). No artifact needed from them.

## The honest bottom line

There is **no in-control hugit engineering left** that isn't (a) blocked on one of the
artifacts above, or (b) gold-plating (which the rigor mandate forbids — unrequested scope
is debt, not progress). Everything buildable behind each seam is already built + tested +
fail-closed. Each ask above turns "blocked" into "one artifact away."

Priority order if you want a single thread to pull first:
1. **Clerk JWKS** (Server TL Ask 1) — the keystone; real identity unlocks real tenancy.
2. **Attestation transport + fabric key** (runners TL Asks 1+2) — closes the P0
   verdict-forgery window at enforcement.
3. The rest (tenant, §13, shared-store, runner box) follow naturally.

— hugit TL
