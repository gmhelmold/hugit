# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- fix(policy): route audit-event hash to canonical `hugit_refstore::compute_this_hash` (VEC-framed principal_chain, LP-framed fields); delete bespoke hasher that diverged on principal as scalar with no u32 count prefix; strengthen acceptance_d6 item③ oracle to pin `this_hash` against the refstore formula (WP-rpolicy-hash)

- fix(cli): R-cli — remediation of the brutal-review hugit-cli findings, oracle-first (each oracle strengthened to RED on baseline, then fixed to GREEN): (1) the REAL `hugit` binary now exists — a clap dispatch shell wiring `why`/`impact`/`tournament`/`export` end-to-end to the library with correct exit codes (0 success / non-zero error), and the canonical verb registry is exposed as `hugit_cli::HUGIT_VERBS`/`hugit_verbs()` for WP-X5 to consume the real surface; (2) export streams the JSON envelope field-by-field / element-by-element (no whole-envelope `to_vec`), with a `peak_serialize_scratch` OOM-bound proof; (3) `hugit why` resolves line ranges + symbols (two different lines on one file → different events; unattributed line/symbol rejected, never mis-attributed); (4) export git-object OIDs are path-traversal-sanitized before any write (fail-closed); (5) cut `ref_state()` returns `Result` — a malformed ref payload is a hard export failure, never silent-empty; (6) the persuasion negative test is now a real structural barrier proof (a persuadable reviewer that is NOT flipped) and `RedactionManifest::content_ref()` propagates serialize errors instead of a fixed hash (WP-R-cli)

- fix(fence): R-fence — real enforcement seam + genuine fence redteam vectors (remediation): the named enforcement gate (`check_access`/`is_admitted`) is now the single live predicate the materialize seam routes every candidate through (no dead enforcement API; a path the gate denies can never be placed); a sixth red-team vector (`fence_materialized_escape`) materializes a real `FenceManifest` then reads an out-of-fence path in the *same* container — the one vector where the fence (`classify` + sparse materialize), not the Docker namespace, is the control, so it escapes under a no-op classifier; ENOENT probe is locale-independent (`test ! -e` exit-code, dir-at-path handled, fail-closed on probe-shell error); fork-bomb/disk-fill now sample the peak over a window and assert the cap actually BIT (saturation / ENOSPC observed), not a single race-y sample; classifier oracle hardened (symlink-into-fence, absolute-path injection, prefix-collision, `..`-escaping-root); allow-all (`./`/`.`/``) path_set entries rejected fail-closed before any box command; `place_file` re-guards no-`..`; credential scan fails closed on an unparseable/truncated report (no default-clean); broker enforces lease→op authz (non-`Held` lease refused) with the lease↔container trust boundary documented (WP-rfence)

- fix(ledger): R-ledger — complete view-redaction coverage + surface malformed records (remediation): routes intent_id/campaign/deep_link_target through redact::apply in the ledger projection (previously surfaced raw); adds redaction to fleet workspace_id/agent_id and deeplink target; adds FleetState.malformed counter (malformed payloads are counted, never coalesced to "unknown"); adds EventClass::Other so unknown event kinds no longer mislabel as Landing. Oracle-first: six new acceptance assertions go RED on old code, GREEN after fix. D5/D11 preserved green.

- fix(queue): R-queue — THE WEDGE remediation (oracle-first). Pair-exclusion now lands end-to-end: a `UnionFail` predecessor is transparent, so innocent successors behind an excluded pair actually land (was structurally blocked by the ordering gate). Recovery replay on the same batch is idempotent (no `AlreadyTerminal` hard-error). Bisection is minimal+honest: an individually-red item is a `SingleItem` failure (never a false pair), and an unlocalisable red union is an explicit `Unlocalised` (never a silent empty drop). Duplicate `order_index` is rejected at batch construction. Stale-head is enforced at the engine, not delegated — a force-pushed stale union is refused even when the `MergeApi` ignores `expected_head` (WP-R-queue)

- fix(proto): R-proto — remediation of the brutal-review write/read-path defects. Wire the flag-gate and compare-and-append total-order into the REAL receive-pack ingest (flag off ⇒ push refused before any CAS write/event; concurrent real pushes get a contiguous total order, stale tip ⇒ rejected, no lost update — via the single-writer `SerializedReceiver`); require the ref target be REACHABLE from the pushed pack (not merely present in the scratch odb); cap INFLATED bytes + object count to defuse decompression/object-count bombs (compressed bound alone no longer admits a tiny pack that inflates huge). Read path: degradation kill-test now exercises an observable smart-layer that Disabled genuinely bypasses (no longer `let _ = state`); CPU-budget fallback routes on the MEASURED cost of the actually-assembled pack (not an injected estimate); client-matrix conformance round-trips the real serve pack through a real `git` clone and diffs against the source (no self-comparison; FAIL-not-skip when git absent). Plus: `GitObject::try_oid` propagates a hashing error instead of `expect`, and the single-writer mutex recovers a poisoned lock instead of panicking. Every defect proven oracle-RED-then-GREEN. (R-proto)

- fix(runner): R-runner — enforce X4 pin on real spawn surface + sanitize tmp_root (remediation). Per the brutal review (R2/R4 §hugit-runner): the supply-chain pin/verify was a wrapper the live path bypassed, and `tmp_root` flowed unsanitized into `sh -c` (root RCE). Now `ContainerSpec::from_lease` rejects any non-`@sha256:`-pinned image AND validates `tmp_root` against `^/[A-Za-z0-9._/-]+$`; `DockerEngine::spawn` integrity-verifies the pin against the box BEFORE `docker run` (fail-closed, no container on failure). Added a hermetic `FakeBox`/`FakeEngine` oracle proving verify-before-run + tmp_root-reject offline in the bare `cargo test --workspace` gate (no box, fail-not-skip). Also: the dedup spawner no longer holds the global lock across spawn (claim-then-spawn with a per-id condvar) and liveness-probes cached handles (no dead-container reuse); state-restore streams its payload over stdin instead of shell-constructing it; `shell_join` always-quotes (no allowlist passthrough). Concurrency fix (cold-verify caught): under ≥8 jobs sharing one pinned digest, `verify_on_box` raced the Docker daemon — 8 simultaneous pulls of the SAME digest returned a *transient* error that was misread as an integrity failure and failed CLOSED, killing a genuinely-pinned job (C2b item_4 spurious fail). Now verify (a) serializes same-digest pulls through a per-digest lock (the box pulls a given digest once at a time; distinct digests never block each other) and (b) classifies pull errors — a permanent signal (manifest unknown / not found / digest mismatch / denied) fails CLOSED on the first attempt with NO retry, while a transient network/daemon/race hiccup is retried a bounded number of times with backoff. Tamper rejection is unchanged: an unpinned/tampered digest still fails closed before spawn. Added hermetic regression tests (transient-retried, permanent-fail-closed-no-retry, budget-exhaustion-fail-closed, 8-thread same-digest all-succeed, classifier). Box suites c2a/c2b/c9 green with resolved pinned digests (WP-R-runner)

- fix(mirror): R-mirror — structural one-way proof + real import/LFS oracles + token redact (remediation): the SACRED one-way invariant is now proven by construction (the only mirror-mutation primitive `MirrorMutation` can carry only the forge tip; a runtime architecture oracle scans the shipped one-way surface and goes RED if any reverse-sync sink is introduced) instead of a `const CODEPATH_PRESENT=false` fiat; the E2a① "1k-commit byte-identical import" oracle now runs the real `import_commits`/`read_git_object` path against a real on-disk git repo and compares to `git rev-list`/`git cat-file` (no hand-built strings); `read_git_object` validates the cat-file `--batch` header (oid/type/size) and slices exactly `size` bytes; the private-repo `InstallationToken` no longer derives `Debug` over a cleartext token (private field + redacting `Debug` + `expose()`, oracle asserts `{:?}` omits the secret); real Git LFS batch-API fetch implemented (`materialize_lfs_via_batch` speaks the real batch protocol over an `LfsTransport`, driven in-gate by a real on-disk LFS server fixture, fail-closed SHA-256/size verify); PR/issue batch import dedups by `intent_id` (two identical fixtures → one intent); divergence `mirror_write` derived from an observed `MutationOrigin::MirrorSide` event, not tip-inequality (no false positive on lag); E1a① landing + E1a③ soak now driven through at least one REAL git readback (`git rev-parse`), not echo-by-FixtureMirror. All defect oracles confirmed RED on the pre-fix code, GREEN after (WP-RMIRROR)

- fix(app): real uninstall token revocation + halt + use installation token (remediation): `WebhookProcessor` now carries an `Arc<Mutex<HashMap>>` token store — `handle_uninstall` removes/zeros the entry and returns `token_revoked` based on actual stored state; `PersistenceAdapter::halt_installation` inserts into a real halted-set and `persist_event` now accepts `installation_id: Option<&str>` and returns `PersistenceError::InstallationHalted` for halted installs; `write_check_run` returns `ChecksClientError::TokenRevoked` on `None` token; oracle-strengthened acceptance suite goes RED on stubs, GREEN on fixes (WP-rapp-uninstall)

- fix(contracts,refstore): R0 — single-source + byte-exact hash-chain/memo-key/attestation formula: canonical `compute_this_hash`/`compute_memo_key`/`canonical_json`/`attestation_sig_preimage` in hugit-refstore (the one source of truth), byte-exact frozen doc-specs in contracts (LP/VEC framing, recorded_at excluded, canonical-JSON payload, 64-ASCII-'0' genesis), and an independent cross-crate hash-pin so re-transcription drift (brutal-review R1) goes RED (WP-R0)

- feat(runner): C9 — workspace lifecycle: attach/resume/spawn-dedup, local≡remote (WP-C9)

- feat(fence): C5b — secrets broker v0 (credentials never enter the runner; principal-chain audit; fail-closed) + active escape red-team harness (traversal/symlink/out-of-fence/fork-bomb/disk-fill contained, box residue 0) (WP-C5b)

- feat(diag): D8 — experiment harness + gate binds (claims/regen blocked until PASS) (WP-D8)

- feat(queue): C10 — pricing no-shock: per-surface cap/degrade + pre-exhaustion warning + zero-overage billing fixture (WP-C10)

- feat(proto): D2a — git wire read path: protocol-v2 negotiate/ls-refs/want-have, pack assembly from CAS, byte-identical clone + delta-only fetch (WP-D2a)

- feat(ledger): D11 — session journals + ctx resume: tenant-private journal objects bound to ws/intent, within-horizon reconstruction, beyond-horizon documented refusal (WP-D11)

- feat(runner): E4 — Actions-YAML shim v0: published supported-subset contract (15 features, proven-to-execute), explicit out-of-contract actionable reports, secrets fail-CLOSED via broker (red-team asserted), execution equivalence harness with determinism-precondition gate (PARTIAL: shim lane green, live GH lane wired) (WP-E4)

- feat: hugit-invariants — X4 supply-chain: content-pinned runner images, verified deps, fail-closed before spawn (WP-X4)

- feat(invariants): X5 — namespace laws: no hugit CLI verb shadows a git verb (mechanized `git help -a` check), managed refs/hugit/… never collide with user branches/tags (property test corpus) (WP-X5)

- feat(mirror): E2b — PR/issue import: proposed/non-authoritative intents with per-element provenance, fidelity contract (body/comments/state/labels/cross-refs), explicit NON_IMPORTED enumeration, idempotent re-sync (WP-E2b)

- feat(mirror): E1c — verified-mirror bootstrap + disaster recovery: resumable cold-seed of full history to a fresh repo (byte-identity hash-verified), GitHub-side loss DR (App-revocation + repo deletion/rename detected → fail-closed incident with pinned recovery-source and resume-from-recovered-state), and substrate-loss DR — the mirror is a byte-complete working git repo, recovery proven end-to-end, recovered content imports as change-events never fabricated intents (WP-E1c)

- feat(mirror): E1a — one-way mirror (hugit→GitHub): outbound sync writer + GitHub App installation-token auth, per-push content-hash verify (byte-identity, fail-CLOSED → divergence), durable ordered/capacity-bounded outage queue (stated bound, overflow → backpressure + incident, never drop/reorder), <60s SLA + soak metric; live GH lane PARTIAL when installation uncovered, never faked (WP-E1a)

- feat(cli): E5 — export + exit proof: one-command git+JSON dump validated against the versioned ExportSchema (machine check), object-for-object restore round-trip, redaction-at-export with manifest, bounded-memory streaming, point-in-time-consistent cut over D1, terminating-account read-only path, and THE EXIT PROOF — exported artifact clone/log/branch/push with ZERO hugit tooling on PATH; redaction red-team asserts seeded secrets nowhere (WP-E5)

- feat: D9 — attention queue: documented composite ranking (policy × blast-radius × confidence), policy-mandatory floor, fast-approve (90s) gating (blocked for high-risk/mandatory, permitted for low-risk), honest "ranking degraded" up-zoom under missing inputs (WP-D9)

- feat(proto): D2b — read-path edges: client matrix (git 2.40+/jj/libgit2), jj first-class stacked changes with change-ids stable across forge ops + identical stack reconstruction, per-request CPU budget (70% of platform limit) + chunked fallback, degradation kill-test (smart layers off, steady-state + mid-op → vanilla git still serves), per-dimension scale ceilings with documented bounded refusal (PARTIAL: jj binary absent locally → item ⑦ wire round-trip skipped, in-process model proven) (WP-D2b)

- feat(proto): D3b — push concurrency: concurrent pushes serialized through the D1 single-writer point get a strict total order with compare-and-append stale rejection (no lost update); raw push = opaque external-change event with attribution (who/when/ref), never a fabricated intent; write path off unless self-hosted-alpha; intent-log-clean negative + write-path red-team (WP-D3b)

- feat(cli): D13 — tournament: N-candidate fan-out, judge-panel selection per documented criteria, losers addressable as evidence, policy-capped + C7 budget-bounded (zero overage) (WP-D13)

- feat(checks): D12 — regen gate: opt-in scope only, lands only on acceptance re-pass + fresh independent verdict (fail-closed), per-regen attestation records the authorizing gate-verdict ref, anti-smuggling blocks+audits false "derived" declarations (WP-D12)

- feat(mirror): E1b — failure modes: outage durable queue with bounded backoff (no drop/reorder, drain-to-verified + gap incident), force-push/branch-delete/tag replication with orphan-aware no-false-divergence, partial divergence repaired scoped to the broken ref, webhook-loss poll fallback within SLA, and ONE-WAY enforced — reverse mirror writes treated as divergence → forge-authoritative repair, zero reverse-sync codepath (WP-E1b)

- feat(mirror): E2a — git history import: byte-identity (OID round-trip via SHA-1), LFS object materialization (SHA-256 verified, not pointer), resumable across timeouts (cursor-based, no restart-from-zero), idempotency (unchanged→no-op, changed→incremental, no dupes), import boundary (bare commit→opaque EventRecord, no intent synthesized) (WP-E2a)

- feat(invariants): X3 — context privacy: tenant-scoped journals (cross-tenant fetch denied + audited), redaction at capture AND export (ExportSchema-validated), retention/deletion purge verified-absent, training/eval exclusion documented control + audit trail (WP-X3)

- feat(invariants): X2 — attestation e2e with ed25519 public verification: full provenance chain (tree+def+runner+model+principal) resolves cryptographically, tampered/unsigned/forged rejected fail-closed at promotion (with audit), PUBLIC verification procedure provable with the public key alone (signing key never needed/present — replaces the prior HMAC keyed-MAC), cross-tenant shared-hit honesty via anonymized PLATFORM attestation (no producer-tenant leak, no mis-attribution) (WP-X2)

- feat(proto): D3a — git push path v0: receive-pack→CAS+log, raw push=change-event (WP-D3a)

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
