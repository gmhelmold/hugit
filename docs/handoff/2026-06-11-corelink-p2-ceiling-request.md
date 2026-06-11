# Request to the CoreLink TechLead — lift the hugit P2 ceiling (3 infra seams)

> **From:** hugit TechLead · **Date:** 2026-06-11 · **Status:** OPEN, owner-gated
> **Supersedes scope of:** `docs/handoff/2026-06-08-corelink-p2-tenant-request.md`
> (the tenant-provisioning request) — that doc stands; this one **adds the three
> live-infra seams** that two adversarial audit rounds proved are the *only*
> things standing between hugit and a shippable production forge.
>
> **Read this first if you read nothing else (§0).** Everything below is
> evidence-driven: each ask cites the adversarial finding that proves hugit
> cannot close it from the code side. Nothing here waits on hugit — §6 shows
> every hugit-side seam is built, hermetically proven, and `NotWired`-gated,
> waiting only for the values you hand back.

---

## 0. Executive summary — what I need, in one screen

hugit's engine is **code-complete and adversarially hardened**: two fresh-context
7-agent refutation rounds (`docs/review/2026-06-11-adversarial-round-1.md`,
`-round-2.md`) confirmed the integrity spine (hash-chained log, D14 authz,
redaction, money) holds. The rounds also proved that **three** ship-blocking
gaps are **not closable in hugit code** — they are CoreLink infrastructure
seams. They are the entire "P2 ceiling":

| # | Seam | What it unblocks | Adversarial finding | CoreLink primitive needed |
|---|---|---|---|---|
| **A** | **Live Action Cache transport** | the memoized-CI wedge becomes *observable* (today it renders null on every real log) | PS-1 / P-WEDGE-HOLLOW | AC namespace + HTTP endpoint + PAT (the §2 tenant ask, now load-bearing) |
| **B** | **CAS erase / tombstone API (R2)** | right-to-erasure on the forever-store becomes *real* (today proven only against an in-process toy) | PS-3 / SOTA-audit S4 | R2 keyed delete + a durable tombstone record |
| **C** | **Session→token exchange endpoint** | `--author-kind` stops being caller-asserted (today a subagent can claim `orchestrator`) | PS-2 / SOTA-audit S3 | the ADR-0002 §A1 endpoint (Clerk-backed) |

Each is detailed below with an exact contract and a definition-of-done. **A** is
the smallest and highest-value (it lights up the product's headline). **C** is
the one with a cross-repo identity dependency (ADR-0002). **B** is a focused R2
capability.

---

## 1. Context — why these three, why now

The hugit thesis is *memoize by content, price flat; agent work is legible and
accountable*. Two audit rounds confirmed the **legible/accountable** half is
built: envelopes capture, the Ledger rolls up cost, provenance is signed, the
log is tamper-evident. But the rounds also reproduced, on the live binary, that
the **memoize** half — the wedge a customer pays for — is **invisible end-to-end**
because the cache it reads has no live transport (Seam A). Separately they
showed two *correctness-of-claim* gaps: we advertise right-to-erasure and
authenticated authorship, and both are honest-stubbed pending your infra
(Seams B, C).

These are not hugit defects. They are the disclosed live-infra boundary the
whitepaper always named (AC/CAS · runners · identity). This document converts
that boundary into three precise, independently-shippable asks.

---

## 2. SEAM A — Live Action Cache transport (the wedge)

### What I need
The AC namespace + HTTP endpoint + scoped PAT from the §2 tenant request, now
**load-bearing**: hugit's memoized-CI executor must be able to do, over the wire,
the two-call protocol it already speaks hermetically:

```
GET  {AC_URL}/v1/ac/{tenant}/{memo_key}        → 200 {CheckResult json} | 404 miss
PUT  {AC_URL}/v1/ac/{tenant}/{memo_key}         → 200/201 stored | 4xx/5xx explicit error
     body: {CheckResult json}, Authorization: Bearer <PAT>
```
where `memo_key = sha256(tree_hash ‖ def_digest ‖ toolchain_digest)` —
the frozen 3-axis key (`hugit-checks/src/memo_key.rs`, hermetically tested).

### Why this is the wedge (adversarial evidence)
> Round-2 (product lens), reproduced live: `hugit checks show` →
> `{"kpis":{"hit_rate_pct":null,"hits":null,"executed":null,"saved_ms":null}}`
> on a real workflow log, with the honest note *"no porcelain verb appends
> check.recorded yet."*

The recorder verb (`hugit check` → emits `check.recorded`) is **hugit-side and
P2-independent** — I will build it (PS-1). But for the recorded result to mean
*"cache hit, 0 execution, $X saved"* — the thing we sell — the executor must hit
a **live AC**. Until then `hit_rate` is structurally 0% because every check is a
cold local execution. Seam A is what turns the demo green.

### Exact contract (confirm or correct)
- Hit semantics: a `200` with a stored `CheckResult` ⇒ hugit reports `cache_hit`,
  `duration_ms: 0`, and counts the would-be exec cost as `saved`. A `404` ⇒ miss
  ⇒ local execute ⇒ `PUT` the result.
- Trust boundary: your edge resolves `PAT → tenant` and **rejects any
  `path-tenant != PAT-tenant`** (cross-tenant isolation — the X10⑤ non-interference
  bound). hugit treats a `401/403` as a hard fail-closed error, never a miss.
- Idempotent `PUT`: storing the same `memo_key` twice with byte-identical body
  is a no-op success; a *different* body for an existing key is a `409` (we must
  never silently overwrite a proven result).

### Definition of done
1. `corelink_ac_live_smoke` (`tests/corelink_ac_smoke.rs`, already wired,
   currently SKIP-with-reason) runs green when `HUGIT_CORELINK_AC_URL` + tenant +
   PAT are present: a cold check `PUT`s, a warm re-run `GET`s a hit with
   `duration_ms: 0`.
2. `hugit checks show` on a log that ran ≥1 memoized check reports non-null
   `hit_rate_pct` and `saved_ms`.
3. Cross-tenant `GET` with a foreign PAT returns `403`, and the hugit client
   surfaces it as `AcError::Forbidden`, not a miss (fail-closed test).

---

## 3. SEAM B — CAS erase / tombstone API (right-to-erasure)

### What I need
An R2-backed **content-addressed erase** on the tenant CAS:
```
DELETE {CAS_URL}/v1/cas/{tenant}/{content_hash}
   → 200 {tombstone: {erased_hash, erased_at, by}}   (object bytes gone, tombstone durable)
   → 404 if never stored
GET    {CAS_URL}/v1/cas/{tenant}/{content_hash}  after erase
   → 410 Gone {tombstone}     (NEVER 404, NEVER the bytes)
```
The distinction is the whole point: an erased object must read back as a
**distinguishable tombstone (410)**, never as absent (404) and never as bytes.

### Why (adversarial evidence)
> SOTA-audit S4 / Round-1: WA3 shipped tombstone erasure on the `ColdBlobStore`
> trait with `GetOutcome::{Present,Erased,Absent}` and resurrection-refused —
> but the **real** cold store (R2 via CoreLink CAS) has no erase path; X7/X12
> prove the cascade only against an in-process `InMemoryObjectStore`.

ADR-0001 ratified *retention forever, erasure only by explicit tombstone* ("o
conteúdo é apagável; a prova, não"). That law is currently unsatisfiable on the
real data path. Trajectory blobs are **cold-tier, write-once** (the owner's
2026-06-10 decision keeps them OFF the hot AC/CAS path — see the §8 addendum of
the 2026-06-08 doc), so this erase capability is needed on the **cheap cold R2
bucket**, not the latency-sensitive AC.

### Contract notes
- Erase is **by content hash** (dedupe-aware): erasing a hash erases the one
  shared blob for all references — that is the intended semantic, documented.
- The tombstone is itself content-addressed and immutable (it is "the proof that
  survives"); it carries `{erased_hash, requested_by, requested_at}`.
- `PUT` of a previously-erased hash must be **refused** (resurrection-closed) —
  return `409 Gone`.

### Definition of done
1. The production `ColdStore` adapter (`hugit-refstore` cold tier over CoreLink
   CAS) implements `erase` against the real R2 DELETE.
2. X7① + X12① acceptance suites re-target to the **real** adapter under
   `HUGIT_CORELINK_*` (run-not-skip); removing the adapter's erase call turns
   X7① RED (proven load-bearing).
3. A round-trip test: store → get(Present) → erase → get(410 Gone tombstone) →
   put(409 refused).

---

## 4. SEAM C — Session→token exchange endpoint (authenticated authorship)

### What I need
The ADR-0002 **§A1** server-side endpoint: exchange a HuGR identity session
(Clerk-backed) for a tenant-scoped, short-lived capability the forge can verify,
so `author_kind` is **derived from the authenticated principal**, not asserted by
a CLI flag. A PAT never reaches a browser (ADR-0002 invariant).

```
POST {ID_URL}/v1/session/exchange
   Authorization: <HuGR session>     (server-side only; never client-exposed)
   → 200 { principal, tenant, author_kind_claims:[...], exp }
```

### Why (adversarial evidence)
> SOTA-audit S3 / Round-2 (fable), reproduced live:
> `hugit pr open --author-kind orchestrator --run-id run-fake-anyone` succeeds
> with a fabricated run-id and no credential. The D14 guard enforces the matrix
> *given the claimed kind* — but the claim is unauthenticated.

hugit's D14 authz spine is built and wired (Wave A/E: `append_authorized` +
`AuditedGuard`, full caller audit, no bypass). The **only** missing piece is the
binding from a verified principal to `author_kind`. That binding is identity
infrastructure (ADR-0002), and its server-side token-exchange contract is yours
to confirm.

### Contract notes
- This is the §A1 contract already raised in
  `docs/handoff/2026-06-09-hugr-identity-rollout.md` — **explicitly NOT a P2
  blocker for the other two seams**; it is needed by githugr Wave 4 and by
  hugit's authn binding, on your timeline.
- hugit needs only: given a session, return the principal + the set of
  `author_kind` values that principal may assert. The forge does the matrix
  enforcement; you do the authentication.

### Definition of done
1. The endpoint contract (request/response shape, token lifetime, revocation
   propagation per `INV-PAT-REVOKE-PROPAGATION`) is confirmed or corrected.
2. hugit's `--author-kind` becomes an override that is **rejected** when it
   exceeds the session's `author_kind_claims`; a subagent claiming `orchestrator`
   without the claim is denied + audited (the D14 oracle extends to a
   mutation-verified test).

---

## 5. Sequencing & non-interference (please respect)

- **Independence:** A, B, C are independently shippable. Recommended order by
  value × effort: **A first** (lights the wedge), **B next** (one R2 capability),
  **C on the identity timeline** (cross-repo, ADR-0002).
- **Non-interference (non-negotiable):** every hugit AC/CAS call is **policy-capped
  from day 1** (rate + monthly $ ceiling, §2.4 of the tenant request). A hugit-side
  storm must be **structurally bounded before it can ever test CoreLink's fairness
  layer** — hugit work must never touch the launch route or the campaign-#1/#2
  critical paths. Tell me the caps you set and I document them.
- **Tense discipline:** this request describes hugit's needs against CoreLink as
  it is at the time of writing. If any primitive here (R2 keyed delete, the AC
  409-on-divergent-body semantic, the §A1 endpoint) does **not** exist yet on the
  CoreLink side, say so — I will not assume it, and I will mark the seam
  `NotWired` on the hugit side until you confirm.

---

## 6. What is ALREADY done on the hugit side (nothing waits on me)

Every seam below is built, hermetically proven, and gated so it **cannot rot to
a false green** before you deliver — it fails closed or skips loudly:

| Seam | hugit-side status | Proof |
|---|---|---|
| A — AC client | `HttpAcClient` speaks the GET/PUT 2-call protocol; `corelink_ac_from_env()` loads URL/tenant/PAT; PAT redacted in Debug/Display; `NotWired` until configured | hermetic recording-mock transport tests; `corelink_ac_live_smoke` SKIP-gated |
| A — recorder verb | `hugit check` recorder (`check.recorded` producer) is **hugit-side, P2-independent** — I build it next; it works against `InMemoryAc` today and swaps to live AC by config | PS-1 acceptance criteria |
| B — erase trait | `ColdBlobStore::erase` + `GetOutcome::{Present,Erased,Absent}` + resurrection-refused shipped (WA3); X7/X12 prove the cascade on the in-process store | CHANGELOG WA3; `crates/hugit-invariants/x7,x12` |
| C — authz spine | D14 `append_authorized` + `AuditedGuard` wired to every mutation path; full caller audit, no bypass (Wave A/E, E-GUARD/E-GUARD2) | `docs/review/2026-06-11-adversarial-round-2.md` (authz lens: spine confirmed sound) |
| Secrets | PAT lands at `~/.hugit/secrets/corelink/pat` (mode 600, outside every repo); gitleaks-class redaction-on-write so no PAT/secret enters the forever-log | CHANGELOG WA1/WF-REDACT |

When A/B/C land, each goes live with **configuration only — no further hugit code
change** beyond the recorder verb I already own. Ping me per seam and I run its
DoD and report.

---

## 7. Definition of done (how we both confirm the ceiling is lifted)

- **A:** `corelink_ac_live_smoke` green; `hugit checks show` non-null hit-rate on
  a real log; cross-tenant `GET` → 403 fail-closed.
- **B:** X7①/X12① green against the real R2 adapter (run-not-skip); erase→410,
  put-after-erase→409.
- **C:** §A1 endpoint confirmed; subagent-claims-orchestrator denied unless the
  session grants it.

When all three are green, a fresh adversarial round re-audits, and if it
converges on ship-it, the hugit engine is SOTA **as a production forge**, not
just hermetically. Until then it is SOTA **by construction** with three
honestly-disclosed, infra-gated seams — and that distinction is stated plainly
in `CLAUDE.md` and the pending-seams register (`docs/plan/2026-06-11-pending-seams.md`).

---

*Cross-references: tenant provisioning `docs/handoff/2026-06-08-corelink-p2-tenant-request.md`
· identity `docs/handoff/2026-06-09-hugr-identity-rollout.md` + `docs/adr/0002-hugr-identity.md`
· evidence `docs/review/2026-06-11-adversarial-round-{1,2}.md` · seam register
`docs/plan/2026-06-11-pending-seams.md` · the interop seam map `docs/interop.md`.*
