# hugit Roadmap — post lazy-CAS, toward open-source + a demonstrable wedge

**Date:** 2026-06-20 · **Author:** hugit TL · **Basis:** 3-agent read-only study (delivery-state · product-vision · code-architecture) + live-probe baseline.
**Supersedes** the planning framing in `docs/plan/2026-06-17-delivery-roadmap-post-honest-audit.md` for sequencing (that audit's capability facts still hold except where noted below).

---

## 0. Honest baseline (live-verified 2026-06-20, corrects stale docs)

- **Engine LIVE, non-degraded:** `engine.githugr.com` runs main `0450e2c` (lazy CAS load, PR #170),
  `CAS_DISABLED=false`, `/readyz` 200 binding in ~5s **with git-from-CAS on**. CAS fully re-ingested
  (6862 objects). The 2026-06-20 outage is fully resolved. *(The study's delivery agent read pre-today
  docs and reported "degraded/CAS-off" — that is stale.)*
- **`/v1` read+write API live** for the `hugit` repo: ~31 read VMs real + chain-verified, 10 POST verbs
  real + R2-CAS-persisted, authz-gated (404-no-oracle), SSE replay. dev-token auth live.
- **Symbol outline IS wired** (W6/#161/#168 — blob `outline` calls `compute_outline`; `hugit symbol --file`
  real). The CLAUDE.md "`[]` default / not wired" line is STALE — fix in W0.
- **Integrity spine is SOTA + hermetic** (real Ed25519/SHA-256, 13 adversarial rounds): the strongest asset.
- **Built-but-DARK (the gap that matters most):** the wedge — union landing queue + memoized CI — is
  hermetic but unreachable/undemonstrable (no live AC infra; porcelain thin).
- **Auth-gated, not public:** anonymous `git clone` 404s by the read-visibility gate (correct). Open-source = a decision (W0).
- **Genuinely gated:** CoreLink P2 hot CAS+AC (owner/infra), runner fabric (corelink-runners), GitHub App + mirror (owner), multi-tenant Clerk + `hugit-prod-d1` (owner/infra).

## The bar (what "complete/SOTA" means — measure against this, not the campaign)
Per whitepaper §14: *a forge where conflicts surface before work begins, merges re-execute instead of
re-fighting, main is always green at fleet scale, and humans command through a ledger + attention queue
instead of drowning in diffs.* Today the **spine** is there; the **wedge is dark** and the **live stack** is single-tenant-thin.

---

## Sequencing principle
Order by **leverage per unblock**, and by **what hugit can ship without owner/infra gates** first. Three
buckets: (A) **fleet-buildable now** (code only, I orchestrate), (B) **owner/infra-gated** (I prep + file
the precise ask), (C) **cross-team** (corelink-runners / CoreLink P2). Waves interleave A heavily; B/C are surfaced early so the owner can unblock in parallel.

---

## WAVE 0 — Open-source go-live + honesty truing (SMALL, owner-greenlit, do first)
Goal: hugit becomes open-source cleanly, and the repo's own docs stop lying about state.
| WP | What | Bucket | Size | Disjoint files |
|----|------|--------|------|----------------|
| W0.1 | **Secrets/history sweep** — scan full git history (gitleaks-class) for any committed cred before going public. Report; if any hit, scrub before flip. | A (me, read-only first) | S | (scan only) |
| W0.2 | **LICENSE** — add Apache-2.0 (recommended) + SPDX headers policy; update `Cargo.toml` license fields. | A | S | `LICENSE`, `Cargo.toml`s |
| W0.3 | **Docs honesty true-up** — CLAUDE.md + audit doc to current reality (lazy engine live, symbols wired, re-ingest done, CAS on). | A | S | `CLAUDE.md`, `docs/review/*` |
| W0.4 | **Go public** — set GitHub repo public + (decision) `hugit meta set --visibility public` for anonymous clone. | B (owner flip) | S | n/a |
**Gate:** W0.1 PASS (no secrets) is a HARD precondition for W0.4. W0.1–W0.3 disjoint → parallelizable.

## WAVE 1 — Make the wedge visible + finish the serve surface (HIGH leverage, fleet-buildable)
Goal: the product thesis (union queue + memoized CI) becomes demonstrable through the porcelain; the
serve read surface is complete; the security-trust findings close. All code-only — I orchestrate.
| WP | What | Size | Disjoint files |
|----|------|------|----------------|
| W1.1 | **`hugit checks` deepen + `hugit queue show`** — surface memo-key, hit/miss, cached proof ref; union-batch composition, ETA, failing pair on UNION-FAIL. (SOTA-audit P1.) | M | `hugit-cli/src/checks/*`, `queue/*` |
| W1.2 | **D14 authz guard onto the mutation path** — wrap `EventLog::append`; `--author-kind` must not be caller-spoofable (subagent ≠ orchestrator). | M | `hugit-refstore/*`, `hugit-cli` append sites |
| W1.3 | **Redaction hardening** — pattern+entropy (gitleaks-class) at the capture boundary, not literal-marker. | M | `hugit-ledger/src/redact*` |
| W1.4 | **Serve consumers**: `blob.tree` (file-tree projection) + `hugit symbol --ref` (git-tree-backed outline). | S–M | `hugit-serve/handlers/blob.rs`, `hugit-cli/symbol/*` |
| W1.5 | **`blob.blame`** — intent-graph line attribution into the blob VM (`hugit why` seam). | L | `hugit-serve/handlers/blob.rs`, `why/resolver` |
**Disjointness:** W1.1–W1.5 touch distinct crates/files → parallel fleet, conflict-free. W1.2 + the append sites need a frozen contract first (the guard signature) — I freeze it before dispatch.

## WAVE 2 — Live substrate (owner/infra-gated; I prep + file precise asks, then wire)
| WP | What | Bucket | Note |
|----|------|--------|------|
| W2.1 | Deploy current main + wire Clerk identity (`HUGIT_SESSION_EXCHANGE_URL`) | B | engine already on main; just env + the exchange flip. Near-zero once owner sets env. |
| W2.2 | CoreLink **P2 tenant — hot CAS+AC live** | C | AC client is plug-and-play with one PAT; unlocks the economic thesis (memoized hit-rates). |
| W2.3 | Runner fabric live (corelink-runners) | C | unblocks check execution, merge-as-re-execution, the landing queue EXECUTE. |
**My part:** the wiring code + the conformance probes are buildable now; I file the exact provisioning asks (tenant id, PATs, env) so the owner/CoreLink can unblock in parallel with Wave 1.

## WAVE 3 — Forge completeness + GA economics (after substrate is live)
`git push` (receive-pack) · GitHub App registration + live mirror/import (E-series) · multi-tenant Clerk +
`hugit-prod-d1` · the money gate (B9) with real dogfood cohort data · the build-in-public cost-ledger launch motion.

---

## Orchestration plan (how I run it)
- **I keep judgment + integration; agents execute pre-decided WPs.** Per WP: a chewed packet (target marked,
  contract frozen where dependent), a compact return-shape, central build + cold-verify + merge by me.
- **Parallelism:** Wave 0 (W0.1–0.3) and Wave 1 (W1.1–1.5) are conflict-disjoint by crate → fan out, ≤6 concurrent.
  Coupled edits (W1.2 guard + its append callers) are contract-frozen first or run sequential.
- **Quality bar:** local gate (fmt + clippy -D warnings + test --workspace --locked + deny) per WP, CI CLEAN
  before merge, no merge on a false green, no debt without a logged waiver. Live-probe before any "live" claim.
- **Branch → PR → merge** discipline; the changelog gate is real (every feat/fix needs a CHANGELOG entry).

## Immediate next step
Run **W0.1 (secrets sweep)** now — read-only, zero risk, the hard precondition for going public — then W0.2/W0.3
in parallel, present W0.4 (the public flip) for the owner to execute. Wave 1 fans out once W0 is clear.
