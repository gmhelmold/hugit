# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- feat(refstore): D4 — native intents over the event log + deterministic two-altitude projection (intent/machine), generated commits embedding intent_id, sidecar-corpus import by intent_id, externals stay external-change (no fake intents) (WP-D4)

- feat(refstore): D14 — forge authz: fail-closed mutation guard over push/land/undo/policy, golden per-principal-class permission matrix (human/orchestrator/worker/model), audited denials via EventRecord (WP-D14)

- feat(refstore): D1c — serialized concurrent ops over the single-writer append point, explicit back-pressure (zero silent loss), measured p99<500ms at 100 concurrent ops (WP-D1c)

- feat(app/exit): hugit-app-exit — exit telemetry + the money gate: per-install week-3 retention vs ≥40%, UNPROMPTED/PROMPTED classifier with ≥3 gate, cohort/window guards (n=10, ≥3 weeks, 90-day anchored window), auditable generated report, billing structurally blocked until PASS — DEGRADED evaluator fails closed, enable-billing event audited via EventRecord (WP-B9)

- feat: hugit-checks/affected — affected-target engine v0; cargo/pnpm/turbo graph adapters, root-edit full-set, fail-open policy (WP-B3)

- feat: hugit-refstore — event-log core: append-only hash chain, deterministic replay, tamper detection (WP-D1a)

- feat(queue): hugit-queue union-queue core — batching, union-tree fold, minimal-failing-pair bisection, ordered idempotent landing, structural ordering state machine (WP-B4a)

- feat(checks): C4 — regen drivers v0: lockfile/codegen/snapshot regenerate-never-merge (WP-C4)

- feat: hugit-policy — declarative gate engine v0: 3 ported gates (DCO/changelog/secrets), fail-closed enforcement, audited policy changes via EventRecord (WP-D6)

- feat: hugit-app — GitHub App skeleton: X-Hub-Signature-256 webhook auth + ingest, PR-event persistence, Checks-API write-back, least-privilege manifest, uninstall revoke+halt (WP-B1)

- feat: hugit-contracts — 15 frozen contract types + JSON Schemas + golden serde suite (WP-00)
- feat: Rust workspace scaffold — 12 crates, CI gates (fmt/clippy/test/audit), DCO + changelog discipline (WP-01)
- feat(runner): ephemeral runner v0 — container-per-job lease lifecycle + tmp/net isolation, teardown with forensic re-scan (WP-C2a)
