# WP-DOCK-4 — cost metering contract (gateway wire v1) + spool + attestation

squad: hugit-core (+ Omnirouter irmão, owner-gated) · L→M · opus · 90k · branch: wp/dock-4

## Charter
**What:** the FROZEN wire contract for per-dock cost metering — the DTO the
gateway (Omnirouter) emits and hugit consumes: `{dock_id, model, tokens,
cost_usd_micros, ts}` — plus the local spool (reuse `OutageQueue`) so offline
never loses metering, and the flush path that lands the samples into the
attestation chain (`attest_keyset`), where `why` already reads cost.
**Why:** this is the killer: real, per-unit, non-derived cost. The contract is
frozen v1 + conformance vector + tripwire (B5) so the irmão wire never drifts
again (history: 3 wire-drifts).

## Owned acceptance (VERBATIM from design §5, §7)
- (B5) — contract frozen v1: DTO + conformance vector + tripwire (byte-identical
  both repos, mirroring the #220 freeze discipline).
- (§5) — spool local (reuse `OutageQueue`): cost spooled per-dock; flushed
  when gateway up; offline never loses metering.
- (§5) — attestation: minted entry {dock_id, model, tokens, cost_usd_micros,
  ts} signed/verified via the existing `attest_keyset` seam (#57); lands in the
  attestation chain `why` reads — never re-derived.
- (F3) — decoupled from the irmão: product runs honest today (spool +
  `unlabeled`); live cost owner-gated; `CloseResponse` attestation block stays
  v2 (existing tracked change).
- (A2 / R5) — unlabeled when no dock; reconciled-late on self-heal; cwd-wins
  rule honored by the sample's dock_id resolution.

## Contract deps
- `attest_keyset` (`select_attestation_key`, `verify_with_keyset`) — exists,
  built + conformance-pinned (#57).
- `OutageQueue` — exists (`hugit-mirror/queue`) — reused, not modified.
- `why` reads cost from attestation chain (already wired).
- Dock record + resolver (WP-DOCK-1/2/3).

## Claims (paths)
- `crates/hugit-contracts/` (or the metering seam crate) — the frozen
  `CostSample` DTO + serde golden.
- `conformance/` — the metering vector file + tripwire entry.
- `crates/hugit-cli/src/dock/spool.rs` — spool + flush (reuses OutageQueue).
- `crates/hugit-cli/src/dock/attest.rs` — sign/verify via attest_keyset.
- `crates/hugit-cli/tests/dock_metering_*.rs`.

## Dispatch packet
- This contract + ADR-0005 §5 (+ the freeze rule precedent: #220 vectors).
- Anchor: the `attest_keyset` API + `OutageQueue` API.
- **Owner-gated:** the Omnirouter implementation lands in the irmão repo; this
  WP owns the contract + spool + attest + hermetic fake gateway.

## Properties (Lamport-style)

**M1 (safety — frozen wire):** the on-wire DTO is byte-identical across both
repos; the conformance vector re-verifies on every CI (tripwire). Any drift
fails the gate (fail-closed, never silent).

**M2 (safety — no loss):** every accepted cost sample that reaches the spool
is eventually flushed to the attestation chain OR lands in a residual bucket
(exact-once; no silent drop across gateway outage).

**M3 (safety — no fabrication):** a cost sample is written to the chain ONLY
if it passes the attestation verify (signature/keyset) — a forged sample
cannot enter.

**M4 (safety — honest zero):** absent a gateway sample, cost is `None`/zero —
NEVER derived from internal metrics (the core honesty rule, verbatim from
CLAUDE.md: "NEVER the derived IntentMetrics COGS").

**L4 (liveness — flush):** IF the gateway is up, THEN spooled samples are
flushed (eventually) and land (attested) into the log.

## Implementation notes (pre-decided)
- 1. Freeze the DTO ONE time under a version (`CostSampleV1`) — no silent
  field additions.
- 2. Spool: per-dock file queue (OutageQueue-compatible), flush on gateway
  reachability (backoff like #184).
- 3. Attest: reuse `attest_keyset::sign` with the CURRENT key; verify on read.
- 4. Hermetic fake gateway: a stub emitting signed samples — proves M1-M4
  without the live irmão.

## DoD
- Global gate green.
- B5/M1-M4/L4 red→green, hermetic (fake gateway) + e2e (spool→flush→attest).
- Conformance vector committed + tripwire active in CI.
- Cold-verify pass by non-author.

## Completeness
All owned items green · zero writes outside claims · evidence bundle attached
to SEAL.

## Return shape (SEAL)
status, evidence refs, deviations.