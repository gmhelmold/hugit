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

## PS-9 — Intent store vs. log: `intent show`/`intent list` read a cwd-global index (owner decision pending)

**Status:** TRACKED — design decision pending owner input; DO NOT fix the code until the owner decides  
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

## PS-10 — AC write-boundary axis guard lives only on the CLI `FileAc` (defense-in-depth completeness)

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

## PS-11 — Check `env_manifest` captures only an allowlist; ad-hoc `--cmd` custom env vars are not keyed

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

## PS-12 — Self-hosted runner is missing `cargo-deny` / `cargo-audit` (CI gate cannot run the advisory check)

**Source:** Round-7 — CI run 27422619640 (the Wave J + WK-AC push) concluded FAILURE; the
failing step was `gates/deny` exit **127** (`cargo-deny` binary not found on the self-hosted
runner). Locally `cargo deny check` passes (exit 0) and covers the RustSec advisory DB; the
standalone `cargo-audit` is also absent locally. So the advisory gate runs LOCALLY but the
self-hosted runner cannot run it — a green LOCAL gate is not reproduced on CI for that step.

**Owner:** infra (self-hosted runner provisioning).  
**Acceptance criteria:** install `cargo-deny` (and `cargo-audit`) on the `corelink-builder`
runner image so the `gates` job runs the full advisory check; until then the advisory gate is
LOCAL-verified only, and a `deny`-step CI red of exit 127 is a known infra gap, NOT a code or
dependency failure. (Distinct from the test/clippy gates, which DO run on the runner.)

---

## PS-13 — Read-path `verify_chain` is enforced per-loader, not at one chokepoint (defence-in-depth)

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
and the seal guard (L-D / C5-F2)). **Owner/lead decision pending** — schedule as a follow-up WP
(small, mechanical: converge five loaders onto one function) once Round 9 confirms the live holes
are closed.

---

## PS-14 — Deny-by-default identifier scrub: the safe-shape allowlist exempts ANY-length hex / numeric / low-per-char-entropy values (OWNER TUNING DECISION)

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

**Owner decision pending — options:**
1. **Accept + document** the wider survivor band honestly (a hex/numeric value indistinguishable
   from an address is irreducible physics); keep the generous allowlist (no over-scrub).
2. **Tighten** the hex exemption to known digest lengths {40,64} (+ a numeric-length cap), accept
   the over-scrub risk on non-standard-length legit hex ids, and add a low-entropy specimen to the
   `acceptance_wj_matrix` guard.
3. **Hybrid** — length-pin hex to {40,64} but keep integers/short slugs generous (covers the
   common legit-id cases while catching odd-length hex/long-numeric secrets).

Also tracked: the guard matrix (`acceptance_wj_matrix`) lacks a low-entropy/odd-length-hex
specimen (R9-2), and the two scrub engines (`porcelain` / `redact.rs`) remain hand-kept-in-lockstep
(R9-3, P2) — both fold into whichever option is chosen.

---

## Closed seams (reference — do not re-open without owner approval)

| Seam | Shipped | Governing commit |
|---|---|---|
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
