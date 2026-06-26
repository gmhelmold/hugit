# Honest delivery audit (double-checked) — hugit vs the whitepaper, 2026-06-17

> **PARTIALLY SUPERSEDED 2026-06-22** — several "NOT live" gaps below have since shipped:
> the engine is now MULTI-REPO and deployed serving `hugit` + `githugr` (`/readyz git_repos:2`,
> F6a); the CoreLink **AC memoization is LIVE** (#182); the product-refinement (Phase 1+2, #179–181)
> and git-ingest/CAS hardening (#184) merged. `ctx resume`/`review`/`land queue` are REAL. **git
> `push`/receive-pack is now LIVE** (#198, 2026-06-26 — git-free gix-pack unpack on the distroless
> engine). **Caveat (a) "one push per ref per engine lifetime" is now CLOSED** by the live ref
> hot-swap (#201, deployed + verified — a pushed ref is advertised immediately, no reboot). Remaining
> push caveats: v0 = self-contained packs (an *incremental* push on server-side history is rejected
> fail-closed — thin-pack/CAS-base reachability is the tracked follow-up), and clone-back is gated on
> the public-flag. STILL open: anonymous-clone public-flag, live runner exec,
> multi-tenant, and the killer-data **render** is verified only to "route serves (401)" from here —
> the githugr TL's authed www smoke is pending. **The "~15–20% live" figure in the body below is
> superseded: with read+WRITE (git push) now live, CLAUDE.md's current estimate is ~25–30% of a
> single-tenant forge (still single-digit % for a full multi-tenant forge).** See CLAUDE.md's
> **Update 2026-06-22 / 2026-06-26** blocks for the current honest state; the per-capability analysis
> below remains the deep reference for everything not touched this round.

> Grounded in the whitepaper (`docs/whitepaper/hugit-v1.md`), the 67-WP
> decomposition (`docs/plan/decomposition.md`), and a 2-fleet code sweep
> (44 read-only agents: scoping + per-area completeness double-check). This
> CORRECTS the maximalist "67/67 built · complete · converged" framing in
> CLAUDE.md, which conflated *PR-merged* and *test-green-hermetically* with
> *delivered live*. Macro verdicts confirmed; coverage gaps + over-harsh
> mis-classifications corrected.

## The one-line truth

We built the **brain** (engine logic + platform invariants, hermetically tested,
real crypto) and a **narrow live slice of the body** (the deployed `/v1` read+write
API against the single `hugit` launch repo, R2-persisted, CAS-versioned). The
**full forge** — git hosting, real CI on runners, the CoreLink substrate,
merge-as-re-execution, the live UI, multi-tenant identity — is **not delivered**.

## Macro verdicts (confirmed by every area sweep)

- **No live git wire protocol.** `hugit-proto` has a full gix-based clone/fetch/push
  implementation, but `hugit-serve` binds the only listener and routes ONLY `/v1/*`
  + `/readyz` — no `/info/refs`, no `git-upload-pack`/`receive-pack`. **`git clone`
  against the deployed engine returns 404.**
- **Merge-as-re-execution is an intentional P2 deferral** (`write_dispatch` records
  the demand + draft PR, never auto-spawns — a documented lead decision). The regen
  gate (D12) + experiment gate (D8) logic is real + hermetic; the live runner seam is P2.
- **CoreLink substrate is P2-infra-gated** (hot CAS / AC) or **external-transferred**
  (runner fabric → corelink-runners @ b6319a3, with acceptance suites intact). The AC
  client (`hugit-checks/src/client/ac.rs`) has a REAL ureq transport + `from_runtime()`
  loader — plug-and-play, one PAT from live; the in-memory AC is a test double only.
- **Identity is a dev-token stub live**, with the full Clerk→engine-token exchange
  (`token.rs`, Option-B) code-complete; the CoreLink exchange endpoint
  (`corelink-api.humangr.com/v1/session/exchange`) is **already live** (401 on probe).
  Gap: `HUGIT_SESSION_EXCHANGE_URL` absent in the deployed container + stale image.

## Corrections (where the first-pass audit was TOO HARSH)

| Item | First-pass (wrong) | Corrected (true) |
|---|---|---|
| `hugit check` / `hugit verdict` | stub-ish | **real EXECUTE paths** (hermetic env, 3-axis memo key, file-backed AC, `check.recorded`; verdict diversity + reject-stickiness + post-seal guard) |
| `/v1` write path | "not a live capability" | **code-complete, live-R2-CAS-proven** against production; gap is a stale deployed image, not code |
| R2 data plane | "static recorded snapshot" | **live mutable CAS** — POST verbs append via If-Match compare-and-swap; live round-trip test ran against prod R2 |
| E6 bidirectional mirror | "gate-bound deferral" | **built + acceptance-tested** (forge-arbitrated convergence proof); only live GitHub detect is the seam |
| C5b secrets broker | "mocked/transferred" | **production-quality retained** (HMAC-SHA256, traversal guard, fail-closed); only C5a transferred |
| B2a AC client | "behind a mock transport" | **real ureq transport**, seam-ready |
| E5 export / exit-proof | (bucketed hermetic) | **real-live** — runs a real `git` binary with constrained PATH, zero hugit dep; the strongest delivered guarantee |
| Per-tenant authz gate | (omitted) | **real-live** on every `/v1` request (404 denial, no existence oracle) — the one deployed security boundary |
| 11 `/v1` reads | "partially fixture/data-gated" | **serve real chain-verified R2 data** for the `hugit` repo |

## Omissions (whole layers the first-pass audit FORGOT)

- **Squad X — 14 platform invariants** (`hugit-invariants`): tenant isolation (HMAC
  partition), attestation (Ed25519), context privacy + training-exclusion, namespace
  laws, non-interference, erasure cascade (X7/X12), self-release attestation (Rekor
  seam), cross-phase identity, focus gate, degradation composition, legibility,
  deep-link integrity. **Mostly hermetic-REAL with real crypto**; X6/X8/X10/X11 have
  P2-gated live lanes. The most auditable evidence of discipline — entirely omitted.
- **Squad B — 10 WPs**: GitHub App skeleton (HMAC/webhook/check-runs), affected-targets,
  union queue + GitHub merge JWT, bisect/diagnosis, intent sidecar, status page + cost
  model, dogfood harness (5-PR wave + 48h soak seam), **the money gate (all 6 items,
  fail-closed)**, negative-scope compile-time proofs. All hermetic, none live.
- **The §9 security model** as a named layer (5 locks + tenant boundary).
- **6 retained Squad C WPs**: C4 (regen DriverRegistry), C5b (broker), C6 (flake stats),
  C7 (budgets / weighted-fair-queue), C8 (shadow checks), C10 (no-shock guard).
- **The CLI machine-UX contract**: PorcelainError, one-error/one-exit-code law,
  structural scrub-on-append redaction, `HUGIT_VERBS` registry. Plus the reserved-verb
  census — **reserved-but-unimplemented**: approve, reject, undo, policy, ws, ctx,
  dispatch, fleet, journal, diag; **not even reserved**: ask, log, ws spawn.
- **Object-model depth**: ContextEnvelope (ADR-0001 four-altitude redesign),
  TrajectoryRecorder, Journal/ctx-resume (BeyondHorizon documented refusal), the
  cold-store seam (`UnwiredColdStore` — a P2 gap DISTINCT from the hot CAS: transcript
  blobs persist in no live deployment), FenceManifest.
- **D8** (experiment harness, fail-closed gate binding), **D13** (tournament — a LIVE
  dispatched verb), **E3** (status/badge compat — fully acceptance-tested).
- **Semantic index — ABSENT** (not even a WP; whitepaper aspiration). **jj** —
  hermetic logic, served nowhere.

## Corrected high-level status

| Capability | Status |
|---|---|
| Git wire protocol (clone/fetch/push) | hermetic-logic-only — **served nowhere; clone → 404** |
| Merge-as-re-execution / agent dispatch | intentional P2 deferral (records demand, no auto-spawn) |
| CoreLink hot CAS / AC | p2-infra-gated (real ureq client, plug-and-play; needs P2 tenant) |
| CoreLink cold-store (trajectory/envelope blobs) | p2-infra-gated, **distinct seam** (`UnwiredColdStore`) |
| Runner fabric (exec, warm boot, fences) | external-transferred → corelink-runners (suites intact) |
| GitHub App (Squad B) | hermetic-logic-only (no Worker binding, no App registration) |
| `/v1` read surface | partially-live — 11/20 reads serve real R2 data for `hugit`; ~9 git/identity reads honest-default fixture; **deployed image stale** |
| `/v1` write surface | code-complete, live-R2-proven, **stale-image deploy gap** |
| SSE event stream | live replay-then-close; hold-open live-tail is P2 |
| Auth / identity | dev-token stub live; Clerk exchange code-complete, P2-gated (env+image) |
| Per-tenant authz gate | **real-live** (the one deployed security boundary) |
| §9 security locks | hermetic-logic + one transferred half (C5a); audit trail not persisted to EventLog |
| Squad X invariants | hermetic-REAL (real crypto) + P2-gated live lanes |
| CLI verb surface | 13+ verbs **real-dispatch**, hermetic backends; 10+ verbs reserved-unimplemented |
| Mirror (E1/E2/E3/E6) | hermetic-logic-only; live GitHub token exchange is the single E-wide seam |
| Object model | hermetic-logic-only — substantially more built than first conveyed |
| Data plane (R2) | **live-mutable CAS** (If-Match CAS writes; prod round-trip tested) |
| Economics / money gate | hermetic-logic-only; cross-tenant dedup = post-GA, zero code |
| Semantic index | **absent** |

## Honest magnitude

- **~15–20% live** for the deployed `/v1` read+write API against the `hugit` launch repo.
- **single-digit %** for a full multi-tenant end-to-end forge experience.
- The hermetic CLI layer + the standing Squad X invariants are substantially more
  built engineering than a naive "single-digit %" conveys — but **none of the
  whitepaper's headline forge experience runs live for a second tenant or a real
  pushed repo.**

## What "100% delivered, reviewed, audited" actually requires (biggest → smallest)

1. **CoreLink P2 tenant** (hot CAS + AC live) — owner/infra-gated. Unblocks memoized
   checks, real storage, the economic thesis.
2. **Runner fabric live** (corelink-runners on a real box) — real CI execution, fences,
   merge-as-re-execution dispatch.
3. **Live git wire protocol serving** — bind `hugit-proto`'s clone/fetch/push to a
   network endpoint (the CAS/file-content seam). Today: `git clone` → 404.
4. **GitHub App registration + live mirror/import/status** (Squad B + E1/E2/E3 live).
5. **Multi-tenant Clerk identity** — set `HUGIT_SESSION_EXCHANGE_URL`, rebuild the
   deployed image, `hugit-prod-d1` token store, live JWKS.
6. **Deploy the current `main` image** — the single cheapest unblock; the stale image
   404s the newly-shipped reads and would break per-session writes if rebuilt without
   the exchange URL (the coupling already flagged to githugr).
7. **The data-model waves**: file-content/blob store (blob/edit/symbol), search index,
   knowledge BYOK, identity reads.
8. **The reserved-verb gap**: implement approve/reject/undo/policy/ws/ctx/dispatch/
   fleet/journal/diag/ask/log; the Front-4B issue verbs (ADR-0006 CLI parity).
9. **The live human experience**: Ledger / Mission Control / attention queue / review
   interrogation as a used UI, not library logic.
10. **The disclosed-seam closures**: cold-store binding, Rekor transparency log, fabric
    attestation pubkey + transport, B5 auto-trigger, B8 48h soak, X6/X8/X10/X11 live lanes.

Most of 1–6 are owner/infra-gated, not "a few PRs". The honest critical path to a
usable single-tenant forge is: **deploy current main → CoreLink P2 tenant → live git
serving → runner fabric**.
