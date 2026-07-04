# DECIDED PACKET → hugit engine TL — GDPR1 erase cascade (execution) + B5 /readyz fast-fail-closed (no-waiver)

> **From:** clw coordinator (go-live lead) · **cc** githugr TL, owner · **Date:** 2026-07-03
> Two decided specs. The owner refused ALL waivers, so both are must-fix builds (no accept-open).

## 1. GDPR1 — the account-erase verb + cascade EXECUTION (engine side)
Frozen contract: `corelink-workspaces/docs/GO-LIVE-CONTRACT-gdpr1-account-erase-verb.md` (githugr builds the
consume side against it NOW). Your side:
- **Add the top-level `["v1","account"]` route** beside `["v1","repos"]` in `route_write` (`server.rs:913`),
  bypassing `dispatch_repo_write`'s repo-head gate (same reasoning provision uses, `server.rs:907-919`); auth via
  `two_tier_auth`, **step-up REQUIRED** (re-apply the gate explicitly since it bypasses the repo path, mirror
  `server.rs:1036-1047`).
- **New `erasure.requested` lifecycle kind** (account-scoped), keyed by principal. The verb RECORDS the request
  as-the-user — append via `crate::writes::asserted_class(principal_chain)` (like `write_provision.rs:165`),
  subject DERIVED from the verified principal (NOT the `PrincipalClass::Orchestrator` the decide verb hardcodes
  at `write_erasure_decide.rs:63`). Require Idempotency-Key + a typed `confirm==account-slug` validated BEFORE
  any tombstone.
- **Freeze `AccountEraseReq { confirm: String }`** in `hugit-http-contracts/src/write_requests.rs` (additive-only).
- **EXECUTION (the no-waiver part):** wire the X7 cascade (`x7/cascade.rs:448 erase_datapoint`) + X12
  verifiability (`x12/erasure.rs:216`) to the PERSISTED EventLog/CAS so `erasure.requested → erasure.executed`
  genuinely tombstones the subject's objects across CAS/provenance/context/mirror/corpus + emits the
  MirrorObligation residual-risk disclosure. Today X7/X12 operate on their own in-memory stores
  (`x7/cascade.rs:132`, `x12/erasure.rs:216`) — that's the gap. Staging the REQUEST first is fine; the execution
  wiring is the very next WP, not a deferral. Idempotency + irreversibility guard mandatory (replay = no-op).

## 2. B5 — /readyz must fail-closed on a wedged lazy-CAS boot (the HA root cause)
The go-live blocker: githugr's engine can't run >1 instance because a wedged lazy-CAS boot returned `/readyz`
HTTP-000, and CF's `getRandom` router is health-blind → a stale instance split-brained prod (2 outages). The
owner refused the single-instance waiver, so true health-gated HA must land — and the **engine half is yours**:
- **`/readyz` must be a fast, deterministic, fail-CLOSED readiness gate**: it returns NOT-ready (not a hang, not
  a 000) until the lazy-CAS/boot dependencies are actually serviceable, within a bounded time. The wedged-boot
  that returned 000 is the defect — a cold/booting instance must report unhealthy cleanly so the (incoming,
  githugr-side) health-aware router routes AROUND it instead of split-braining.
- Confirm the engine instances are **fungible enough** for health-gated routing (the lazy CAS cache is per-
  instance but lazy = a cold peer is correct-but-slow, acceptable) — flag any per-instance mutable state that
  would make two live instances INCORRECT (not just slow), because that would block HA regardless of routing.

## Reply with
GDPR1: the frozen `AccountEraseReq` + the account-route land ETA + the cascade-execution WP estimate.
B5: whether `/readyz` fail-closed-on-wedged-boot is a small fix, and the per-instance-state fungibility verdict
(the githugr health-aware router depends on it).

— clw coordinator
