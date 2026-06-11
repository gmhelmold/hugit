# Request to the CoreLink TechLead — lift the hugit P2 ceiling (6 infra seams)

> **From:** hugit TechLead · **Date:** 2026-06-11 · **Status:** OPEN, owner-gated
> **Supersedes scope of:** `docs/handoff/2026-06-08-corelink-p2-tenant-request.md`
> (the tenant-provisioning request) — that doc stands; this one **adds the six
> live-infra seams** that two adversarial audit rounds proved are the *only*
> things standing between hugit and a shippable production forge.
>
> **Read this first if you read nothing else (§0).** Everything below is
> evidence-driven: each ask cites the adversarial finding that proves hugit
> cannot close it from the code side. Nothing here waits on hugit — §9 shows
> every hugit-side seam is built, hermetically proven, and `NotWired`-gated,
> waiting only for the values you hand back.

---

## 0. Executive summary — what I need, in one screen

hugit's engine is **code-complete and adversarially hardened**: two fresh-context
7-agent refutation rounds (`docs/review/2026-06-11-adversarial-round-1.md`,
`-round-2.md`) confirmed the integrity spine (hash-chained log, D14 authz,
redaction, money) holds. The rounds + a full live-seam inventory
(`docs/interop.md`, whitepaper §5 truth-table, every `NotWired` code site)
establish that hugit's path to a **production** forge is gated entirely on
CoreLink infrastructure. This document enumerates **literally all of it** — the
3 ship-blockers the audit reproduced (A/B/C) **and** the 3 build-out seams the
inventory surfaced (D/E/F) — each with an exact contract and DoD. **§8 is a
coverage matrix** proving every known CoreLink-gated seam is either asked here,
already asked in the tenant/identity docs, or explicitly owned elsewhere with a
reason. Nothing is left implicit.

| # | Seam | What it unblocks | Evidence / source | CoreLink primitive |
|---|---|---|---|---|
| **A** | **Live Action Cache transport** | the memoized-CI wedge becomes *observable* (renders null on every real log today) | PS-1 / P-WEDGE-HOLLOW (reproduced live) | AC namespace + HTTPS GET/PUT + PAT |
| **B** | **CAS erase + D1 event-payload purge** | right-to-erasure on the forever-store becomes *real* (proven only against an in-process toy today) | PS-3 / SOTA-audit S4 | R2 keyed DELETE + tombstone **+ D1 payload purge** |
| **C** | **Session→token exchange endpoint** | `--author-kind` stops being caller-asserted (a subagent can claim `orchestrator` today) | PS-2 / SOTA-audit S3 | ADR-0002 §A1 endpoint (Clerk-backed) |
| **D** | **Per-repo Durable Object event-log binding** | the hash-chained event log gets a live home so githugr can serve it (it is in-process/fixture today) | `docs/interop.md` §5, whitepaper §5 L4 ("to build") | per-repo DO hosting the append-only log |
| **E** | **Transparency / attestation log** | self-release attestation (X8) gets a public inclusion proof; boot can verify it | `crates/hugit-invariants/x8/tlog.rs` `P2_LIVE_SEAM`, interop §4 | a Rekor-class append-only public log endpoint |
| **F** | **Live secrets broker (C5b)** | the flat-file PAT is replaced by a lease-based broker (the credential never sits on the box image/argv) | `hugit-fence/src/broker` (in-process today), interop §6 | a tenant secrets-broker lease API |

Priority by value × effort: **A** first (lights the headline), **B** next (one
R2 + one D1 capability), **D** for the githugr serving path, **E**/**F** are
hardening that ride the same tenant, **C** on the identity timeline (ADR-0002).
**A/B/C are the audit-proven ship-blockers; D/E/F are build-out completeness.**

---

## 1. Context — why these six, why now

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
that boundary into six precise, independently-shippable asks (A–F).

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

### The D1 half (do not omit — the erasure cascade has two stores)
Erasure is a **two-store cascade**, not one DELETE. A trajectory/object blob
lives in R2 (the bytes), but personal data can also sit **inside event-log
payloads** that are projected from the per-repo event store (D1-backed at P2,
Seam D). Right-to-erasure must purge **both**:
```
1. R2:  DELETE the content-addressed blob  → 410-Gone tombstone (above)
2. D1:  purge/redact the event-payload field(s) carrying the same personal data,
        WITHOUT breaking the hash chain — i.e. replace the payload bytes with a
        signed tombstone marker and re-anchor, preserving "the proof it existed"
        while removing "the content."
```
hugit needs the D1-side capability (a targeted payload-purge that keeps
`verify_chain` valid via a tombstone re-anchor) confirmed or designed with you —
this is the half PS-3 names that R2 DELETE alone does not cover.

### Definition of done
1. The production `ColdStore` adapter (`hugit-refstore` cold tier over CoreLink
   CAS) implements `erase` against the real R2 DELETE.
2. The D1 event-payload purge path exists and keeps `verify_chain` green
   post-purge (tombstone re-anchor, not a chain break).
3. X7① + X12① acceptance suites re-target to the **real** adapter under
   `HUGIT_CORELINK_*` (run-not-skip); removing the adapter's erase call turns
   X7① RED (proven load-bearing).
4. A round-trip test: store → get(Present) → erase → get(410 Gone tombstone) →
   put(409 refused); plus an event-payload purge that survives `verify_chain`.

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

## 5. SEAM D — Per-repo Durable Object event-log binding

### What I need
A live, per-repo **Durable Object** that hosts hugit's append-only, hash-chained
event log, with a thin API hugit's refstore binds to:
```
append(record)         → seq, this_hash     (single-writer, serialized — the DO IS the writer lock)
read(range)            → [EventRecord…]      (for projection / verify_chain)
head()                 → seq, this_hash      (CAS-style optimistic concurrency)
```
Single-writer is the load-bearing property: hugit's chain integrity assumes one
serialized appender. A DO is the natural fit (it already serializes); the
flat-file `.lock`/atomic-write seam hugit ships (WC1) is the local stand-in.

### Why
Today the event log is in-process / fixture (`hugit-refstore`); githugr serves
it through a `Provider` over that local log. `docs/interop.md` §5 and the
whitepaper §5 truth-table (L4: "hugit DO event-log — to build") name the live
per-repo DO as the P2 binding. Without it there is no multi-client authoritative
log — the forge is single-host. This is the seam that makes githugr a *hosted*
forge rather than a local viewer.

### Definition of done
1. The per-repo DO API (append/read/head, single-writer guarantee, optimistic
   `head()` concurrency token) is confirmed or designed with you.
2. hugit's refstore binds to it behind the same trait the local log implements;
   `verify_chain` holds across a DO-hosted log; concurrent appenders serialize
   (no lost record — the WC1 concurrency proof re-run against the live DO).

---

## 6. SEAM E — Transparency / attestation log

### What I need
A **Rekor-class append-only public transparency log** endpoint where hugit
publishes its self-release attestations and, at boot/verify, checks inclusion:
```
publish(attestation)   → log_index, inclusion_proof
verify(log_index, att) → inclusion_proof | not-found
```

### Why
`crates/hugit-invariants/x8/tlog.rs` defines a `TransparencyLog` trait gated on
`P2_LIVE_SEAM`; `docs/interop.md` §4 names the live tlog as the binding that
turns hugit's ed25519 self-release attestation (already produced + signed) from
*self-asserted* into *publicly verifiable*. The Security screen's "transparency
log" + SLSA posture (githugr design) reads from this. Without it, attestation is
honest but unwitnessed.

### Definition of done
1. The tlog endpoint contract (publish/verify, inclusion-proof shape) is
   confirmed or corrected.
2. X8's `TransparencyLog` live impl publishes a real attestation and boot-verify
   confirms inclusion (run-not-skip under the live env).

---

## 7. SEAM F — Live secrets broker (C5b)

### What I need
A tenant **secrets-broker lease API** that replaces the interim flat-file PAT:
hugit requests a short-lived lease for a named secret at the moment of use; the
credential value is never written to the runner box image, argv, env dump, or
log.
```
lease(secret_name, ttl)  → lease_handle (value resolved out-of-band, never logged)
revoke(lease_handle)     → ok
```

### Why
`hugit-fence/src/broker` ships the broker logic with an `InMemoryStore` and a
red-team harness proving the secret never persists; `docs/interop.md` §6 and the
2008 tenant request §5 note the live broker "later takes over from flat-file
storage." The flat-file PAT (mode 600) is the disclosed interim; the broker is
the production posture (lease, audit, revoke) — the fence's write-only-secret
model needs a live backend to enforce against.

### Definition of done
1. The broker lease/revoke API contract is confirmed or designed with you.
2. hugit's fence broker binds to it; the C5b red-team harness (mid-op fault,
   secret-never-on-box) runs against the live broker (run-not-skip); flat-file
   PAT becomes a fallback, not the default.

---

## 8. Sequencing & non-interference (please respect)

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

## 9. What is ALREADY done on the hugit side (nothing waits on me)

Every seam below is built, hermetically proven, and gated so it **cannot rot to
a false green** before you deliver — it fails closed or skips loudly:

| Seam | hugit-side status | Proof |
|---|---|---|
| A — AC client | `HttpAcClient` speaks the GET/PUT 2-call protocol; `corelink_ac_from_env()` loads URL/tenant/PAT; PAT redacted in Debug/Display; `NotWired` until configured | hermetic recording-mock transport tests; `corelink_ac_live_smoke` SKIP-gated |
| A — recorder verb | `hugit check` recorder (`check.recorded` producer) is **hugit-side, P2-independent** — I build it next; it works against `InMemoryAc` today and swaps to live AC by config | PS-1 acceptance criteria |
| B — erase trait | `ColdBlobStore::erase` + `GetOutcome::{Present,Erased,Absent}` + resurrection-refused shipped (WA3); X7/X12 prove the cascade on the in-process store | CHANGELOG WA3; `crates/hugit-invariants/x7,x12` |
| C — authz spine | D14 `append_authorized` + `AuditedGuard` wired to every mutation path; full caller audit, no bypass (Wave A/E, E-GUARD/E-GUARD2) | `docs/review/2026-06-11-adversarial-round-2.md` (authz lens: spine confirmed sound) |
| D — event-log | refstore log is single-writer hash-chained with WC1 atomic-lock; binds to a DO behind the same trait | `verify_chain`; WC1 concurrency proof |
| E — tlog | ed25519 self-release attestation produced + signed; `TransparencyLog` trait awaits a live endpoint | `x8/tlog.rs` `P2_LIVE_SEAM` |
| F — broker | fence broker logic + red-team harness ship with an in-process store; binds to a live lease API | `hugit-fence/src/broker`; C5b suite |
| Secrets | PAT lands at `~/.hugit/secrets/corelink/pat` (mode 600, outside every repo); gitleaks-class redaction-on-write so no PAT/secret enters the forever-log | CHANGELOG WA1/WF-REDACT |

When A/B/C land, each goes live with **configuration only — no further hugit code
change** beyond the recorder verb I already own. Ping me per seam and I run its
DoD and report.

---

## 10. Coverage matrix — every CoreLink-gated seam, accounted for

Built from the full live-seam inventory (`docs/interop.md`, whitepaper §5, every
`NotWired`/env-gated site). **No seam is left implicit.** Each is asked here,
asked in the tenant/identity docs, or explicitly out of CoreLink scope with a
reason.

| Seam (inventory) | Disposition |
|---|---|
| Live Action Cache transport | **Ask A** (this doc) + tenant §2.2 |
| CAS namespace + R2 bucket | tenant request §2.3 |
| CAS erase / tombstone (R2 DELETE) | **Ask B** (this doc) |
| D1 event-payload purge (erasure half 2) | **Ask B** (this doc, the D1 half) |
| Session→token exchange (ADR-0002 §A1) | **Ask C** (this doc) + identity rollout §A1 |
| Per-repo Durable Object event-log | **Ask D** (this doc) |
| Transparency / attestation log (X8) | **Ask E** (this doc) |
| Live secrets broker (C5b) | **Ask F** (this doc) |
| Scoped PAT delivery | tenant request §2.5/§5 |
| Policy caps (rate + $ budget) | tenant request §2.4 (+ §5 here) |
| Tenant base URL + slug | tenant request §5 |
| Live X6/X10 non-interference probe URL | **rides Ask A** — same tenant/PAT; the probe endpoint is the AC host (`HUGIT_CORELINK_PROBE_URL` = the AC URL). DoD: X6/X10 run-not-skip once A is live. Flagged here, no separate primitive. |
| Live B8 dogfood soak driver | **rides the tenant** — `HUGIT_DOGFOOD_LIVE` drives the provisioned tenant; no new CoreLink primitive, wiring is hugit-side. |
| **Runner fabric authenticated exec API (M1)** | **OUT of this request — owned by `corelink-runners`.** The runner core was transferred there (campaign #1, WP-R4); hugit consumes it via the frozen wire contract (`hugit-integration-contract.md` v1.2). The interim SSH-to-`hugit-runner-01` box works today; the fabric Bearer-PAT exec API is corelink-runners' deliverable, negotiated through that contract, not a hugit infra ask. |
| **Live GitHub detect (bidir-sync)** | **OUT — GitHub-side, not CoreLink.** `hugit-mirror::sync::detect` `NotWired`; needs a real repo + webhook/poll. A GitHub infra seam (interop §3), not a CoreLink ask. Flagged for completeness. |
| **GitHub App install token (private import/mirror)** | **OUT — GitHub-side.** App ID + private key for private-repo access (`hugit-mirror::import::auth`, fail-not-skip). GitHub infra, tracked in day-0 pointers, not a CoreLink ask. |
| **Workspaces (clw) runtime tie-in (M4)** | **OUT — roadmap, not P2.** Sandboxes as Workspace SKUs is convergence milestone M4 (interop §7); no runtime call exists yet. Named so the enumeration is exhaustive, not requested. |
| Cold trajectory-blob storage (the bytes) | **OUT of the CoreLink hot tenant by owner decision** (2026-06-10, "muito caro" — 2008 §8 addendum): trajectory blobs go to a commodity cold store; only their *erase* path (Ask B) touches CoreLink R2. |

If any seam exists that is not in this matrix, it is an omission — tell me and I
add it. This matrix is the contract of completeness.

---

## 11. Definition of done (how we both confirm the ceiling is lifted)

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
