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

## PS-6 — Queue batch-verdict → queue-show projection (RESOLVED 2026-06-15)

**Status:** RESOLVED — `queue show` now projects a REAL per-entry + per-batch
`verdict` + `implicated_pr` from the SAME `verdict.recorded` events `campaign
show` reads (the shared `hugit_ledger::Ledger` reject-sticky fold), so the two
views AGREE by construction. No P2 dependency was needed (a pure projection seam
on the existing log, as predicted below). A union batch's verdict is `"reject"`
if any member intent carries an outstanding (sticky) reject — `implicated_pr`
names the first such PR in queue order — `"approve"` once every member intent is
proven, and `null` (with the disclosing `verdict_note`) while no `verdict.recorded`
event covers the batch yet (honest unknown, never a faked pass/fail). The join is
by `intent_id`; a recorded intent_id is always a safe-address shape (the door
rejects a secret-shaped `--id`/`--intent` at input), so the ledger's view-boundary
redaction is a no-op on it and the lookup is exact. Cold-verified end-to-end:
`acceptance_wb2::queue_show_projects_real_union_verdict_and_blame` (auth union has
a reject → batch rejects + blame PR 3; billing union all-approve → approves) and
`queue_show_verdict_is_null_until_a_verdict_covers_the_batch` (the honest-null
floor). See the Closed seams table.

**Status (historical):** DEFERRED — union-batch verdict seam not yet wired  
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
   remains the local defence. **(Round 7 honesty correction, closed by Wave K / K-CHAIN
   `def8a18`):** this criterion was momentarily FALSE — `hugit why` and `hugit export`
   were two read paths that skipped `verify_chain` and projected a tampered log as
   authoritative. Both now verify the chain and return `chain_broken` exit-2 on
   corruption (tests `tampered_chain_is_chain_broken_exit_two_on_{why,export}`); the
   every-read claim is true again.
   **(Round 8 honesty correction, closed by Wave L / L-B `integ/wave-l`):** the
   every-read claim was STILL false — `hugit intent list --log` (`intent::list::
   resolve_landed`) was a THIRD read path that rehydrated and projected landed-state
   WITHOUT `verify_chain`. Fixed: `resolve_landed` now verifies the chain → tampered
   log yields `chain_broken` exit-2 (test `acceptance_round8_readpath.rs`). **Structural
   residual (defence-in-depth, NOT a live hole):** `verify_chain` is still called
   ad-hoc per loader — five loaders (`intent/list`, `intent/canonical_log`, `pr/cli`,
   `campaign/world`, `export/cut`) each duplicate the read→verify pattern inline rather
   than routing through one `load_verified_log` chokepoint. All five now verify
   correctly, but a NEW read verb can still forget. The single-chokepoint refactor that
   makes forgetting structurally impossible is tracked as **PS-13** below. This is the
   third consecutive round (R5/R7/R8) the "every read" claim has needed a correction —
   the per-verb pattern is the recurring root; PS-13 closes it.

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

### AR-5 — Unprefixed high-entropy credential in an identifier field (accepted physics)

**Source:** Round-7 (`docs/review/2026-06-12-adversarial-round-7.md`, finding #9). Identifier
fields (`--campaign`/`--id`/`--pr`/`--run-id`/`--tree-hash`) are stored UNREDACTED by design —
they are addresses, not free text — and the structural-secret door rejects only values that
carry a recognized credential PREFIX/shape (`ghp_`, `xoxb-`, `sk-`, `Bearer`, `AKIA`, `eyJ`,
PEM, conn-string, `cas:<credential>`). An UNPREFIXED high-entropy string (e.g. a generic
40-char API key with no vendor marker) in an identifier field therefore survives verbatim.

**Accepted risk:** A random 40-char credential with no structural marker is
**information-theoretically indistinguishable** from a legitimate address (a ULID, a 40-hex
SHA, a content-address). Adding an entropy heuristic to the door would reject real addresses
(false positives that break addressability — the one thing identifiers must preserve). The
decided posture (lead, 2026-06-12): **do NOT add an entropy heuristic**; accept that an
unprefixed secret deliberately smuggled into an identifier field is the operator's error,
not a redaction defect. The forever-log's defence is the prefix/shape detector (closes every
*recognizable* credential class) plus operator discipline. Revisit only if a forcing function
(a real vendor token format with no prefix) appears.

**Not mitigated by code; tracked as accepted.** The matrix tests the recognizable classes;
the unprefixed-entropy class is documented here as out-of-scope-by-physics.

---

## PS-9 — Intent store vs. log: `intent show`/`intent list` read a cwd-global index (RESOLVED 2026-06-13)

**Status:** RESOLVED — owner DECIDED the SOTA hybrid (2026-06-13) and it is implemented + cold-verified.  

**DECISION (owner, 2026-06-13): the SOTA hybrid — global view, truthful per-source-log
`landed`, `--log` as a scope filter.** Implemented:
- `intent new --log L` records `L` as the intent's OWNING log in the store
  (`IntentStoreFile.source_logs`, a `serde(skip_serializing_if = is_empty)` map → zero
  on-disk shape change for stores that never used `--log`; the reconcile path records it
  for healed intents too).
- `intent list`/`show` resolve each intent's `landed` against ITS OWN owning log, so the
  default global (one-call) `intent list` is a TRUTHFUL fleet view — each item carries a
  new `log` field (which log owns it) and a per-intent `landed`. A recorded-but-missing
  owning log → `landed:null` (honest unknown, never a misleading `false`); a TAMPERED
  owning log fails the whole call closed (`chain_broken`/exit-2 — the shared read-path
  invariant, memoized per distinct log).
- `--log` on `intent list` is now a SCOPE FILTER (restrict to intents authored against
  that exact canonical log), replacing the old resolve-against-this override.

Cold-verified: `acceptance_wbint` ③ rewritten to assert BOTH the global truthful view
(i1 landed:true vs its own log + `log` set; i2 landed:null + `log` null) AND the `--log`
scope filter (only i1 shown); full `hugit-cli` suite green; the read-path tamper tests
(`acceptance_round8_readpath`, `acceptance_wave_m_readpath`) still fail closed.

**Original framing (kept for the record):**
**Status (historical):** TRACKED — design decision pending owner input; DO NOT fix the code until the owner decides  
**Adversarial finding:** Round-6 Cluster D (`docs/review/2026-06-11-adversarial-round-6.md`);
product-correctness seam for multi-log agent fleets.  
**Governing docs:** `docs/review/2026-06-11-adversarial-round-6.md` §Cluster D;
`crates/hugit-cli/src/intent/` (show/list verbs); `docs/plan/decomposition.md` (intent design).

**What is tracked:**

`hugit intent show` and `hugit intent list` read from the cwd-global
`.hugit/intents.json` store — an index that spans ALL logs that have ever
written intents under the current working directory. This gives a cross-log
merged view and shows `landed: null` for intents whose PRs have not yet
landed (decoupled from any specific `--log` file). The `--log` flag is NOT
consulted for these two verbs; the intent store is maintained independently
of the event log.

**The design question (two options):**

1. **Per-log store (current default for writes; show/list diverge):** intent
   records are written to a per-log store keyed by `--log` path AND to the
   cwd-global index. `intent show`/`intent list` only surface intents whose
   source log matches the active `--log` flag. Upside: scoped to the current
   workflow. Downside: a multi-log fleet (N agents, N log files) cannot get a
   merged view across campaigns; each agent only sees its own log's intents.

2. **cwd-global index (current show/list behaviour):** `intent show`/`intent
   list` read the cwd-global `.hugit/intents.json` without filtering by
   `--log`. Upside: an orchestrator at the repo root sees ALL intents from ALL
   agents' logs in one `intent list` call — the natural multi-log fleet view.
   Downside: if two agents write conflicting intent IDs to separate logs, the
   merged index may surface both, with no log-scoping to disambiguate.

**The multi-log-fleet UX problem:**

In a fleet of N concurrent agents (N worktrees, N `--log` files), the
`intent list` verb is the orchestrator's primary view of what work is queued.
If the view is per-log, the orchestrator must call `intent list --log <path>`
once per agent log to get the full picture — O(N) calls, no merged projection.
If the view is cwd-global, one call suffices, but `landed: null` for a PR
that landed in a different log's workflow may appear as "not yet landed" in
the global view even if the intent IS landed in its own log.

**Owner decision required:**

- Should `intent show`/`intent list` be scoped to `--log` (per-log, N-call
  fleet model) or cwd-global (one-call orchestrator view)?
- Is `landed: null` for cross-log intents an acceptable state in the global
  index, or should the store track which log owns which intent?
- If cwd-global wins: should `intent list` expose a `--log <path>` filter
  flag to let callers scope the view when needed?

**Owner:** owner decision pending — do not change the code until decided.  
**Acceptance criteria (after owner decision):**
1. The chosen model is documented in `docs/plan/decomposition.md` and the
   CLI `--help` text for `intent show`/`intent list`.
2. If per-log: `intent list` without `--log` either errors or defaults to the
   cwd global index with a clear disclosure; fleet docs note the N-call pattern.
3. If cwd-global: `landed: null` is documented as the expected state for
   intents whose owning log has not yet recorded a `pr.landed` event; the
   index TTL/rotation strategy is defined.

---

## PS-10 — AC write-boundary axis guard lives only on the CLI `FileAc` (RESOLVED 2026-06-15)

**Status:** RESOLVED — the structural-secret axis guard is now enforced on the
SHARED `hugit_checks::client::ac::ActionCache` layer, so EVERY backend (`FileAc`,
`InMemoryAc`, `HttpAcClient`) refuses to persist a `CheckResult` whose any memo
axis carries a secret shape — not only the CLI `FileAc`. The hoist is a
`pub fn guard_axes_not_secret(result)` in `hugit-checks/src/client/ac.rs` (placed
exactly like the already-shared `verify_hit`), called at the top of all three
`store` impls. The predicate is `!hugit_ledger::secret_shape::is_safe_identifier_shape(axis)`
— **byte-equivalent** to the CLI's prior `structural_secret_scrub(v) != v` (a value
is a secret iff it is not a provable safe-address shape), so the CLI `FileAc`
behavior is UNCHANGED (it now delegates to the shared fn and dropped its local
copy). `hugit-checks` gained a `hugit-ledger` path-dep (no cycle — hugit-ledger
depends only on contracts + refstore; `cargo update` locked **0** new external
packages, a single-line lock diff). All `hugit-checks` + `hugit-cli` AC tests
green (the fixtures use 64-hex / short-slug / empty axes, all safe shapes — no
regression). This is the close-by-construction completion of the WK-AC
defense-in-depth — the shared trait's contract is no longer weaker than one of
its implementations. See the Closed seams table.

**Status (historical):** TRACKED — defense-in-depth completeness, no code change
required until P2.

WK-AC (2026-06-12) closed the live `.ac` toolchain-digest leak with **two**
layers: (1) the DOOR — `hugit check --toolchain`/`--def` now reject a
structurally-secret value at input (exit-2 `secret_in_identifier`), reusing the
one `porcelain::structural_secret_scrub` detector; and (2) a fail-closed
write-boundary guard inside the CLI's `FileAc::store` that refuses to persist a
`CheckResult` whose any memo axis (`tree_hash`/`def_digest`/`toolchain_digest`)
is structurally a secret (refuse, never scrub — scrubbing an axis would change
the recomputed `memo_key` and break every cache HIT).

**The remaining seam (NOT an open leak):** layer-2 lives in `hugit-cli`
(`FileAc`), not on the shared `hugit_checks::client::ac::ActionCache::store`
trait — so the `InMemoryAc` (test) and `HttpAcClient` (P2) backends do not yet
carry the axis guard. This is defense-in-depth completeness, not a live hole:
the DOOR (layer 1) rejects a secret `--toolchain`/`--def` for **every** CLI verb
before any backend is reached, so no secret axis can arrive at the in-memory or
HTTP backend via the CLI surface. The guard is single-crate today because the
detector lives in `hugit-cli`/`hugit-ledger` and `hugit-checks` does not depend
on them.

**Owner:** accepted as-is for the local tier.  
**Acceptance criteria (when the P2 live AC / `HttpAcClient` is wired):** hoist a
structural-secret axis predicate into a crate `hugit-checks` can depend on
(e.g. `hugit-ledger`), and move the fail-closed axis guard onto the shared
`ActionCache::store` trait — mirroring how `verify_hit` is shared across all
three backends (`ac.rs` doc) — so the in-memory and HTTP backends enforce it
identically. Tracked; no code change required until P2.

---

## PS-11 — Check `env_manifest` captures only an allowlist; ad-hoc `--cmd` custom env vars are not keyed (RESOLVED 2026-06-13 — `--env-axis` shipped)

**Status:** RESOLVED — the soundness hole was already closed by L-C (hermetic clear); the
remaining FEATURE residual (a check that LEGITIMATELY needs a custom env var) is now shipped as
the owner-approved opt-in `--env-axis <VAR>` flag.

**SHIPPED (2026-06-13): `hugit check --env-axis <VAR>` (repeatable).** A caller-declared var is
added to the SINGLE captured-env source (`captured_hermetic_env`) so it is BOTH (a) folded into
the memo-key env manifest (a change to its value is a MISS) AND (b) passed through to the hermetic
spawn (the spawn otherwise clears it). "declared == keyed == present" by construction — the same
invariant the allowlist vars hold — so a declared dependency is sound (never a stale green), while
an UNDECLARED custom var stays cleared (hit-rate preserved). The var's value is hashed into
`def_digest` before any persistence (only `def_digest` reaches the log, never the raw manifest),
so a declared var may even be a secret without leaking. Cold-verified end-to-end against the real
binary (`acceptance_ps11_env_axis`: declared var value change → MISS; undeclared same change → HIT)
plus pure unit tests on the capture/manifest logic (no process-global env mutation). The
do-NOT-silently-widen-the-allowlist guard stands — `--env-axis` is explicit, per-invocation, and
narrow.

**Original framing (kept for the record):**

K-RUN (`def8a18`, Round 7 finding #4) closed the wedge stale-green by folding a CANONICAL,
SORTED snapshot of an ALLOWLIST of result-affecting env vars into the memo key (`RUSTFLAGS`,
`RUSTDOCFLAGS`, `RUSTC*`, `RUSTUP_TOOLCHAIN`, `CC`/`CXX`/`CFLAGS`/`CXXFLAGS`/`LDFLAGS`/`AR`,
and the `CARGO_`/`RUST_`/`CARGO_BUILD_` prefixes). A change to any of those now busts the key.

**The residual (disclosed, by design):** an **ad-hoc `--cmd` check that reads a CUSTOM env var
NOT on the allowlist** (e.g. `MY_GATE_MODE`) is still served a warm HIT when only that var
changes — a missed-miss for that specific check. Full-environment capture was rejected because
it would fold `PWD`/`SHLVL`/etc. into the key and destroy the hit-rate (the wedge's value).

**Owner/lead decision pending — the proposed fix (K-RUN escalation):** an opt-in
`--env-axis <VAR>` flag (repeatable) that adds caller-named vars to the keyed manifest, so an
ad-hoc check can declare exactly the env it depends on. Until then the allowlist covers the
realistic Rust/Cargo toolchain vectors; an ad-hoc check with a custom env dependency must
either use an allowlisted var or accept the documented residual. Do NOT silently widen the
allowlist (that is the "exemption is a hole" failure mode in reverse — an over-broad key
destroys the wedge).

**Wave L / L-C update (Round 8 C3 — the stale-green RESIDUAL is now CLOSED, not just narrowed):**
the K-RUN allowlist was an *enumerate-the-inputs* approach (a denylist-of-the-unknown — the
recurring root). L-C replaced it with **hermetic execution**: the check spawn now `env_clear()`s
and sets ONLY the captured allowlist + a pinned, hashed `PATH`, and pins `cwd` to `--root`. A
custom var NOT on the allowlist (e.g. `MY_GATE_MODE`) is therefore **CLEARED before the spawn**,
so the check reads it UNSET deterministically — it can no longer produce a stale HIT off-key.
The missed-miss is gone. What REMAINS of PS-11 is purely a *feature* request, not a soundness
hole: a check that LEGITIMATELY needs a custom env var must declare it (the `--env-axis` opt-in
above) since it is now cleared rather than silently inherited. The soundness scope L-C does NOT
close — files outside `--root`, network, the clock — is the disclosed **P2 runner-sandbox seam**
(see PS-8's family / the corelink-runners seeded-rootfs); the local executor mirrors that
contract's soundness for cwd/env/PATH and does not exceed it.

---

## PS-12 — Self-hosted runner missing `cargo-deny` / `cargo-audit` (RESOLVED 2026-06-12)

**Source:** Round-7 — CI run 27422619640 (the Wave J + WK-AC push) concluded FAILURE; the
failing step was `gates/deny` exit **127** (`cargo-deny` binary not found on the self-hosted
runner). Locally `cargo deny check` passes (exit 0) and covers the RustSec advisory DB; the
standalone `cargo-audit` was also absent locally. So the advisory gate ran LOCALLY but the
self-hosted runner could not run it — a green LOCAL gate was not reproduced on CI for that step.

**RESOLVED (2026-06-12, Wave L push — CI run `27440432238`):** the workflow now provisions the
advisory tooling on the runner via `taiki-e/install-action`; the `gates` job ran
`fmt`/`clippy`/`test`/`deny`/`audit` ALL green on `corelink-builder` and concluded `success`.
The advisory gate is now runner-verified, not LOCAL-only — the "concluded remote green is the
source of truth" caveat in CLAUDE.md is closed.

**PS-12b — runner contention disrupts gate-step env (ambient infra, owner/P2; mitigated 2026-06-13):**
the shared `corelink-builder` runner intermittently loses the toolchain action's `$GITHUB_PATH`
PER-STEP — observed `cargo: command not found` in a single step (e.g. `test`, or `deny`) while
OTHER steps in the SAME run found cargo (~37% flake; a concurrent family-repo job mutating the env
during the ~10-min test step is the likely cause). **Mitigated** (`3b6924e`): each gate step now
prepends `~/.cargo/bin` to PATH itself, independent of `$GITHUB_PATH` — this got the test step green
where it had flaked twice. **NOT fully resolved**: a long step + cross-repo contention can still
disrupt the env; the real fix is **dedicated runner capacity / job isolation** (part of the P2
runner provisioning — owner/infra). The CODE is runner-green-capable (run `27457719247` concluded
`success`, all 5 gates green on the runner); a contention flake on a future push is re-enqueueable
(`gh run rerun --failed`), NOT a code failure.

---

## PS-13 — Read-path `verify_chain` per-loader, not one chokepoint (RESOLVED 2026-06-12 — Wave M / M-1)

**Source:** Round 8 C2 (the read-path class audit). Every log read MUST `verify_chain` before
projecting content as authoritative. This is TRUE on `integ/wave-l` for all five current loaders
(`intent/list`, `intent/canonical_log`, `pr/cli`, `campaign/world`, `export/cut`) — but each
calls `verify_chain` AD-HOC after its own inline read→rehydrate loop. A NEW read verb can forget,
exactly as `why`/`export` (Round 7) and `intent list` (Round 8) did. This is the recurring root
behind three consecutive corrections of the PS-8 "every read" claim (R5/R7/R8).

**Status:** NOT a live hole (all five verify correctly; cold-verified by tamper-repro on
`intent list` + the existing `why`/`export` tests). This is a **structural-hardening** item.
**Proposed fix:** a single `load_verified_log()` that performs read→rehydrate→`verify_chain` and
is the ONLY way to obtain a projectable log; make the raw `EventLog::new() + push_record` read
pattern `pub(crate)`/unreachable from verbs, so forgetting is a COMPILE error (the same
"close-by-construction" pattern Wave L applied to redaction (L-A), the append door (L-D / C4),
and the seal guard (L-D / C5-F2)).

**RESOLVED (2026-06-12, Wave M / M-1):** the single chokepoint exists —
`checks::rehydrate_and_verify` is the SOLE production site of `EventLog::new() + push_record +
verify_chain`; `load_event_log` delegates to it. The four canonical loaders (`intent/list`,
`intent/canonical_log`, `pr/cli`, `campaign/world`) AND the M-3 `intent/new.rs` reconcile path all
route through it, and a source-invariant test (`canonical_log_loaders_route_through_the_chokepoint`)
FAILS THE BUILD if any of those files hand-rolls `verify_chain(`/`.push_record(` — so forgetting is
now a compile error. `why`/`export` read DIFFERENT on-disk shapes and verify their own with their
own `verify_chain` (documented special-shape siblings, intentionally outside the canonical set);
`export::restore_from_bytes` skips verify but is not CLI-wired (dead, R10-2 F-2). Round 10 (class 2)
cold-verified every read verb tampered→`chain_broken` exit-2 incl. the reconcile (fail-closed on a
tampered log). The PS-8 "every read verifies" claim is now STRUCTURALLY guaranteed for the canonical
loaders, not just true-today.

---

## PS-14 — Deny-by-default identifier scrub: hex/numeric exemption tuning (DECIDED 2026-06-12 — hybrid implemented; low-entropy-base32 residual accepted)

**Source:** Round 9 C1 re-audit. Wave L / L-A inverted the identifier scrub to deny-by-default
(`is_safe_identifier_shape` — a value survives verbatim only if it proves a bounded safe-address
shape). This **CLOSED the Round-8 P0** (prefix-less dense SaaS keys — AWS/SendGrid/Stripe/base64
— are now rejected/redacted, cold-verified). The residual: the safe-shape gate uses an entropy
threshold (≈4.5 bits/char) as the discriminator for charset values, but **hex has a per-char
entropy CEILING of log2(16)=4.0 < 4.5**, so ANY-length hex always passes — not just the {40,64}-hex
content-address/digest shapes. The R9 auditor live-confirmed that a 32-hex API key, a 50-hex
secret, a base32 TOTP seed, and a 24-digit numeric secret **survive verbatim** at rest. They are
indistinguishable from a legit address (a 64-hex *could* be a sha256 OR a key) on the **physics
boundary**, but the survivor band is **WIDER than the "{40,64}-hex irreducible residual" the L-A
docs/matrix claimed** — so this is at minimum a documented-residual honesty correction, and a
genuine tuning knob.

**Severity:** P1 (honesty/seam) — **NOT a P0 class re-open** (deny-by-default holds; dense
credentials are caught). **No autonomous code change made:** tightening the gate (e.g. length-pin
the hex exemption to {40,64}, cap numeric length, raise/replace the entropy rule) risks
**over-scrubbing legit short identifiers** — git short-hashes (7–12 hex), small integer `--pr`/
`--run-id` values, short slugs — which is a correctness/UX regression. That security-vs-over-scrub
trade is the **owner's call**.

**DECIDED (owner, 2026-06-12): the HYBRID — implemented.** `is_safe_identifier_shape` now
redacts any BARE all-hex/all-numeric value of length ≥ 20 that is NOT a {40,64}-hex digest
(those, plus `cas:` refs and ULIDs, survive at `is_digest_shaped`/`is_ulid_shaped` above). This
catches odd-length hex (32/50) and long-numeric (24-digit) secrets while keeping INTEGERS and
SLUGS generous: short integers (`--pr 7`, a CI `--run-id 12345`) are below the length floor;
UUIDs (hyphens → not all-hex), prefixed ids (hugit's own `intent-<16hex>`), bare ≤19-char short
hashes, and slugs are untouched. Cold-verified end-to-end through the identifier door: 32-hex /
50-hex / 24-digit-numeric → `secret_in_identifier` exit 2 (not stored); 40-hex / 64-hex / ULID /
UUID / integer / `intent-<16hex>` / slug → survive. Regression specimens added to the
`is_safe_identifier_shape` unit test (R9-2 guard-matrix gap closed).

**Residual after the hybrid (accepted physics, documented):** a LOW-entropy base32 value (e.g. a
repetitive TOTP seed, Shannon ≈ 3.4) is neither all-hex nor structural-secret-shaped and clears
the entropy gate — indistinguishable from a legit slug, so it survives. This is the irreducible
boundary (you cannot tell a low-entropy base32 secret from a low-entropy base32 address). **R9-3
RESOLVED (2026-06-12, Wave M / M-2):** the two scrub engines no longer drift by hand — the shared
structural-secret / content-address-shape / entropy primitives now live in ONE module
(`crates/hugit-ledger/src/secret_shape.rs`); `porcelain.rs` (identifier door) and `redact.rs`
(free-text) both call it, keeping their DISTINCT policies (door deny-by-default 4.5 + {40,64}-hex
pin; free-text 4.0 + bare-hex exemption) but on a single source of truth. Round 10 (class 1)
cold-verified ZERO behavior drift (a pre-M2 vs post-M2 binary differential was byte-identical over
162 inputs × 2 layers). The PS-14 hybrid remains identifier-door only (free-text hex tightening is
a higher-over-scrub trade, not in scope).

---

## PS-15 — Review-sweep (2026-06-12) deferred items (non-blocking; tracked so nothing is silently dropped)

A 7-agent code/audit/perf/SOTA review sweep (`docs/review/sweep-2026-06-12/`) ran on `main 5457730`.
The SHIP-BLOCKER (memo stale-green via the dropped POSIX mode bit) and the two other real defects
(O(n²) intent reconcile; `hugit-policy` scrub-engine drift) were fixed in **Wave N**. The remaining
findings are tracked here as accepted/deferred:

- **PERF (verify_chain is O(n) per read)** — every `--log` read re-verifies the whole hash chain
  (measured ~103 ms @ 10k events). This is largely INHERENT for correctness (you must verify what
  you project, and a cross-process prefix-memo is unsound — the on-disk log can change between
  invocations). **Honest correction (2026-06-15):** the "verify-once per verb invocation" half of
  this win is **already achieved** — `checks::rehydrate_and_verify` is the single load chokepoint
  (PS-13/M-1), and `intent list` memoizes per distinct log (`landed_cache`), so a recon traced
  `intent list` / `campaign show` calling `verify_chain` EXACTLY ONCE per invocation — there is no
  double-verification to remove. What genuinely remains is only a per-record alloc micro-shave
  (reuse the `compute_this_hash` buffer / drop the per-record hex `String`), a small-constant win
  on the most-audited integrity-critical code; **DEFERRED by the lead** — marginal at current log
  sizes and the regression surface on the hash spine is not worth it without a measured need. The
  O(n) re-verify itself is inherent and stays. (No code change; the "verify-once" framing is now
  recorded as solved, not pending.)
- **ERGONOMICS F-2 (error JSON key order) — RESOLVED 2026-06-15.** `kind` is now emitted FIRST
  (`kind`→`message`→`fix`→context) in every agent-facing CLI error envelope. Both porcelain-error
  twins (`porcelain::PorcelainError` and `intent::error::PorcelainError`) render through one shared
  `porcelain::ordered_error_object` builder that assembles the object in explicit order (each value
  via `serde_json::Value`'s always-valid-JSON `Display`; only `{`/`}`/`:`/`,` hand-written) —
  **zero new dep**, no workspace-wide `preserve_order`/`indexmap`. All four stdout error paths
  (`main`, `checks`, `verdict`, `queue`, `intent`, `pr`) route through the string renderer; the
  `PrError::to_json()->Value` library API is never printed (Display uses the string). Cold-verified
  live: `hugit queue show --log /no/such` emits `{"error":{"kind":"log_not_found",…}}`. Existing
  field-access error tests stay green (order-independent).
- **TEST-QUALITY debt** (sweep `tests.md`): T-1 tautological shadow-scheduler test
  (`hugit-checks/src/shadow/tests.rs`); T-3 `unsafe set_var` in parallel test binaries (cross-file
  env race); T-5 scratch-dir PID-only collision (`acceptance_wj_matrix.rs`); T-6 presence-not-field
  redaction assert; T-8 zero property/fuzz tests on the scrubber/chain/memo-key. Hardening backlog.
- **CODE-REVIEW F2 (PS-13 invariant scans a hard-coded file list)** — a new read verb in a NEW file
  escapes the source-invariant. Defence-in-depth (every current loader IS policed); harden by
  walking `src/**.rs` minus tagged exemptions.
- **N-1/N-6 false-positive corrected:** the sweep's "package count is 16 not 17" finding was WRONG —
  `cargo metadata --no-deps` shows 17 workspace members (4 `hugit-app*` + 13 feature crates); the
  CLAUDE.md "17-package" claim is correct and was NOT changed.

---

## PS-16 — Post-sweep round 2 (2026-06-12/13): Wave O hardening + a 3rd wedge stale-green (closed) + residuals

After Wave N, a focused re-audit of the WEDGE stale-green class (`docs/review/sweep-2026-06-12/wedge-stale-green-reaudit.md`)
+ a property/fuzz pass (Wave O) ran. Outcomes:

- **The wedge stale-green class needed FIVE local fixes — now CONVERGED-local (Round 13).** (This
  bullet once claimed closure after the 3rd fix — an OVERCLAIM Round 11 caught; corrected.) The five
  bounded local holes, each found by an adversarial round and fixed + cold-verified: (1) env → K-RUN;
  (2) file mode → N-1, refined to exec-only `0o111` in Wave P (Round 11 caught N-1 over-capturing the
  umask → cross-runner hit-rate loss); (3) in-root toolchain config → WO-GLOBMISS (per-def globs);
  (4) ANCESTOR toolchain config (`.cargo/config.toml`/`rustfmt.toml`/`rust-toolchain.toml`, cargo
  searches UPWARD) → Wave P (`ancestor_config_digest` walks up + `$CARGO_HOME`); (5) ANCESTOR
  `Cargo.toml` (`[workspace.lints]`/`[profile]`/`[patch]`) + `Cargo.lock` → Wave Q (Round 12 found it).
  This COMPLETES the finite, known cargo/rustc config set. **Round 13 (fresh decider) confirmed
  CONVERGED-local:** all 5 fixes hold, NO 6th bounded hole (the rustup toolchain override is
  captured-by-effect via the live `rustc --version --verbose` probe; `[env]`/`[patch]`/`[profile]`/
  `CARGO_TARGET_DIR` all captured; determinism sound both ways — identical trees → same key, 1-byte
  diff → no collision). Waves P/Q regression-clean (no abs-path leak into the digest, no panic on a
  permission-denied/weird-`CARGO_HOME` ancestor, symlink-loop-safe, 120-deep walk 0.15s).
- **RESIDUAL — the recurring ROOT = a non-hermetic command, genuinely closed only by the P2 seam.**
  Four holes from one class: you cannot soundly memoize a NON-hermetic command by ENUMERATING its
  inputs — each round found one more axis. The bounded, KNOWN axes are now captured locally (env,
  exec-mode, in-root + ancestor toolchain config). The GENUINELY irreducible residual is the UNBOUNDED
  read: a `test` reading an ARBITRARY fixture (`tests/data/*`, `include_str!`, a `build.rs`-emitted
  path), network, or the clock — no enumeration predicts these. The class-killing fix is **hermetic
  execution** (the runner's isolated rootfs so the action physically cannot read outside the seeded
  tree axis) — the **P2 corelink-runners sandbox seam** (`docs/interop.md` §2 / Round-8 C3). That
  residual is honestly P2; **Round 13 confirmed the local bounded axes are all captured (dry)** — the
  unbounded read is the only surviving stale-green vector, and it is the documented P2 hermetic seam.
  Narrow accepted edge: the toolchain digest hashes `rustc --version --verbose`, not the binary bytes
  (two rustc with identical version strings would false-HIT — vanishingly unlikely, accepted).
  **PS-17 (minor P3, defensive) — RESOLVED 2026-06-13:** the tree/ancestor-config snapshot read
  previously pulled each matched file WHOLE into memory (a 200MB adversarial config → ~400MB peak).
  CLOSED in `checks/run.rs`: all four snapshot read sites (the tree-axis `collect_files` + the three
  `ancestor_config_digest` reads, incl. `$CARGO_HOME/config[.toml]`) now route through
  `read_snapshot_content`, which folds a file `≤ 64 MiB` as its raw bytes (BYTE-IDENTICAL to the old
  read — the memo key and hit-rate are UNCHANGED for every realistic input) and a file `> 64 MiB` as a
  bounded `OVERSIZE:<len>:<streamed-sha256>` sentinel computed with a 1 MiB streaming buffer. Soundness
  is preserved (a change to an oversized file changes its length or hash → the sentinel changes → a
  MISS that re-executes — no stale green), peak memory is bounded, and an I/O fault still returns
  "absent" (fail-safe, exactly as the prior `fs::read(..).ok()`). Unit test
  `read_snapshot_content_caps_oversized_files_soundly` exercises both branches + determinism +
  soundness + the missing-file fail-safe with a tiny cap (no >64 MiB fixture materialized).
- **Hash-chain tail-truncation (PS-8 instance, property-confirmed).** Wave O's `verify_chain` proptest
  precisely bounded the LOCAL guarantee: any MID-STREAM corruption (byte-flip / drop / reorder /
  duplicate) is detected, but dropping the TAIL record leaves a still-valid prefix that `verify_chain`
  accepts — tail-truncation needs a published length/head-hash anchor, which is the **PS-8 P2
  server-side keyed seam**. Not a code defect; the property was constrained to mid-stream (the chain's
  actual promise) and documented. This is now a regression-guarding property test, not just prose.
- **Property/fuzz coverage added (T-8 closed):** `proptest` suites now guard the scrubber
  (`hugit-ledger`), the hash chain (`hugit-refstore`), and the memo key (`hugit-checks`) over
  generated inputs — continuous fuzzing of the security spine, not hand-picked fixtures.
- **Test-quality (T-1/T-3/T-5/T-6) + the PS-13 invariant (now walks `src/**`)** hardened in Wave O / O-2.

---

## PS-18 — `hugit-serve` /v1 backend: P2-gated + Wave-2 seams (disclosed, not faked)

**Source:** the githugr `/v1` HTTP backend (PR #111, 2026-06-13). The read-path serves REAL
engine data + documented honest defaults; these are the disclosed gaps so nothing is silently
shipped as complete.

- **Real auth (P2/identity):** `hugit-serve` uses a Bearer **dev-token stub**
  (`HUGIT_ENGINE_DEV_TOKEN`, length-invariant constant-time compare, fail-closed). The real
  ADR-0002 Clerk-JWKS / RFC-8693 validation is the **P2 identity seam** (needs the CoreLink
  tenant) — same family as PS-2. No code path fakes auth; the stub is honest + isolated in
  `auth.rs`.
- **Data source (deploy decision):** `state.rs` reads logs from a local dir
  (`HUGIT_SERVE_LOG_DIR/<repo>.json`). For a real Cloudflare deploy the source is **R2**
  (a bounded `state.rs` change — an R2/S3 read source, not yet built; build on request) or the
  **CoreLink CAS** (P2). Tracked in `docs/handoff/2026-06-13-hugit-serve-deploy-handoff.md`.
- **Structurally-sparse fields (no local seam):** `main_green`/`main_status`, diffstat
  (file_count/added/removed/diff), file tree (no git-tree API), mirror, the union-oracle,
  list_groups, GitHub-mirror About fields, fleet KPIs — all served as honest defaults; several
  need P2 (live AC, CoreLink) or new engine seams (a git-tree reader, a diffstat projector, a
  local main-CI status). Field-by-field map: `docs/plan/2026-06-13-hugit-serve-wave1-master-plan.md` §0/§5.
- **Wave-2 (deferred):** the SSE event stream (§2) + all write verbs (§3). Wave-1 is the 5 reads.
- **`tiny_http` hardening (deployment):** no built-in body-size/slowloris/connection limits;
  **localhost-until-fronted** by the edge proxy (Cloudflare terminates TLS + should enforce
  limits) before any public bind.
- **503-vs-404 existence signal (accepted trade-off):** a corrupt/tampered EXISTING repo → 503
  (fail-honest, never a fake-empty VM) while an absent repo → 404 — this distinguishes
  exists-corrupt from absent (a narrow, documented info signal). Accepted: fail-honest beats
  hiding a corrupt-data fault; owner may revisit if the no-existence-leak rule is read strictly.

**Owner/seam:** P2 CoreLink tenant unblocks real auth + the CoreLink data source + live KPIs;
the R2 source + the new sparse-field engine seams are buildable on request; deploy is the
githugr-TL + owner lane.

---

## Closed seams (reference — do not re-open without owner approval)

| Seam | Shipped | Governing commit |
|---|---|---|
| **PS-6 — Queue batch-verdict → queue-show projection** — `queue show` carried `verdict:null`/`implicated_pr:null` always; the recorded verdicts flowed to `campaign show` but not to the queue projection. | 2026-06-15 | branch `fix/pending-seams-ps6-ps10-ps15`; `queue::show` now folds the shared `hugit_ledger::Ledger` per-intent (proven, rejected) into a union verdict + blame; tests `acceptance_wb2::queue_show_projects_real_union_verdict_and_blame` + `…_verdict_is_null_until_a_verdict_covers_the_batch`. |
| **PS-10 — AC write-boundary axis guard only on the CLI `FileAc`** — the shared `ActionCache::store` contract was weaker than its CLI impl; `InMemoryAc`/`HttpAcClient` had no axis guard. | 2026-06-15 | branch `fix/pending-seams-ps6-ps10-ps15`; `pub fn guard_axes_not_secret` hoisted to `hugit_checks::client::ac` (called by all 3 `store` impls), predicate byte-equivalent to the prior CLI scrub (`!is_safe_identifier_shape`); `hugit-checks`→`hugit-ledger` path-dep, 0 new lock versions. |
| **PS-15 F-2 — error JSON `kind` not first** — `serde_json`'s BTreeMap-backed object sorted keys alphabetically (`fix`/`kind`/`message`), so agents stream-matching on `kind` found it second. | 2026-06-15 | branch `fix/pending-seams-ps6-ps10-ps15`; shared `porcelain::ordered_error_object` emits `kind`→`message`→`fix`→context in explicit order (zero new dep); both porcelain-error twins route through it. |
| **`cas:` exemption secret leak** — any `cas:`-prefixed value in a digest field was blanket-exempted from the detector, so `verdict --tree-hash "cas:ghp_…"` stored a PAT verbatim in the forever-log (pre-existing since WH-SCRUB). Round-7 finding #1. | 2026-06-12 (K-SCRUB) | merge `def8a18`; value-gate the `cas:` exemption (run the ONE detector on the payload) + `--tree-hash` door + matrix (`verdict_tree_hash_cas_credential_does_not_survive`). |
| **`hugit why` / `hugit export` skipped `verify_chain`** — two read paths projected a tampered log as authoritative provenance; PS-8 AC4 was momentarily false. Round-7 findings #2/#5. | 2026-06-12 (K-CHAIN) | merge `def8a18`; both now verify the chain → `chain_broken` exit 2 (`tampered_chain_is_chain_broken_exit_two_on_{why,export}`). |
| **Verdict rejection-laundering by lens substitution** — `latest-record-wins` let an approve under a novel `--lens` name erase a prior reject from the projection. Round-7 finding #3. | 2026-06-12 (K-VERDICT) | merge `def8a18`; **reject-sticky** resolution (a reject clears only on a SAME-lens re-approval) + post-seal append guard (`campaign_sealed` exit 2). Owner-decided rule. |
| **Wedge stale-green** — `env_manifest` hardcoded empty, so a result-changing env/`RUSTFLAGS` change did not bust the memo key (missed-miss). Round-7 finding #4. | 2026-06-12 (K-RUN) | merge `def8a18`; allowlist of result-affecting env folded into the key. Residual (custom ad-hoc vars) tracked as PS-11. |
| **`ac_busy`→`ac_error` taxonomy collapse (flaky gate)** — the retryable lock-exhaustion was flattened to terminal `ac_error`, making `concurrent_checks…` non-deterministic under `--workspace` load. Round-7 finding #6. | 2026-06-12 (K-RUN) | merge `def8a18`; busy path preserves retryable `ac_busy`/`log_busy` kind. Stress-verified 10/10 + full workspace run. |
| **`pr` I/O fault bare error + `intent new` non-atomic commit** — `pr open/land/abandon` I/O faults emitted bare stderr exit 1 (not structured exit 2); `intent new` saved `--store` before the `--log` append (divergence on log failure). Round-7 findings #7/#8. | 2026-06-12 (K-ERRLAW2) | merge `def8a18`; structured `io_error` exit 2 + log-gated store commit (no orphan, retry converges). |
| **`.ac` Action-Cache toolchain-digest secret leak** — secret-shaped `--toolchain`/`--def` persisted verbatim into `<log>.ac` (bypassing the `--log` WG-SCRUB seam). FOUND by the WJ-INT per-verb secret matrix; CLOSED by WK-AC (door reject + `FileAc` write-boundary guard). | 2026-06-12 (WK-AC) | merge `d04e199`; tests `check_toolchain_secret_rejected_at_door_and_never_in_ac`, `write_boundary_guard_refuses_a_secret_toolchain_axis`. Defense-in-depth completeness tracked as PS-10. |
| **Round-8 C1 — prefix-less secret leaks through identifier-address fields** — a credential with no known prefix (AWS/SendGrid/Stripe/32-char base64) rode any identifier field (`--id`/`--run-id`/`--principal`/`--campaign`/`--pr`) verbatim into the forever-log + local store; the scrub was a secret-PREFIX allowlist (open-by-default). The 6th "exemption is a hole" instance / the AR-5 reframe. | 2026-06-12 (Wave L / L-A, `integ/wave-l` — **pending merge to main**) | **Polarity inverted to DENY-BY-DEFAULT** (`porcelain::structural_secret_scrub`): an identifier value survives verbatim ONLY if it proves a bounded safe-address shape (`is_safe_identifier_shape`: ULID, sha-hex, `cas:`/digest, low-entropy slug, integer), else the door rejects (`secret_in_identifier` exit 2) or the payload boundary redacts. Cold-verified: AWS key in `--id`→exit 2, in `--charter`→`[REDACTED]`; ULID survives. Matrix `acceptance_wj_matrix` extended with prefix-less specimens. |
| **Round-8 C2 — `hugit intent list --log` skipped `verify_chain`** — a third read path projected a tampered log as authoritative landed-state. | 2026-06-12 (Wave L / L-B, `integ/wave-l` — **pending merge**) | `resolve_landed` now verifies the chain → `chain_broken` exit 2 (`acceptance_round8_readpath`). Structural residual (per-loader, not one chokepoint) tracked as **PS-13**. |
| **Round-8 C3 — wedge stale-green from cwd / arbitrary env / PATH** (+ **Round-9: stdin**) — the memo key tried to ENUMERATE the inputs of a non-hermetic `sh -c`; cwd, an unlisted env var, PATH, and (found in the R9 re-audit) **stdin** were uncaptured → cached PASS where a real run FAILs. | 2026-06-12 (Wave L / L-C + R9-C3, `integ/wave-l` — **pending merge**) | **Hermetic execution** in `ProcessRunner::run`: `env_clear()` + captured allowlist only, `cwd` pinned to `--root`, PATH pinned + hashed into the env axis, **stdin nulled** (`Stdio::null()`). cwd/env/PATH/stdin changes now MISS or are deterministic (cold-verified; test `stdin_is_nulled_not_inherited`). FS/network/clock = disclosed P2 runner-sandbox seam. Supersedes the PS-11 allowlist residual. |
| **Round-8 C5-F1 — within-record lens-substitution launders a sticky reject** — `verdict --lens X reject --lens X approve` in ONE call projected `proven:1 rejected:0` (the ledger fold was last-wins within a record; K-VERDICT only fixed cross-record). | 2026-06-12 (Wave L / L-D, `integ/wave-l` — **pending merge**) | The fold is reject-sticky WITHIN a record (`merge_lens_outcome`); the recorder refuses a conflicting duplicate-lens input (`duplicate_lens` exit 2). Cold-verified; legit single reject / multi-lens / cross-record clear all intact. |
| **Round-8 C5-F2 — a SEALED campaign was not terminal** — the post-seal guard was point-local to `verdict`; `intent new`/`pr open|land|settle|abandon` still appended into a closed campaign. | 2026-06-12 (Wave L / L-D, `integ/wave-l` — **pending merge**) | A single shared seal guard enforced at the `hugit-refstore` append chokepoint, so ALL campaign-scoped verbs inherit it → `campaign_sealed` exit 2. Cold-verified across intent + pr; `done` stays put. Compound: clean-seal over a reject now requires `--allow-rejected`. |
| **Round-8 C4 — the raw `EventLog::append` door was workspace-`pub`** — "every verb routes through `append_authorized`" was enforced by prose, not types (latent; Round-7 export bug was this door). | 2026-06-12 (Wave L / L-D, `integ/wave-l` — **pending merge**) | `append` demoted to `pub(crate)`; cross-crate users routed through `append_authorized` or a typed closed-enum `append_external_change(ExternalChangeKind)` shim; test-only raw access is the `#[cfg(feature="test-support")]` `append_for_test`. A forged cross-crate raw `pr.opened` no longer compiles. |
| **Round-8 C6 — `clap` arg errors bypassed the error envelope** — malformed invocations emitted bare `error:` stderr (no `{kind,fix}`); retryability was a string convention (`starts_with("ac_busy:")`). | 2026-06-12 (Wave L / L-C, `integ/wave-l` — **pending merge**) | `Cli::try_parse()` renders the structured `invalid_arguments` envelope exit 2; retryability is the TYPED `AcError::Busy` matched exhaustively in `map_exec_error` (string-sniff deleted). HTTP 429/503 busy path is typed but exercised only at P2 (local `.ac` is the live path). |
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
