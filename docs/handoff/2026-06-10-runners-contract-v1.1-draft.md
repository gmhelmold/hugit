# What hugit needs from CoreLink Runners — integration contract v1.1 (draft)

> **Version note — WP-R6 draft (2026-06-10):** this file is the WP-R6
> deliverable: a draft amendment that the hugit lead will review, then apply
> to `corelink-runners/docs/spec/hugit-integration-contract.md` once WP-R4
> frees that tree. The v1.0 body below is REPRODUCED VERBATIM and is
> READ-ONLY in this file; the amendment is ADDITIVE — a new section (§13)
> plus the amendment log appended here. Nothing in §0–§12 is edited.
>
> **APPLIED** — 2026-06-10. Amendment applied to
> `corelink-runners/docs/spec/hugit-integration-contract.md` @ 9796aa8
> (branch `integ/seed-runner`, WP-R6). The live contract in corelink-runners
> is the source of truth; this file is the historical draft + rationale
> record. **CLOSED** — no further action needed from hugit's side.
>
> **How to apply:** replace the v1.0 header (first paragraph and the version
> implied by the date) with the header below, append §13 and the amendment
> log, commit on branch `integ/seed-runner` with the normal fixup discipline
> (no amend, status/log sanity first).

---

> Authored by the **hugit techlead** (2026-06-09). This is the seam **frozen from
> hugit's side**: the fabric must satisfy it for hugit's live CI to light up. It is
> deliberately **not** a work-package decomposition — it is the full-context design
> of what hugit consumes and *why*, so the CoreLink techlead can build the fabric
> side (`corelink-fabric-stub.md`) with every constraint visible.
>
> hugit is already built (campaign #3). The CLIENT of this contract is the
> `hugit-runner` crate in `../hugit/crates/hugit-runner` (lease, fences, cache-warm
> orchestration, byte-identity expectations) plus `hugit-checks` (the memo key + AC
> client). Today it runs against an **interim** runner box (`hugit-runner-01`,
> Hetzner, over SSH); the target is this fabric's API. Where the interim transport
> matters it is called out.

---

## 0. How hugit's CI works (the context that makes the requirements make sense)

hugit's CI is **checks-as-code, memoized in CoreLink's Action Cache, executed on
runners**. The loop, per check:

1. hugit computes a **memo key** = `H(tree_hash ‖ check_def_digest ‖ toolchain_digest)`
   (three-axis; the exact construction is `hugit-checks::compute_memo_key`).
2. hugit asks the **Action Cache**: is this key present?
   - **Hit** ⇒ return the stored `CheckResult` bytes. **Zero execution.** This is the
     wedge — most checks in a fleet are cache hits.
   - **Miss** ⇒ hugit needs a **runner** to execute the check, then **store** the
     result under the key so the next identical run is a hit.
3. The landing queue (the forge's single writer of `main`) triggers execution when a
   batch needs checks it doesn't have cached (e.g. a red PR, a new tree).

Everything below exists to make step 2 (the miss path) **fast, safe, deterministic,
and attested** — because a non-deterministic or unattested result silently poisons the
memo and the provenance chain.

The wire types hugit speaks are **frozen** in `../hugit/crates/hugit-contracts`:
`CheckDef`, `CheckResult`, `RunnerLease`, `QueueApi`, `AttestationChain`,
`FenceManifest`. **The fabric must speak these exact types** (Rust + JSON Schema +
golden serde are pinned there). Treat them as the IDL.

---

## 1. Lease lifecycle  (frozen type: `RunnerLease`)

hugit requests a runner, runs one check, releases it.

- **Acquire:** hugit sends a lease request: `{ tenant, size (vCPU/mem), image (pinned
  digest), ttl, claim (the fence manifest §4) }` → fabric returns a `RunnerLease`
  with a lease id, an exec endpoint, and a deadline.
- **States:** `Pending → Held → (Released | Expired | Crashed)`. hugit's client models
  exactly these; the fabric must not invent intermediate authoritative states hugit
  can't observe.
- **One lease = one isolated job.** No reuse of a dirty runner across leases (a fresh
  microVM per lease — see §3 isolation).
- **Expiry is fail-closed:** at `ttl` the job is killed and the lease `Expired`; a
  half-finished check must **never** produce a stored result (a partial result that
  got memoized is a correctness disaster).
- **Crash recovery:** if the runner dies mid-job, the lease goes `Crashed`, hugit
  retries on a new lease; **no partial/duplicate result** is emitted. (hugit's C2b
  client already drives crash/expiry; the fabric must make these observable + clean.)
- **Concurrency:** many leases in flight per tenant, bounded by §6 caps.

*Interim note:* today the "lease" is hugit SSHing to the box and `docker run`-ing with
`StrictHostKeyChecking=accept-new` + a pinned known-hosts. The fabric replaces this with
an authenticated API (Bearer PAT, same as the CAS/AC), but the **logical lease contract
above is what hugit's client already expects.**

---

## 2. Cache-warm boot  (the speed half of the value)

- The runner must **boot with CoreLink's CAS/AC warm** for the job's working set: the
  check's declared inputs (toolchain, deps, source tree by content) are **already local**
  before the job's first instruction.
- **Warm vs cold:** hugit measures and asserts a warm boot is materially faster than a
  cold one (its C3 oracle). The fabric should expose enough signal (or just be fast)
  that "cache-warm" is real, not nominal.
- **Cache-down is fail-closed:** if CAS/AC is unreachable, the runner must **refuse to
  serve a degraded (cold, unmemoized) result silently** — it returns an explicit error,
  not a result that looks cached but isn't. (hugit's C3 fail-closed expectation.)

---

## 3. Execution + **byte-identical determinism**  (frozen types: `CheckDef` → `CheckResult`)

This is the single most load-bearing requirement, because hugit memoizes by **content**.

- hugit hands the runner a `CheckDef` (the command, declared inputs, the pinned
  toolchain). The runner executes it and returns a `CheckResult` (exit status, the
  canonical result bytes, duration, and the **content digest** of the output).
- **Byte-identity:** the same `CheckDef` over the same inputs MUST produce a
  **byte-identical** `CheckResult` (same content digest) on any runner, any time.
  hugit stores the result under the memo key; if two runners disagree on bytes for the
  same key, the memo is poisoned. The fabric must control the determinism knobs
  (clock, randomness, locale, paths, parallelism nondeterminism, timestamps in
  artifacts) or surface them so hugit's normalization holds.
- **Non-determinism detection:** hugit re-runs and, if a check is non-deterministic
  (differs after **3** runs), flags it rather than memoizing a lie. The fabric only has
  to be *honestly deterministic*; hugit owns the detection, but the fabric must not
  *introduce* nondeterminism (e.g. injecting a per-boot value into the job env).
- **Honest hit-rate:** hugit reports a truthful partial hit-rate; the fabric must report
  truthful exec vs cache-hit accounting (no inflating "served from cache").

---

## 4. Isolation & fences  (frozen type: `FenceManifest`; the untrusted-compute core)

Runners execute **untrusted code** — including code AI agents just wrote. This is the
hard part and the reason hugit will not operate metal itself.

- **Per-claim sparse fence:** a lease carries a `FenceManifest` — the *only* paths the
  job may read/write. The runner must enforce it: a job touching anything outside the
  claimed set is denied (hugit's C5a path-enforcement; absolute-path injection, `..`
  escape, and `srcfoo`-vs-`src/` prefix-collisions are all attacks hugit's tests cover —
  the fabric must hold the same line).
- **No cross-tenant, ever:** strict tenant isolation, fail-closed (CoreLink's HMAC-prefix
  boundary). A job in tenant A cannot observe or touch tenant B's bytes, cache, or runner.
- **No escape:** the sandbox is the trust boundary. hugit has an escape red-team (C5b);
  the fabric should expect to be red-teamed and stay closed.
- **Ephemeral:** the box is destroyed after the lease — no state leaks to the next job.

---

## 5. Secrets broker  (never on the box)

- A job sometimes needs a secret (e.g. a registry token). The secret must be **resolved
  into the job without ever landing on the box image, disk, or argv** — hugit's C5b
  write-only broker model: the credential is delivered through a channel the job can use
  but cannot exfiltrate, and a **credential-scan attestation** proves it's absent
  (`env=0, proc=0, disk=0`) — **fail-closed on any unparseable scan**.
- The fabric must provide (or host) this broker so the same guarantee holds on real metal.
  hugit's broker logic exists; it needs the fabric to honor the "secret never persists on
  the box" property end-to-end.

---

## 6. Budgets, fairness & non-interference  (the X10/X6/C7 bound)

hugit is a **tenant** of CoreLink. A hugit agent-fleet storm must be **structurally
bounded before it can ever degrade CoreLink's launch route or other tenants**.

- **Per-tenant caps:** a request-rate ceiling **and** a concurrency/budget cap on the
  tenant, enforced at the fabric — set *before* load, so the cap is preventive, not
  reactive (hugit's X10⑤).
- **Fairness:** under contention, no single tenant starves others (a p95-wait fairness
  bound; hugit's C7).
- **Non-interference is measurable:** hugit asserts (X6/X10 live seams) that CoreLink
  latency for other tenants is **unaffected** under hugit load. The fabric should expose
  the measurement surface (or hold the SLO) so this can be proven, not assumed.

---

## 7. Attestation & provenance  (frozen type: `AttestationChain`; feeds hugit X8)

- The runner must **attest what it ran**: the **image digest (sha256, pinned — §8)**, the
  resolved inputs, and the result content hash, signed. hugit folds this into its
  `AttestationChain` / transparency log (X8) and its provenance (`why`).
- A result with no/!valid attestation must be **rejected** by hugit — so the fabric
  emitting a correct, signed attestation per execution is mandatory, not optional.

---

## 8. Supply chain  (verify-before-spawn; hugit X4)

- Job images are **pinned by `sha256:` digest** — never a floating tag. The fabric must
  **verify the pinned digest before spawning** and refuse to run an unpinned or
  digest-mismatched image (hugit's X4 `PinnedImage` + verify-before-spawn ordering).
- Dependencies enter via the content-addressed cache (already verified by CAS), not ad-hoc
  network fetches inside the job (which would also break byte-identity, §3).

---

## 9. The trigger path  (frozen type: `QueueApi`; hugit B5 seam)

- hugit's **landing queue** must be able to **trigger check execution on demand**: when a
  batch/PR needs an uncached check (e.g. turns red), the queue calls the fabric to execute
  it, then auto-bisects on failure. This is the `QueueApi` auto-trigger — hugit has the
  logic; it needs the fabric endpoint to call. (Today this is a code stub awaiting the live
  fabric; it is hugit's documented P2 seam.)

---

## 10. What hugit does NOT need from the fabric (scope guard)

- hugit does **not** want the fabric to define check semantics, the memo key, the landing
  logic, or provenance — those are hugit's. The fabric provides **execution + isolation +
  attestation + cache-warm boot + metering**; hugit provides everything above it.
- hugit does **not** want per-minute metering exposed to its customers (one bill, flat —
  see product §5/§7). The fabric meters for COGS/accounting; hugit prices flat on top.

---

## 11. Acceptance from hugit's side (how we'll know the seam is real)

hugit already has the **gated, fail-closed seam tests** that flip live when the fabric
exists (see `../hugit/docs/handoff/2026-06-08-p2-go-live-runbook.md`). The fabric is
"contract-satisfied" when, with `HUGIT_RUNNER_HOST`/the fabric endpoint set:

- B2b runner-side **byte-identity** passes against real fabric execution.
- C3 **cache-warm boot** (warm < cold; cache-down fail-closed) passes.
- C2a/C2b/C9 **lease lifecycle / crash / expiry / ws-lifecycle** pass.
- C5b **secrets-broker escape red-team** passes (credential absent on the box).
- X11 **mid-operation broker fault** degrades fail-closed.
- X6/X10 **non-interference** measures within bound under hugit load.

These are **run-not-skip when the env is set** — they cannot rot to green. The fabric
team can target them directly.

---

## 12. Frozen-from-hugit's-side — change protocol

This contract reflects code hugit has already built and gated. If the fabric needs a
change to any shape or guarantee here, **raise it with the owner / hugit techlead** —
do not assume hugit will adapt. The `hugit-contracts` types are golden-pinned; a change
there is a deliberate, owner-gated event.

---

## 13. Agent/check execution — envelope emission obligations  *(Amendment v1.1)*

> Added 2026-06-10, WP-R6. Closes the deferred dependency flagged when
> ADR-0001 was ratified (hugit `docs/adr/0001-intent-context-envelope.md`,
> ratified same date). The obligations in this section are additive to §0–§12;
> nothing above is relaxed or superseded. The frozen `hugit-contracts` types
> remain unchanged — this section names a subset of them by their Rust paths
> for precision.

When the runner product hosts **agent-driven or agent-triggered check execution**
(i.e. when the job executing on a runner was submitted by or on behalf of an agent
fleet), it acquires three new obligations: **metrics emission**, **capture hook
points**, and **no-persistence**. Each is stated below with the exact guarantee
hugit's forge side expects at the wire.

### 13.1 Per-job metrics emission  (consistent with `IntentMetrics`)

The runner MUST report, at job close, a per-job metrics payload whose fields are
**consistent with** the `IntentMetrics` type frozen in
`hugit-contracts::context_envelope::IntentMetrics` (schema 1.1.0):

| Field | Type | What the runner reports |
|---|---|---|
| `tokens.input` | `u64` | non-cached input tokens consumed by the job's model calls |
| `tokens.output` | `u64` | output tokens |
| `tokens.cache_read` | `u64` | tokens read from prompt cache |
| `tokens.cache_write` | `u64` | tokens written to prompt cache |
| `tokens.total` | `u64` | derived total |
| `wall_ms` | `u64` | job born → job dead wall-clock, ms |
| `active_ms` | `u64` | model + tool busy time, ms (excludes idle/queue wait) |
| `tool_calls` | `u64` | total tool-call count |
| `tool_breakdown` | `[{tool, count}]` | per-tool breakdown (same shape as `ToolCount`) |
| `model_turns` | `u64` | number of model turns |
| `cost_usd` | `f64` | derived COGS in USD (NEVER a billable meter; for trust/audit) |

"Consistent with" means: the runner's payload is a **strict superset-compatible
projection** of `IntentMetrics` — every named field above uses the same name,
the same JSON type, and the same semantic. The runner MAY carry additional
runner-specific fields alongside them (e.g. `cpu_ms`, `mem_peak_mb`); hugit's
forge will ignore fields outside the `IntentMetrics` vocabulary. Missing or
wrong-typed fields in that vocabulary are a contract violation.

**Delivery:** the metrics are returned as part of the `CheckResult` response on
the job-close path (same wire call that returns the result bytes and the
attestation §7). A separate per-job metrics endpoint is acceptable only if it is
always populated before the `CheckResult` is considered final. The forge MUST be
able to read them in the same atomic step as the result — they must never be
optional when the job succeeded.

**The cache split is mandatory.** The forge's cost decomposition (ADR-0001 §2.3
`ci` bucket) distinguishes cache-hit saves from actual execution cost; without
the `cache_read`/`cache_write` split the memoization economics are not
computable. "We don't track cache tokens on the runner side" is not acceptable
for an agent-execution job.

### 13.2 Capture hook points for trajectory blobs  (two-transcript imperative)

hugit's forge maintains the two-transcript imperative (ADR-0001 §2.2, second
owner directive 2026-06-10): at **every altitude** (intent · PR · campaign ·
session) the captured envelope MUST carry both the **full transcript**
(`raw_transcript_ref`) and the **compacted transcript** (`task_transcript_ref`).
These are not optional under the ratified default capture level.

The runner does **not** hold, store, or interpret transcript bytes. That is forge
domain. What the runner MUST provide is:

1. **A streaming hook point** — a channel (e.g. a side-channel pipe, a
   structured event stream, or a callback endpoint) through which the job's
   agent loop can write raw transcript events (model turns, tool calls + results,
   system/charter prompts) as they occur. The hook point is opened at lease
   acquire and closed at job close. Hugit's `hugit-ledger::envelope` producer
   subscribes to this channel and writes the raw bytes directly to cold-tier
   object store via `CAS`. **The runner never buffers or persists these bytes
   beyond in-flight forwarding.**

2. **Trajectory event metadata** — alongside the raw stream, the runner emits
   structured per-turn metadata (turn index, timestamp ms, tool name if
   applicable, token count if known) as a lightweight side-channel. This powers
   the compacted-transcript (`task_transcript_ref`) summariser on the forge side
   without requiring the forge to re-parse the full raw stream.

3. **Job-close signal** — an explicit job-close event carrying the final
   `wall_ms` and `active_ms` values, so the forge-side producer can finalise
   both transcript blobs atomically before the lease transitions to `Released`.
   The forge MUST have written both blobs (or received a confirmed write
   acknowledgement from CAS) before it acknowledges the job-close signal back
   to the runner. If the forge side fails to acknowledge within a
   runner-configured timeout, the runner closes the lease anyway (fail-closed)
   and emits a `capture_incomplete` flag in the `CheckResult`.

**Forge-side producer identity:** `hugit-ledger::envelope` (relocated from
`hugit-runner::envelope` by WP-R4 per the R0 freeze). The runner exposes the
hook point; it does not know or care which hugit module subscribes. The hook
point is authenticated (Bearer PAT, same as the CAS/AC boundary) so only
authorised forge-side callers can subscribe.

### 13.3 No transcript persistence on the runner  (redaction stays forge-side)

The runner is execution infrastructure, not a data store. Therefore:

- **The runner MUST NOT persist transcript bytes** — not to disk, not to a
  local DB, not to any object store the runner controls. The bytes are forwarded
  through the capture hook (§13.2) and then released. An in-flight forwarding
  buffer (ring buffer, bounded FIFO) is acceptable; a write to any durable
  medium is not.
- **Redaction is on the forge's write path.** The forge's `hugit-ledger::envelope`
  producer applies `redaction_policy` (as carried in `Trajectory.redaction_policy`,
  `hugit-contracts::context_envelope::Trajectory`) before committing the blob to
  CAS. The runner forwards raw bytes; it does not scrub secrets or PII. This is by
  design: the forge owns the tenant's privacy contract; the runner must not silently
  drop or modify transcript content before it reaches the redaction stage.
- **Credential-scan attestation (§5) is unchanged.** The §5 guarantee (`env=0,
  proc=0, disk=0`) applies to job secrets/credentials, not to transcript content.
  The transcript channel is a separate, explicitly authenticated stream and is not
  in scope of the credential-scan attestation.

### 13.4 Conformance-vector drift tripwire

The `IntentMetrics` shape named in §13.1 is part of the cross-repo conformance
vector set maintained by hugit (WP-R3/R4 of the runner-transfer campaign,
`../hugit/conformance/`). The same byte-identical vector manifest is committed in
both `hugit` and `corelink-runners`; each side's golden tests pin every vector.

Any change to a field named in §13.1 — name, JSON type, or documented semantic
— is a **contract event** requiring:

1. A new vector version committed byte-identically in both repos (PR in each,
   reviewed against the frozen hugit side before merge).
2. A version bump to the `CONTEXT_ENVELOPE_SCHEMA_VERSION` constant in
   `hugit-contracts::context_envelope` (currently `"1.1.0"`; next would be
   `"1.2.0"` for additive changes).
3. A corresponding amendment to this contract (§12 change protocol applies).

The shared vector manifest is the **drift tripwire**: either side's golden suite
breaks on silent drift, before any integration test is needed. This is the
AC/CAS pattern applied to the runner seam.

---

## Amendment log

| Version | Date | Author | Summary |
|---|---|---|---|
| v1.0 | 2026-06-09 | hugit techlead | Initial contract; §0–§12; frozen from hugit's side. |
| v1.1 | 2026-06-10 | hugit techlead (WP-R6 draft) | Added §13: per-job metrics emission consistent with `IntentMetrics` (§13.1); capture hook points for full + compacted transcript blobs — two-transcript imperative (§13.2); no-persistence + forge-side redaction obligation (§13.3); conformance-vector drift tripwire (§13.4). Cross-reference: hugit ADR-0001 (ratified 2026-06-10). **DRAFT — lead review pending; apply after WP-R4 frees corelink-runners tree.** |

*Change protocol (§12) applies to all future amendments.*
