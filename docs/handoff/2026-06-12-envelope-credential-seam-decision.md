# DECISION → corelink-runners: envelope subscriber identity (§13.2) = Option A (same tenant PAT)

**From:** hugit techlead · **Date:** 2026-06-12 ·
**Re:** corelink-runners handoff `2026-06-12-hugit-envelope-credential-seam.md` (PR #24, `CaptureHook` credential seam) ·
**Routing:** via owner for ratification (per the request's ACTION) ·
**Governing:** ADR-0002 (`docs/adr/0002-hugr-identity.md`, owner-ratified 2026-06-09) · `docs/interop.md` (runner seam) · the frozen integration contract §1 (Bearer-PAT)

---

## Decision

**Option A — the envelope subscriber presents the SAME tenant Bearer PAT that acquired the lease.**
No contract change; `AcquireResponse` stays frozen and untouched. The `HookRegistry` credential
registered at `acquire` (the raw tenant Bearer PAT) is exactly the credential the hugit envelope
poller presents on `GET /v1/leases/{id}/envelope/{events,meta}`.

## Answer to P1 / P2

**P1 — same PAT.** hugit does **not** separate the acquire and subscribe roles. The hugit engine
(orchestrator) acquires the runner lease and polls the envelope endpoints as the SAME machine
principal, using the SAME tenant PAT. There is no "PAT-A acquires / PAT-B subscribes" split, so the
silent-503 failure mode that is option A's only con **cannot occur** in hugit's architecture.

**P2 — n/a** (no different credential is needed).

## Why A (architecture-verified, not just asserted)

1. **ADR-0002 §2.2 — machine credentials are PATs, unchanged** (Argon2id, tenant-scoped). One
   tenant PAT per org = tenant; that is what the engine speaks to CoreLink on every machine seam.
2. **ADR-0002 §4 (hugit row) — "no code change"**; the PAT flow is already contract-shaped; "the
   `operator` field in envelopes/attestations references the HuGR account principal." The envelope
   producer carries the principal as *data*, not as a second auth credential.
3. **ADR-0002 §6.4 — the identity surface adds NO parallel auth domain** (one user pool, one
   org→tenant mapping). A per-envelope or per-tenant *envelope* credential (options B/C) would be a
   second machine-auth domain — exactly what the ADR forbids absent a forcing function (§2.4/§5).
   There is **no forcing function** here.
4. **Frozen integration contract §1 + ADR-0002 §4 (corelink-runners row): the lease API stays
   Bearer-PAT, unchanged.** The envelope poll is on the same lease, same wire, same Bearer PAT.
5. **Code check (hugit side):** `crates/hugit-ledger/src/envelope/` is a pure producer/redaction
   module (trajectory capture + write-path redaction + metrics over the `cold_store` seam). It
   contains **no** PAT / Bearer / credential / lease / acquire / subscribe logic — auth is entirely
   a wire-layer (lease-client) concern carrying the tenant PAT. There is no separate authenticated
   "envelope subscriber service" to hold a distinct credential. (Verified at `main` HEAD `5457730`.)

## Consequences / what corelink-runners should do

- Keep the current default: register the `CaptureHook` credential = the acquire tenant Bearer PAT.
  The flagged-in-code assumption at `crates/corelink-fabric-server/src/handlers/leases.rs` is
  **CORRECT for hugit** — promote it from "assumption" to "ratified" with a pointer to this doc.
- **No frozen-type amendment** (option C) and **no new CoreLink credential API** (option B) are
  needed for hugit. If a *different* family consumer later separates roles, that is a new forcing
  function to be decided then — not now, and not on hugit's behalf.
- The fail-closed posture (wrong credential → 503, never a cross-tenant drain) and the independent
  tenant-ownership gate (404, no existence oracle) are correct and should stay.

## Reversibility / scope

Per the request, the registered credential is a one-line change in the acquire composition root, no
wire/conformance-vector impact. Option A means that line stays as-is. This unblocks §13.2 production
wiring for M1. The `IntentMetrics` conformance vector (§13.4) remains owner/hugit-gated and is **not**
touched by this decision.

## ACTION (owner)

Ratify Option A and relay to the corelink-runners techlead (read-only from hugit's side — the
hugit session fence forbids writing into the sibling repo; this doc is the hugit-side record to
relay). No hugit code change is required (ADR-0002 §4 holds).
