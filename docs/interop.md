# hugit interop — how this repo talks to the family (microscopic seam map)

> 2026-06-09. Every seam hugit speaks, with the exact types, transports, keys,
> env gates, and failure semantics. Sources of truth: `crates/hugit-contracts`
> (the IDL — Rust + JSON Schema + golden serde, frozen),
> `../corelink-runners/docs/spec/hugit-integration-contract.md` (the fabric
> seam, frozen from hugit's side; **v1.2.0** as of 2026-06-11 — §13 envelope
> emission (WP-R6, 2026-06-10) + §13.1 `cost_usd_micros` money amendment (WA4,
> 2026-06-11); the frozen v1.0 §0–§12 are otherwise unchanged),
> `docs/adr/0001` (context envelope),
> `docs/adr/0002` (identity), `docs/handoff/2026-06-08-p2-go-live-runbook.md`
> (what flips live). Production-state claims cite source repos — never memory.
>
> *Updated 2026-06-11 (post runner-transfer + ADR-0001 ratification):* §2
> updated to reflect that `hugit-runner` transferred to `corelink-runners`
> (WP-R4, 2026-06-10) and transcript blobs are retained forever by ratified
> design (erasure via explicit tombstone only — no TTL).
>
> *Updated 2026-06-11 (WG-DOCS):* §2 runner-contract version advanced to
> **v1.2.0** (WA4 `cost_usd_micros` u64 amendment applied; v1.1 was stale).

```
            githugr (campaign #4, design)  ──reads──▶  hugit (THIS REPO, built)
                                                          │ consumes (never forks)
   GitHub ◀──mirror/App/shim──  hugit  ──AC/CAS──▶  CoreLink Cache   (live, GA staged)
                                  │────exec────▶  CoreLink Runners  (spec; interim box racked)
                                  │────concepts─▶  CoreLink Workspaces (clw phase 1)
                                  └────attest───▶  transparency log (Rekor-class; P2)
```

## 1. hugit → CoreLink Cache (AC + CAS)

| Aspect | Exact detail |
|---|---|
| Client | `hugit-checks` (B2a) — AC HTTP client; CAS access for envelopes/packs |
| Memo key | `key = H(tree_root ‖ check_def_digest ‖ toolchain_digest)` — `hugit-checks::compute_memo_key` (three axes; changing any axis is a new key) |
| Stored value | `CheckResult` bytes (canonical; byte-identical across runners or the memo is poisoned) |
| Transport | HTTPS, `Authorization: Bearer <PAT>` |
| PAT location | `~/.hugit/secrets/corelink/pat` (single-read; the secret-read-guard flags repeat reads — read once, hold in memory) |
| Unconfigured behavior | client returns `NotConfigured` — **fail-closed**: never fabricates a hit, never silently executes-and-pretends |
| Live smoke (P2 DoD) | `corelink_ac_smoke.rs`, 3 probes: miss→404 · put/get round-trip hit · cross-tenant→**403** |
| CAS objects stored | context envelopes + transcript blobs (`cas:` refs, ADR-0001; deduped by content, redacted on write, **retained forever** — ADR-0001 §3 ratified 2026-06-10: no TTL/GC, erasure via tombstone only) · pack objects (D2 wire protocol assembles packs from CAS) · export bundles (E5) |
| Tenancy | one CoreLink tenant for hugit (P2 request: tenant + AC namespace + CAS/R2 + PAT — `docs/handoff/2026-06-08-corelink-p2-tenant-request.md`) |
| Non-interference bound | X6/X10: hugit load must not move other tenants' latency; caps set at the fabric/tenant level **before** load (preventive, not reactive) |

## 2. hugit → Runners (check execution) — contract frozen from our side

Canonical: `../corelink-runners/docs/spec/hugit-integration-contract.md`
(**contract v1.2.0** — §13 envelope emission obligation added 2026-06-10 by
WP-R6; §13.1 `cost_usd_micros|u64` money amendment added 2026-06-11 per WA4;
the runner must emit per-job metrics consistent with `IntentMetrics`
and expose capture hook points for the two-transcript imperative).
`hugit-runner` **transferred to `corelink-runners`** (WP-R4, 2026-06-10) —
the runner execution core (lease, isolation, warm boot, C2/C3/C5 suites)
now lives in `../corelink-runners/crates/corelink-runner`; the seam is the
**wire contract** (`conformance/` vectors byte-identical in both repos).
hugit retains: the consumer seam (`hugit-checks`), the envelope producer
(`hugit-ledger::envelope`), and the invariant proofs (against conformance
fixtures). IDL types (golden-pinned in `hugit-contracts`): `CheckDef`,
`CheckResult`, `RunnerLease`, `FenceManifest`, `QueueApi`, `AttestationChain`.

| Aspect | Exact detail |
|---|---|
| Lease request | `{tenant, size (vCPU/mem), image (sha256-pinned digest), ttl, claim (FenceManifest)}` → `RunnerLease {lease_id, exec endpoint, deadline}` |
| Lease states | `Pending → Held → (Released | Expired | Crashed)` — the fabric may not invent intermediate authoritative states |
| One lease = one job | fresh microVM per lease; no dirty-runner reuse; box destroyed after |
| Expiry semantics | at `ttl` the job is killed, lease `Expired`; **a half-finished check never produces a stored result** (partial-memoization is a correctness disaster) |
| Crash semantics | lease `Crashed`; hugit retries on a new lease; no duplicate/partial result emitted |
| Determinism | same `CheckDef` over same inputs ⇒ **byte-identical** `CheckResult` (same content digest), any runner, any time. hugit detects non-determinism by re-running (differs after **3** runs ⇒ flagged, never memoized). The fabric must not inject per-boot values |
| Fence | `FenceManifest` = the only readable/writable paths; enforced runner-side; covered attacks: absolute-path injection, `..` escape, `srcfoo` vs `src/` prefix-collision |
| Secrets | C5b broker: credential never on box image/disk/argv; scan attestation `env=0, proc=0, disk=0`; **fail-closed on unparseable scan** |
| Attestation | signed `{image digest, resolved inputs, result hash}` per execution → folds into `AttestationChain`/X8; **unattested result = rejected** |
| Supply chain | images pinned `sha256:`, **verify-before-spawn** (X4); deps enter via CAS, never ad-hoc network fetch inside the job |
| Trigger | `QueueApi`: the landing queue (B4) triggers execution for uncached checks; auto-bisect (B5) on red |
| Transport TODAY (interim) | SSH → `hugit-runner-01` (Hetzner), `docker run`; `StrictHostKeyChecking=accept-new` + pinned known-hosts; env `HUGIT_RUNNER_HOST` |
| Transport TARGET (M1) | the fabric's authenticated API (Bearer PAT, same as CAS/AC); logical contract identical |
| Acceptance (run-not-skip when env set) | B2b byte-identity · C3 warm<cold + cache-down fail-closed · C2a/C2b/C9 lease/crash/expiry/ws-lifecycle · C5b secrets red-team · X11 mid-op broker fault · X6/X10 non-interference |

## 3. hugit ↔ GitHub (App, mirror, sync, shim)

| Component | Seam |
|---|---|
| `hugit-app` (B1) | GitHub App: install/permissions, webhook ingest, Checks API client, status badges; uninstall = real stateful revoke; empty webhook secret = rejected |
| `hugit-mirror` (E1/E2) | verified one-way mirror out (content-hash convergence: identical tips ⇒ mirror silent) + one-command import (public/private, LFS); divergence is never silently dropped |
| bidir-sync (supersedes E6) | branches round-trip: GitHub-side change → **external-change event** (D3⑤: opaque, attributed, never a synthetic intent) → mirrored back. **`main` is single-writer by construction**: only the landing queue (B4) advances it; a direct GitHub push to `main` is rerouted (rejected-with-guidance / auto-proposed branch) |
| Incident refs | same-branch divergence: forge tip wins; GitHub tip preserved at `refs/hugit/incidents/…`, hash-chain-verifiable, recoverable |
| `E4` Actions shim | executes the supported subset of `.github/workflows` (`docs/shim/supported-subset.md`); out-of-contract is explicit, never silent |
| Live gate | `HUGIT_GH_TEST_REPO` (webhook/poll detection on a real repo) — P2 seam |

## 4. hugit → transparency log (X8)

ed25519 self-release attestation published to a Rekor-class public log; boot
verifies inclusion. Trait-mocked hermetically; live publication is a P2 seam.

## 5. hugit → githugr (serving the surface)

- **Today:** githugr's `Provider` reads hugit's crates as libraries —
  `hugit-refstore` (intents/refs/event-log) · `hugit-proto` (objects/diffs) ·
  `hugit-checks` (CheckResult, hit-rate) · `hugit-queue` (batches/lanes) ·
  `hugit-ledger` (journals/fleet) · `hugit-diag` (why/impact/bisect) ·
  `hugit-mirror` (sync state) · `hugit-policy` (gates) · `hugit-contracts`
  (the types, incl. `ContextEnvelope`, `AttentionRank`, `AttestationChain`).
- **At P2:** the per-repo Durable Object event-log + CoreLink CAS bind live;
  same trait, screens cannot tell.
- **Write path (githugr, later):** every mutation is an event through policy
  gates + the single-writer queue; **D14 authz**: PR author ∈ {orchestrator,
  human}, campaign owner = human — a subagent can never author either.
- **Parity law:** `hugit why|impact|export` (CLI, D10) and githugr's surfaces
  read the same store and must give identical answers.

## 6. Identity (ADR-0002 — `docs/adr/0002-hugr-identity.md`)

Machine path (this repo): PAT, unchanged. Human path (githugr): HuGR account →
Clerk session → server-side exchange → short-lived tenant-scoped token. **A PAT
never reaches a browser.** `operator` fields in envelopes/attestations carry the
HuGR account principal.

## 7. hugit ↔ Workspaces (clw) — honest current state

Today: **independent implementations over the same CAS concepts** — hugit-fence
materializes claim-fenced workspaces for its own runners; `clw`
snapshots/hydrates workspaces against the live API. No runtime call between
them yet. Convergence is roadmap: agent sandboxes / dev boxes ship as Workspace
SKUs on the Runners fabric (M4), and githugr's "open in workspace" uses
`clw hydrate`.

## 8. Env-gate table (the live seams; fail-not-skip when set)

| Gate | Flips |
|---|---|
| PAT at `~/.hugit/secrets/corelink/pat` + tenant | AC live (B2a smoke, B8 dogfood vs live, X6/X10 measured) |
| `HUGIT_RUNNER_HOST` | runner box execution (B2b, C3, X4-on-spawn, X11, C5b) |
| `HUGIT_GH_TEST_REPO` | live GitHub detect (bidir-sync, E1 to real repo) |
| transparency log endpoint | X8 live publication + boot verify |

> **`HUGIT_RUNNER_HOST` is an INTENTIONAL cross-product seam name, not a naming
> error (PS-4).** After `hugit-runner` transferred to `../corelink-runners` (the
> runner-transfer campaign), the env-var name stayed `HUGIT_RUNNER_HOST` ON
> PURPOSE: it anchors the hugit↔runner wire seam (the variable hugit reads to
> reach the leased runner box) and is frozen by the integration contract — it
> does NOT track the runner product's own crate name (`corelink-runner`). Do not
> "fix" it. (The matching cleanup — the doc-title "hugit-runner" inside the
> corelink-runners product docs — is the sibling-repo half of PS-4, owner-coordinated.)

> **Fleet-dispatch toolchain requirement (PS-7).** The `hugit check` toolchain
> memo axis falls back to the constant `toolchain-unprobed` when `--toolchain` is
> omitted AND `rustc` is unavailable (a sandboxed / rustc-less environment). Two
> distinct rustc-less environments would then share that constant → the same memo
> key → a potential cross-env false cache HIT. **Any fleet-dispatch / multi-env
> orchestration MUST pass `--toolchain <digest>` explicitly** so the toolchain
> axis is a real fingerprint, never the shared `toolchain-unprobed` constant.
> Single-env local runs (where `rustc` is probed) are unaffected.

## 9. Change protocol

`hugit-contracts` is golden-pinned: any shape change is a deliberate,
owner-gated event. The runners contract is frozen from hugit's side — the
fabric adapts or escalates to the owner; hugit does not silently adapt.
Cross-repo decisions are ADRs, canonical in this repo (`docs/adr/README.md`).
