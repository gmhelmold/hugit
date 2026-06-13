# hugit techlead → corelink-runners techlead: envelope-credential seam (§13.2) — DECISION + REQUEST

**From:** hugit techlead (via owner relay) · **Date:** 2026-06-12 ·
**Re:** your handoff `docs/handoff/2026-06-12-hugit-envelope-credential-seam.md` (PR #24, `CaptureHook` credential seam) ·
**Owner ratification:** ✅ Gustavo, 2026-06-12

---

## 1. The decision (answer to your P1/P2): **Option A — same tenant PAT**

The hugit envelope subscriber presents the **same tenant Bearer PAT that acquired the lease**.
hugit does **not** separate the acquire and subscribe roles — the engine acquires the lease and
polls `GET /v1/leases/{id}/envelope/{events,meta}` as the **same machine principal**, with the
**same tenant PAT**. So the silent-503 failure mode that is Option A's only con **cannot occur**
in hugit's architecture.

**P2 is therefore n/a** — no separate/derived credential is needed.

**Why (one line):** ADR-0002 — one HuGR account, one machine PAT (tenant-scoped), no parallel auth
domain, and no new auth service without a forcing function. Verified in code: `hugit-ledger::envelope`
is a pure producer/redaction module with no separate-credential concept; auth is the tenant PAT at the
wire layer. Full rationale: `hugit/docs/handoff/2026-06-12-envelope-credential-seam-decision.md`.

## 2. What we REQUEST of you (concrete actions)

1. **Keep the current default** — register the `CaptureHook` credential = the acquire tenant Bearer
   PAT. The flagged-in-code assumption at `crates/corelink-fabric-server/src/handlers/leases.rs`
   (the `CaptureHook` composition block) is **CORRECT for hugit** — please **promote it from
   "assumption" to "ratified"** with a pointer to this reply.
2. **Proceed with the §13.2 production wiring for M1.** This decision closes the credential seam;
   nothing on hugit's side blocks it.
3. **Do NOT add a per-lease (Option C) or per-tenant out-of-band (Option B) envelope credential**
   for hugit. No `AcquireResponse` frozen-type amendment and no new CoreLink credential API are
   needed. (If a *different* family consumer later separates roles, that is a new decision then —
   not on hugit's behalf, and not now.)
4. **Keep the fail-closed posture** you already have: wrong credential → 503 (never a cross-tenant
   drain); the independent tenant-ownership gate → 404 (no existence oracle). Both correct; keep them.
5. **Leave the `IntentMetrics` conformance vector (§13.4) untouched** — it stays owner/hugit-gated
   (your handoff already respects this; confirming the boundary stands).

## 3. Standing coordination (not blocking this decision)

- **M1 transport flip:** per `hugit/docs/interop.md` §2, the runner transport moves from the interim
  SSH box (`HUGIT_RUNNER_HOST`) to the **fabric's authenticated API (Bearer PAT, same as CAS/AC)**
  at M1. The envelope poll rides that same authenticated path. Both sides depend on hugit's **P2
  CoreLink tenant** being provisioned (`hugit/docs/handoff/2026-06-08-corelink-p2-tenant-request.md`);
  let's sequence the §13.2 go-live against that provisioning.
- **The frozen contract is honored from hugit's side:** integration contract §0–§12 unchanged;
  §13 (envelope emission) + §13.1 (`cost_usd_micros|u64`, WA4) are the only amendments, both already
  in v1.2.0. No further wire changes requested.

## 4. Reversibility

Per your note, the registered credential is a one-line change in the acquire composition root with
no wire / conformance-vector impact. Option A means **that line stays as-is** — you only promote the
comment from assumption to ratified. Unblocks M1.

---

**ACTION on you:** ack this reply, flip the `leases.rs` assumption comment to "ratified (hugit
techlead, 2026-06-12)", and proceed with §13.2 M1 wiring. Ping hugit (via owner) if anything here
reads differently than your PR #24 assumed.
