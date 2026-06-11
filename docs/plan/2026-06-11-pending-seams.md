# Pending-seams register — hugit

> 2026-06-11. Tracked-deferral register. Each entry below is a seam that is
> intentionally deferred with a findable owner, acceptance criteria, and the
> governing document. This file kills "vapor-deferred" (P-DEFERRALS from the
> 2026-06-11 adversarial Round 1 review). Update this file when a seam ships
> or is superseded; never silently drop an entry.

---

## ~~PS-1 — moved to Closed (see table below)~~

*See the Closed seams table for the governing record.*

---

## PS-2 — `--author-kind` authn binding (P2/identity)

**Status:** DEFERRED — requires ADR-0002 identity/P2  
**Adversarial finding:** P-DEFERRALS (Tier 4, `docs/review/2026-06-11-adversarial-round-1.md`);
SOTA audit S3 (`docs/review/2026-06-11-sota-audit.md`)  
**Governing docs:** `docs/adr/0002-hugr-identity.md`;
SOTA audit §S3;
`docs/handoff/2026-06-09-hugr-identity-rollout.md`

**What is deferred:**
- The D14 authz guard (`hugit_refstore::authz::append_authorized` /
  `AuditedGuard`) is golden-tested and wired to the CLI mutation path (Wave A),
  but `--author-kind` is caller-supplied: a subagent can claim `orchestrator` at
  the CLI without any cryptographic check. The guard enforces authorship rules
  correctly given the claimed kind, but the claimed kind is unauthenticated.
- Full fix: author-kind must bind to the authenticated principal from the HuGR
  Clerk-backed identity session (ADR-0002); the forge reads the session token,
  derives `author_kind` from it, and the CLI flag becomes advisory/override for
  the authenticated case only.

**Owner:** hugit techlead (identity rollout assigned to owner milestone P2)  
**Acceptance criteria:**
1. `--author-kind` is verified against the authenticated principal's session
   claims (Clerk / PAT scope); a subagent cannot escalate to `orchestrator` by
   flag alone.
2. The D14 guard oracle is extended with a mutation-verified test: a subagent
   claiming `orchestrator` is denied unless the session token grants it.
3. The CHANGELOG entry for WA2 is amended to reflect the interim limitation
   (already disclosed in the CHANGELOG: "authn binding honestly disclosed as
   the identity/P2 seam").

**Unblocked by:** ADR-0002 identity rollout (`docs/handoff/2026-06-09-hugr-identity-rollout.md`),
which requires P2 CoreLink tenant provisioning.

---

## PS-3 — Refstore cold-tier event-payload erasure (the deferred half of WA3)

**Status:** DEFERRED — production cold-store trait gap  
**Adversarial finding:** SOTA audit S4 (`docs/review/2026-06-11-sota-audit.md`);
implicitly referenced in CHANGELOG WA3 entry  
**Governing docs:** SOTA audit §S4;
`docs/adr/` (ADR-0001 trajectory-tier decision, cold-store decision);
`crates/hugit-invariants/x7/` (X7 acceptance suite);
`crates/hugit-invariants/x12/` (X12 acceptance suite)

**What is deferred:**
- WA3 shipped tombstone erasure on the `hugit_refstore::ColdStore` trait:
  `GetOutcome::{Present,Erased,Absent}` + resurrection-refused. The X7 and
  X12 invariant suites prove the erasure cascade against this trait.
- However, the REAL production cold-store (CloudFlare R2 via the CoreLink CAS
  surface) does not yet implement the `erase` method: `cold_store.rs` (the
  live seam adapter) lacks an erase path. X7/X12 proofs run against the
  in-process `InMemoryObjectStore` toy, not the production trait.
- The event-payload erasure half of the cascade (CAS tombstone → event-log
  event-payload purge on the real R2/D1 backend) is unimplemented.

**Owner:** hugit techlead (requires CoreLink P2 tenant + R2/D1 erase API)  
**Acceptance criteria:**
1. The production `ColdStore` adapter (over CoreLink CAS/R2) implements `erase`:
   issues a real tombstone-write to R2 keyed by content hash.
2. X7 item ① and X12 item ① acceptance suites are re-targeted to run against
   the REAL adapter (not `InMemoryObjectStore`) when `HUGIT_CORELINK_AC_URL` +
   tenant are set (run-not-skip).
3. The cold-store erase path is proven load-bearing: removing the erase call in
   the production adapter turns X7① RED.

**Unblocked by:** P2 CoreLink tenant provisioning (R2 + D1 access);
`docs/handoff/2026-06-08-corelink-p2-tenant-request.md`.

---

## PS-4 — Transplant naming: `HUGIT_RUNNER_HOST` env-var + `hugit-runner` doc title inside corelink-runners

**Status:** DEFERRED — naming drift introduced by runner-transfer campaign (WP-R4, 2026-06-10)  
**Adversarial finding:** Docs/process — Round-2 verdict (`docs/review/2026-06-11-adversarial-round-2.md`);
claimed "tracked" in the Round-1 CHANGELOG but was never entered in this register.  
**Governing docs:** `docs/review/2026-06-11-adversarial-round-2.md` §Docs/process;
`../corelink-runners/docs/spec/hugit-integration-contract.md` (wire contract);
CHANGELOG runner-transfer entry.

**What is deferred:**
- After `hugit-runner` transferred to `../corelink-runners`, the runner product
  retained the crate name `corelink-runner` but the env-var `HUGIT_RUNNER_HOST`
  and the user-facing doc title "hugit-runner" remain in corelink-runners code/docs.
  These identifiers now straddle the product boundary: the env-var name anchors
  the hugit seam contract (fine), but the doc title "hugit-runner" inside a
  CoreLink campaign #1 product doc creates a naming mismatch that will confuse
  operators configuring the runner fleet.
- The fix requires a coordinated rename in `../corelink-runners` (doc title, any
  README references, operator runbook) — read-only from hugit's side (hugit
  consumes `HUGIT_RUNNER_HOST`, the env-var name is intentional product-seam
  naming and does NOT need to change here).

**Owner:** hugit techlead — coordinates with corelink-runners owner  
**Acceptance criteria:**
1. The corelink-runners product docs (CLAUDE.md, operator runbook, any `hugit-runner`
   title references) are updated to use the campaign #1 product name consistently
   (`corelink-runner` / "CoreLink runner"), with a note that the HUGIT integration
   seam uses `HUGIT_RUNNER_HOST` by contract.
2. The hugit `docs/interop.md` seam map is verified: `HUGIT_RUNNER_HOST` is
   documented as the intentional cross-product seam variable (not a naming error).
3. No rename of the env-var itself — it is frozen by the wire contract.

---

## PS-5 — CI fork-guard: `pull_request` trigger on self-hosted runner without fork isolation

**Status:** DEFERRED — accepted LOW risk (private repo today); escalates to HIGH if repo goes public  
**Adversarial finding:** Docs/process — Round-2 verdict (`docs/review/2026-06-11-adversarial-round-2.md`);
claimed "tracked" in the Round-1 CHANGELOG but was never entered in this register.  
**Governing docs:** `docs/review/2026-06-11-adversarial-round-2.md` §Docs/process;
`.github/workflows/ci.yml` + `.github/workflows/dco.yml` (both use `pull_request`
on `[self-hosted, mac, corelink-builder]`).

**What is deferred:**
- Both `ci.yml` and `dco.yml` trigger on `pull_request` without a fork-guard.
  GitHub's `pull_request` trigger for a fork PR runs checkout of the fork's code
  on the self-hosted runner — a fork contributor could execute arbitrary code on
  the corelink-builder machine.
- **Current risk: LOW** — the repo is private; fork PRs require write access; no
  external contributors. The risk profile matches a typical private org repo.
- **If the repo goes public:** risk becomes HIGH (runner compromise from any fork
  PR). The correct fix is `pull_request_target` with an environment protection
  gate OR explicit `github.event.pull_request.head.repo.fork == false` guard.
  See the accepted-risk comment added to both workflow files (WF-DOCS).

**Owner:** hugit techlead — reassess on any repo visibility change  
**Acceptance criteria:**
1. If and when the repo is made public: `ci.yml` and `dco.yml` are updated to
   use `pull_request_target` with a protected environment approval gate for fork
   PRs, OR a fork-origin check (`github.event.pull_request.head.repo.fork == false`)
   with a separate trusted-maintainer path.
2. Until then: the accepted-risk comment in both workflow files (added by WF-DOCS)
   documents the threat model and the fix path, satisfying the audit trail.
3. The transition is tested: a fork PR must NOT run on the self-hosted runner
   without an explicit approver gate.

---

## PS-6 — Queue batch-verdict → queue-show projection

**Status:** DEFERRED — union-batch verdict seam not yet wired  
**Adversarial finding:** Round-4 Cluster D (`docs/review/2026-06-11-adversarial-round-4.md`);
WG-COHERENCE (Wave G) flagged the cross-module half but left `verdict: null` with a
disclosure note; Round 4 confirmed no register entry existed.  
**Governing docs:** `docs/review/2026-06-11-adversarial-round-4.md` §Cluster D;
`crates/hugit-cli/src/queue/mod.rs` (VERDICT_NOTE disclosure); CHANGELOG wedge entry
(corrected: queue show verdict remains null-disclosed; verdict.recorded flows to `proven`
in campaign show, not to the queue projection).

**What is deferred:**
- `hugit queue show` emits `verdict: null` per entry and per batch, with an honest
  `verdict_note` explaining the gap. The `verdict.recorded` event (emitted by
  `hugit verdict --store`) flows to `proven` in `campaign show` (WG-COHERENCE),
  NOT to the queue's per-entry or per-batch verdict field.
- The missing half: wiring the union-batch verdict seam so that when a batch
  completes (all PRs in a campaign land and receive a verdict), the queue
  projection reflects the batch outcome in `verdict` and `implicated_pr`.
- This is distinct from the already-closed PS-1 (recorder verbs): the
  `verdict.recorded` producer is live; the projection from those records INTO the
  queue batch view is the open seam.

**Owner:** hugit techlead  
**Acceptance criteria:**
1. `hugit queue show` entries carry a real `verdict` once `verdict.recorded` events
   cover the batch's intents (not null-disclosed); `implicated_pr` identifies the
   PR implicated by a rejection.
2. The queue projection and `campaign show` agree on the batch verdict for a
   given campaign: both read from the same `verdict.recorded` events on the
   canonical log.
3. A test drives the full cycle: land a PR → record a verdict → `queue show`
   reflects the batch outcome (non-null), coherent with `campaign show proven`.

**Unblocked by:** no P2 dependency — purely a projection seam on the existing
event log. Can be implemented in a future Wave once the other Round 4 code fixes
(WH-SCRUB/WH-CHECK/WH-PROVEN) are merged.

---

## PS-7 — `toolchain-unprobed` fallback: cross-env false-hit risk (accepted, tracked)

**Status:** TRACKED ACCEPTED RISK — no code change required; use `--toolchain`
explicitly in multi-env fleets  
**Adversarial finding:** Round-4 Cluster D (`docs/review/2026-06-11-adversarial-round-4.md`);
noted as untracked cross-env false-hit vector.  
**Governing docs:** `docs/review/2026-06-11-adversarial-round-4.md` §Cluster D;
`crates/hugit-cli/tests/acceptance_wcheck.rs` `omitting_toolchain_yields_a_real_digest_axis`
test (confirms the fallback is NOT the old `local-toolchain` constant).

**What is tracked:**
- When `--toolchain` is omitted and `rustc` is unavailable (e.g., a sandboxed CI
  environment), the toolchain axis falls back to the constant `toolchain-unprobed`.
- Two distinct rustc-less environments will share this constant → the same
  memo key → a potential cross-env false cache hit (a result cached in env A is
  served as a hit in env B, even if the real toolchain differs).
- The distinct-marker approach (a per-env hash or hostname) was evaluated and
  rejected: it would bust the cache for every distinct sandbox even when the
  toolchain is genuinely identical, defeating the memoization value.

**Accepted risk + mitigation:**
- The risk is LOW in practice: environments without `rustc` typically do not run
  Rust checks, and the `toolchain-unprobed` constant is clearly named (not a
  fingerprint).
- **Mitigation: always pass `--toolchain <digest>` explicitly in fleet-dispatch
  contexts.** The `hugit-checks` executor accepts an arbitrary string as the
  toolchain axis; agent orchestrators MUST supply a real toolchain identifier.
- The `--toolchain` flag exists precisely for this case; fleet-facing docs should
  note the requirement (DOCS seam, not a code change).

**Owner:** hugit techlead — no code change required; document the fleet requirement
in operator runbook when fleet-dispatch is wired.  
**Acceptance criteria:**
1. Operator / fleet-dispatch documentation notes that `--toolchain <digest>` is
   REQUIRED in any environment where `rustc` may be absent or non-standard.
2. No code change is needed for this seam — the fallback behaviour is acceptable
   for single-env local runs and explicitly disclosed here.

---

---

## PS-8 — Event-log cryptographic authentication (P2 server-side)

**Status:** DEFERRED — P2 server-side seam; local chain is tamper-EVIDENT, not tamper-PROOF  
**Adversarial finding:** Round-5 Cluster C (`docs/review/2026-06-11-adversarial-round-5.md`);
convergence synthesizer (opus3) — strongest blocker of the round.  
**Governing docs:** `docs/review/2026-06-11-adversarial-round-5.md` §Cluster C;
`docs/handoff/2026-06-11-corelink-p2-ceiling-request.md` Seams D/E;
`crates/hugit-cli/src/checks/run.rs` ~540 (honest AC HMAC disclosure — the peer claim).

**What is deferred:**
- The local hash chain (`verify_chain` in `hugit_refstore::tamper`) detects **partial
  or incomplete corruption** (a naive byte-flip, a dropped record, an out-of-order
  insertion) and provides ordering + append-immutability once published. It does NOT
  prevent a competent local rewriter: an actor with full write access to the shared
  `--log` file can recompute the unkeyed SHA-256 chain forward and forge contents that
  pass `verify_chain` (e.g., reject→approve, failing-check→green). This is physics for
  a local file — no local unkeyed crypto stops the local writer.
- The real authentication against a competent rewriter is **server-side (P2)**: the
  CoreLink per-repo Durable Object event-log (Seam D) enforces server-side single-writer
  append-only chaining, and the transparency log (Seam E) provides an externally-verifiable
  inclusion proof. Together these are the peer of the AC HMAC seam (Seam A) — each seam
  is honestly disclosed in the AC layer (`run.rs` ~540); this register entry makes the
  event-log disclosure match that same honesty level.

**Owner:** hugit techlead (requires P2 CoreLink tenant — Seams D + E of the p2-ceiling request)  
**Acceptance criteria:**
1. The per-repo DO (Seam D) enforces server-side append-only chaining: a client cannot
   submit a rewritten chain that overwrites existing records; the server rejects any
   append that would alter a committed `this_hash`.
2. The transparency log (Seam E) provides an externally-verifiable inclusion proof for
   the event log, so a third party can confirm the log was not rewritten after a given
   `seq`/`this_hash` was published.
3. A competent local rewrite (recomputed chain, altered payloads) is detected and
   rejected by the server when the client attempts to submit the forged log.
4. `verify_chain` continues to run on every read for partial-corruption detection
   (ordering, dropped/inserted records) — this is orthogonal to server-side auth and
   remains the local defence.

**Unblocked by:** P2 CoreLink tenant provisioning; Seams D + E of
`docs/handoff/2026-06-11-corelink-p2-ceiling-request.md`.

---

## Accepted local-tier risks (tracked, no code change required)

The following risks were flagged by Round-5 adversarial agents (Cluster D) and accepted
after review. They are tracked here so the audit trail is complete and future waves can
reassess if conditions change.

### AR-1 — Check double-exec window (accepted)

**Source:** Round-5 Cluster D (`docs/review/2026-06-11-adversarial-round-5.md`);
Wave H (WH-CHECK) introduced lock-only-cache (lock held only for cache ops, not across
execute), which opened a bounded double-exec window between the cache miss and the
store-back. Wave G's "lookup-before-decision lock (no double-exec)" was superseded by
this design.

**Accepted risk:** Two concurrent `hugit check` invocations on the same memo key may
both execute (both cache-miss, both run, both store). The second store is idempotent
(dedup on memo_key + cache_hit → no KPI inflation; the result is byte-identical for the
same inputs). Log integrity is unaffected. The window is bounded to the execution
duration of the check command.

**Mitigation:** Idempotent store-back (WH-CHECK). Acceptable for a local-file Action
Cache; fleet-shared AC (P2 Seam A) inherits its own idempotent PUT semantics from the
CoreLink AC contract (409 on divergent body, no-op on identical).

### AR-2 — `kill(1)` binary portability (accepted)

**Source:** Round-5 Cluster D. `hugit check` uses `kill(1)` (via the system `kill`
command) to terminate child process groups on timeout. This is POSIX-portable but relies
on the `kill` binary being available at `/usr/bin/kill` or on `PATH`.

**Accepted risk:** On a minimal container or unusual platform where `kill(1)` is absent,
the timeout orphan-kill falls back to a softer termination path. The check may not
terminate cleanly, but log integrity is not affected (the record is either written or
not; no partial write).

**Mitigation:** The self-hosted runner (`corelink-builder`, a macOS box) has `kill(1)`.
Accepted for the current deployment target; flag for reassessment if hugit ships on
non-POSIX targets.

### AR-3 — No log rate-limit or quota (accepted)

**Source:** Round-5 Cluster D. The local event log (`--log` file) has no rate-limit or
quota on appends. A runaway agent or a misconfigured fleet can write an unbounded log.

**Accepted risk:** The local log is a single flat file with atomic appends; an unbounded
log grows the file but does not corrupt the chain. The risk is operational (disk space,
projection latency) not integrity-related. Fleet orchestrators are responsible for
log rotation and quota enforcement at the workflow level.

**Mitigation:** Accepted for the local-tier; the P2 DO (Seam D) inherits DO storage
limits from Cloudflare, providing a natural ceiling. No code change needed.

### AR-4 — Orphan grandchild accumulation (accepted)

**Source:** Round-5 Cluster D. When a `hugit check` child process group is killed on
timeout, grandchildren spawned by the check command (e.g., rustc spawned by cargo) that
have already migrated to a different process group are not guaranteed to be killed.

**Accepted risk:** Orphaned grandchildren consume CPU/memory until they exit naturally.
They do not affect log integrity, memo-key correctness, or chain validity. The bounded
execution duration cap (WH-CHECK 300s) limits the worst-case accumulation window.

**Mitigation:** The process-group kill (WH-CHECK) catches the common case; true
grandchild orphans are an OS-level concern accepted for the local runner tier. The
Cloudflare Workers-based P2 execution model (Seam D/runners) does not have this problem
(container isolation).

---

## Closed seams (reference — do not re-open without owner approval)

| Seam | Shipped | Governing commit |
|---|---|---|
| **PS-1 — Recorder verbs (`hugit check` / `hugit verdict` / `pr.landed`)** | 2026-06-11 (wedge wave W0→W-INT) | CHANGELOG `8b3c9c1` (wedge entry); CHANGELOG WB2 entry (checks show); see note below |
| `cost_usd f64 → cost_usd_micros u64` (WA4, contract 1.2.0) | 2026-06-11 | CHANGELOG WA4 entry; corelink-runners contract §12 amendment |
| D14 authz guard wired to CLI mutation path (interim) | 2026-06-11 (WA2) | CHANGELOG WA2 entry |
| WA3 tombstone erasure on `ColdStore` trait (in-process) | 2026-06-11 | CHANGELOG WA3 entry |
| `hugit checks show` / `hugit queue show` CLI verbs (WB2) | 2026-06-11 | CHANGELOG WB2 entry |
| `hugit campaign` / `hugit intent` / `hugit pr` porcelain | 2026-06-10 | CHANGELOG CLI porcelain entry |

**PS-1 closure note:** `hugit check` / `hugit verdict` / `pr.landed` are dispatched
end-to-end on the LOCAL file-backed Action Cache (`<log>.ac`) — the wedge is
observable locally TODAY. `hugit checks show` reports a real hit-rate (non-null
KPIs) over a log with `check.recorded` events. The **LOCAL half is closed**. The
LIVE fleet-shared Action Cache transport (Seam A of the P2 ceiling request,
`docs/handoff/2026-06-11-corelink-p2-ceiling-request.md`) remains P2-pending:
`hit_rate` is structurally local-only until the live AC is wired. P2 does not
re-open PS-1; it is a separately-tracked infra seam.

*Change protocol: amend this file with a `docs(truth):` commit whenever a seam ships or a new deferral is introduced. Never silently drop a pending seam — move it to the Closed table.*
