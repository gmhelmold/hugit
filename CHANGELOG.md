# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- feat(proto): D3b — push concurrency: concurrent pushes serialized through the D1 single-writer point get a strict total order with compare-and-append stale rejection (no lost update); raw push = opaque external-change event with attribution (who/when/ref), never a fabricated intent; write path off unless self-hosted-alpha; intent-log-clean negative + write-path red-team (WP-D3b)

- feat(proto): D2b — read-path edges: client matrix (git 2.40+/jj/libgit2), jj first-class stacked changes with change-ids stable across forge ops + identical stack reconstruction, per-request CPU budget (70% of platform limit) + chunked fallback, degradation kill-test (smart layers off, steady-state + mid-op → vanilla git still serves), per-dimension scale ceilings with documented bounded refusal (PARTIAL: jj binary absent locally → item ⑦ wire round-trip skipped, in-process model proven) (WP-D2b)

- feat: D9 — attention queue: documented composite ranking (policy × blast-radius × confidence), policy-mandatory floor, fast-approve (90s) gating (blocked for high-risk/mandatory, permitted for low-risk), honest "ranking degraded" up-zoom under missing inputs (WP-D9)

- feat: hugit-invariants — X4 supply-chain: content-pinned runner images, verified deps, fail-closed before spawn (WP-X4)

- feat(runner): E4 — Actions-YAML shim v0: published supported-subset contract (15 features, proven-to-execute), explicit out-of-contract actionable reports, secrets fail-CLOSED via broker (red-team asserted), execution equivalence harness with determinism-precondition gate (PARTIAL: shim lane green, live GH lane wired) (WP-E4)

- feat(ledger): D11 — session journals + ctx resume: tenant-private journal objects bound to ws/intent, within-horizon reconstruction, beyond-horizon documented refusal (WP-D11)

- feat(proto): D2a — git wire read path: protocol-v2 negotiate/ls-refs/want-have, pack assembly from CAS, byte-identical clone + delta-only fetch (WP-D2a)

- feat(queue): C10 — pricing no-shock: per-surface cap/degrade + pre-exhaustion warning + zero-overage billing fixture (WP-C10)

- feat(diag): D8 — experiment harness + gate binds (claims/regen blocked until PASS) (WP-D8)

- feat(fence): C5b — secrets broker v0 (credentials never enter the runner; principal-chain audit; fail-closed) + active escape red-team harness (traversal/symlink/out-of-fence/fork-bomb/disk-fill contained, box residue 0) (WP-C5b)

- feat(runner): C9 — workspace lifecycle: attach/resume/spawn-dedup, local≡remote (WP-C9)

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
