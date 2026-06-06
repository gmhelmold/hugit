# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- feat(proto): D2a — git wire read path: protocol-v2 negotiate/ls-refs/want-have, pack assembly from CAS, byte-identical clone + delta-only fetch (WP-D2a)

- feat(queue): C10 — pricing no-shock: per-surface cap/degrade + pre-exhaustion warning + zero-overage billing fixture (WP-C10)

- feat(app): B6 — intent sidecar: parse/validate/render + corpus to CAS ref (WP-B6)

- feat: hugit-checks/affected — affected-target engine v0; cargo/pnpm/turbo graph adapters, root-edit full-set, fail-open policy (WP-B3)

- feat: hugit-refstore — event-log core: append-only hash chain, deterministic replay, tamper detection (WP-D1a)

- feat(queue): hugit-queue union-queue core — batching, union-tree fold, minimal-failing-pair bisection, ordered idempotent landing, structural ordering state machine (WP-B4a)

- feat(queue): hugit-queue GitHub integration — ordered atomic merge API with honored merge method, force-push union recompute, branch-protection holds (never force-merged), crash-idempotent kill-test recovery; live App-JWT installations lane (WP-B4b)

- feat(checks): C4 — regen drivers v0: lockfile/codegen/snapshot regenerate-never-merge (WP-C4)

- feat: hugit-policy — declarative gate engine v0: 3 ported gates (DCO/changelog/secrets), fail-closed enforcement, audited policy changes via EventRecord (WP-D6)

- feat: hugit-app — GitHub App skeleton: X-Hub-Signature-256 webhook auth + ingest, PR-event persistence, Checks-API write-back, least-privilege manifest, uninstall revoke+halt (WP-B1)

- feat: hugit-contracts — 15 frozen contract types + JSON Schemas + golden serde suite (WP-00)
- feat: Rust workspace scaffold — 12 crates, CI gates (fmt/clippy/test/audit), DCO + changelog discipline (WP-01)
- feat(runner): ephemeral runner v0 — container-per-job lease lifecycle + tmp/net isolation, teardown with forensic re-scan (WP-C2a)
