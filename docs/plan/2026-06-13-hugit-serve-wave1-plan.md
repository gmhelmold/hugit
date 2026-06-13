# Plan — `hugit-serve` Wave 1: the `/v1` HTTP engine port (githugr read-path)

**Owner-greenlit 2026-06-13** ("read-path real + auth-stub"). Source request:
`../githugr/docs/handoff/2026-06-13-hugit-http-server-request.md` (P0, unblocks
githugr.com `live`). Frozen contract: `../githugr/docs/spec/2026-06-11-backend-api-v1.md`
(§1 reads · §2 SSE · §3 writes · §4 identity). Canonical VM shapes:
`../githugr/crates/githugr-vm/src/provider.rs` (3,493 lines). Client oracle:
`../githugr/crates/githugr-live/{src,tests/parity.rs}`.

## Scope (Wave 1, owner-approved)
- **5 reads** over REAL local engine state: `home · landing · prs/{n} · checks · commits`.
- **Auth = Bearer STUB** (dev token, tenant-scope by path). Real Clerk JWKS + RFC-8693
  token exchange is the **ADR-0002 identity rollout = P2** (needs the CoreLink tenant);
  honestly deferred + disclosed, not faked.
- Error envelope `{code, reason}`, the §3 status→code map, ETag=`seq`, **404-no-leak**
  (absent == forbidden), **503 fail-honest** (never a fake-empty VM).
- **Out of Wave 1 / deferred:** SSE stream (§2, Wave 2), all writes (§3), live
  fleet-shared check KPIs (P2 — PS-1; serve LOCAL-real hit-rate only, honest).

## Decisions (tech-lead, within the greenlight)
1. **Synchronous HTTP server, NOT axum/tokio.** This workspace is supply-chain-strict
   (exact-pinned deps, `[bans] multiple-versions = "deny"`, license allowlist) and
   **async-free** (client is sync `ureq`). axum/tokio would explode the dep tree and
   fight `cargo deny`. Use a minimal sync server (tiny_http-class). The wire contract is
   server-impl-agnostic — the frozen client can't tell. SSE (Wave 2) revisits this.
2. **`hugit-serve` is the integrator/adapter, not engine core.** The VMs are UI-shaped
   (humanized `age`, pt-BR summaries, `color_class`, `display_label`) — surface concerns.
   They live in `hugit-serve` (domain→VM mapping) + `hugit-http-contracts` (the wire
   types) ON TOP of the domain-pure engine crates. The headless-engine core stays clean;
   the coupling is contained in one adapter layer (the window's "you are the integrator").
3. **`hugit-http-contracts` = frozen wire types**, transcribed BYTE-FOR-FIELD from
   `githugr-vm`, round-trip-tested against the contract's Appendix-A JSON. A field change
   is a wire break (same discipline as the byte-identical `conformance/` vectors).

## Status
- ✅ **`hugit-http-contracts` LANDED (anchor):** `RepoHomeVm` + `CommitsVm` (+ nested:
  Tree/About/LastCommit/Synergy/CommitRow/CommitDay) transcribed exactly; 2 round-trip
  parity tests green; fmt/clippy clean; **zero new deps** (pure serde). This freezes the
  contract for the home+commits verticals.
- ⬜ Transcribe the remaining Wave-1 VMs into the anchor: `LandingVm` (largest — drawers,
  cost, union, intents, diffs), `ChecksVm`, `PrDetailVm` (+ their nested).
- ⬜ `hugit-serve` crate: sync HTTP server, routes `/v1/repos/{repo}/{home,landing,
  prs/{n},checks,commits}` + `/readyz`, Bearer auth-stub middleware, `{code,reason}`
  error marshaler, ETag/404-no-leak/503 transport rules.
- ⬜ Reconcile new deps with `deny.toml` (license allowlist + any multiple-versions
  exceptions) — keep the gate green; this is a real supply-chain-surface expansion to
  land deliberately, owner-visible.
- ⬜ Domain→VM mappings from real engine state (each a disjoint handler; parallelizable
  per-endpoint against the frozen contract): home←`hugit-refstore`/`hugit-proto`;
  commits←`hugit-proto`/`hugit-refstore`; landing/pr_detail←`hugit-queue`/`hugit-ledger`/
  `hugit-policy`; checks←`hugit-checks`/`hugit-diag` (LOCAL hit-rate, honest).
- ⬜ Integration tests: boot the server, hit each route with the stub token, assert the
  serialized JSON parses back through the contract types (mirror `parity.rs` asserts);
  401 without token, 404-no-leak, 503 paths.

## Open questions routed back to the githugr TL (their §9) — to answer in the reply doc
JWKS source (P2/CoreLink) · `seq` per-repo vs global · cost precision (`cost_usd_micros`
↔ VM `f64`/`u64`) · cache-hit provenance source · `/ask` stub allowed in W1 (yes) ·
secret-gate read vs write time · sealed-intent eternal cache. Most are P2-adjacent; the
read-path doesn't block on them.

## Honest cadence
The anchor is done + verified. The server (sync HTTP + auth-stub + the 5 mappings +
deny reconciliation + integration tests) is the next focused increment — a real
multi-hour build, done with full gate rigor (the 12h pressure does NOT loosen the gate).
Real-auth + live-KPIs remain P2 (owner/tenant), disclosed, never faked.
