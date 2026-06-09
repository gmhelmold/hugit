# Audit remediation plan — 7-lens brutal audit (2026-06-09)

> Source: 7 parallel auditors (2 opus: security, correctness; 5 sonnet: hygiene,
> code-quality, test-quality, docs-fidelity, build/deps). TechLead-verified.
> Discipline: **zero-decision contracts** — every fork is pre-decided here; agents
> transcribe/implement, never decide. Oracle-first for behavior changes
> (failing test → fix → green). DoD per fix-package: `cargo fmt --all --check` +
> `cargo clippy -p <crate> --all-targets --locked -D warnings` +
> `cargo test -p <crate> --locked` green; cold-verified by the TechLead on merge.
> Build discipline: set `CARGO_TARGET_DIR` to the main repo `target/` and build
> **targeted** (`-p <crate>`), never `--workspace --all-targets` (disk at 96%).

## False alarms (verified, NOT remediated — recorded so they aren't re-raised)
- **"sidecar excluded from CI / WP-B6 tests never run"** (hygiene+docs) — FALSE.
  `cargo metadata` shows 18 workspace members incl. `hugit-app-sidecar`; Cargo
  auto-includes path-deps residing inside the workspace dir. `--workspace` covers
  it. (Optional cosmetic: list it explicitly in `members` for reader-clarity — FP-7.)
- **"AC client has zero error-path tests, live-only"** (test-quality) — FALSE.
  `crates/hugit-checks/tests/ac_http.rs` covers 401/403/500/503→error, 404→miss,
  undecodable body, content-address guard, fail-closed, PAT redaction, roundtrip.

## Confirmed-solid (adversarially checked — leave alone)
C5b broker, `shell_quote`, tmp/container sanitizers, fail-closed credential scan,
verify-before-spawn pin gate, HMAC tenant partition, P2 seams fail-closed, D1a
tamper (5 mutation modes), D3a/D2b byte-identity, clean `cargo audit`, `--locked`,
no git/path deps, no `build.rs`, no `unsafe` in prod, no `#[ignore]`.

---

## Fix-packages (disjoint claims; merge order = FP-1 → code waves → FP-config → docs)

### FP-1 — Session fence hardening  [SECURITY, TechLead does this directly]
Claims: `.claude/hooks/forbid-sibling-paths.py`, `.claude/settings.json`, `.techlead/profile/profile.json`.
Pre-decided fixes:
1. **Default-deny the parent**: block any path under `~/Documents/HuGR/` that is not
   under `~/Documents/HuGR/hugit/`, instead of enumerating siblings. Fail-closed by
   construction for any new sibling.
2. **Drop `find` and bare `git -C` from the read-only prefixes.** Replace git handling
   with an **allow-list of read-only git subcommands** (`log show diff status blame
   cat-file ls-files ls-tree rev-parse rev-list describe for-each-ref`, `branch
   --list`, `worktree list`, `remote -v`, `tag -l`, `config --get`); deny all others.
3. **Reject shell composition when a HuGR path (or `$`/backtick/assembly) appears**:
   if the command contains `&&`/`||`/`;`/`|`/`$(`/backticks/`>`/`<` AND references a
   forbidden root (or builds a path via a variable), deny — do not try to classify.
4. Add `corelink-workspaces`, `hugr-juiceshop`, `techlead` to `.techlead/profile`
   neverTouch (parity with settings.json), OR fix CLAUDE.md wording (FP-6) — do both.
Acceptance: the audit's bypass vectors become explicit denied-cases (a small
`tests/fence_bypass_cases` harness or inline asserts in the hook's `__main__` self-test):
`find <sibling> -delete` → DENY; `git -C <sibling> gc` → DENY; `cat ok && rm <sibling>/x`
→ DENY; `R=<sibling>; rm -rf "$R"` → DENY; read-only `git -C <sibling> log` → ALLOW;
`cat <sibling>/x` → ALLOW.

### FP-2 — hugit-runner production crashes  [opus, branch `rem/runner`]
Claims: `crates/hugit-runner/src/ws/mod.rs`, `crates/hugit-runner/src/lease.rs`,
`crates/hugit-runner/src/pin.rs`, `crates/hugit-runner/tests/acceptance_c9.rs`.
Pre-decided:
1. `ws/mod.rs` lines 214/227/252/272/295/690 — replace `.lock().unwrap()` /
   `.wait(..).unwrap()` with `.lock().unwrap_or_else(|p| p.into_inner())` (and the
   condvar equivalent), matching `write/order/mod.rs:140`. Add a one-line `// poison
   recovery: the entries map is internally consistent` comment.
2. `lease.rs:227,254` — replace `StrictHostKeyChecking=no` with `=accept-new` plus a
   pinned `UserKnownHostsFile` at `~/.hugit/known_hosts` (env-overridable via
   `HUGIT_RUNNER_KNOWN_HOSTS`); document the pin-on-first-use rationale.
3. `acceptance_c9.rs` — each box-gated early `return;` becomes
   `{ eprintln!("SKIP: HUGIT_RUNNER_HOST unset — box lane"); return; }`.
4. `pin.rs` — add one test `permanent_pull_failure_is_case_insensitive`:
   `is_permanent_pull_failure("Manifest Unknown") == true`,
   `is_permanent_pull_failure("TOOMANYREQUESTS") == false`.
Oracle-first: (1) add a test that a poisoned `entries` mutex still serves
(spawn after a panicking closure) — RED before, GREEN after.

### FP-3 — hugit-proto attribution + json_str  [opus, branch `rem/proto`]
Claims: `crates/hugit-proto/src/write/order/mod.rs`,
`crates/hugit-proto/src/write/receive/mod.rs`, `.../write/external/mod.rs`,
`.../write/store/mod.rs`, `crates/hugit-proto/tests/*` (proto only).
Pre-decided:
1. `order/mod.rs:162` — `SerializedWriter::push` must NOT `.expect()` on
   `record_external_change`. Validate `update.principal_chain` non-empty at the top
   of `push`; on empty, return a `PushOutcome::Rejected(PushReject::MissingAttribution)`
   (add the variant) — never panic. `#[must_use]` on `PushOutcome`.
2. `receive/mod.rs:323` — `receive_pack` must reject an empty `principal_chain` with a
   typed error (symmetry with `record_external_change`), not silently record an
   unattributed event.
3. Consolidate the two `json_str` encoders into one full-escaping `pub(crate) fn
   json_str` in `write/mod.rs`; both call sites use it. Delete the lossy store-side one.
Oracle-first: test `push(RefUpdate{principal_chain: vec![], ..})` → `Rejected`/`Err`
(was: panic); test `receive_pack` empty-chain → `Err` (was: silent unattributed).

### FP-4 — hugit-checks AC client hardening  [opus, branch `rem/checks`]
Claims: `crates/hugit-checks/src/client/ac.rs`, `crates/hugit-checks/tests/ac_http.rs`.
Pre-decided:
1. Validate `memo_key` is `^[0-9a-f]{64}$` in `endpoint()` (or `lookup`/`store`); on
   violation return a new `AcError::InvalidKey`. Validate `tenant` slug is
   `^[a-z0-9][a-z0-9-]{0,62}$` in `AcConfig` construction (it comes from env).
2. Gate `InMemoryAc` with `#[cfg(any(test, feature = "test-utils"))]` and add a
   `test-utils` feature; OR (if cross-crate test use blocks that) `#[doc(hidden)]` +
   a `// TEST FIXTURE — not for production` banner. **Decision: `#[doc(hidden)]` +
   banner** (least disruptive; cross-crate acceptance tests use it via the public path).
3. `read_pat` — on Unix, `stat` the PAT file and return `NotConfigured` (naming the
   file, never the value) if `mode & 0o077 != 0`.
Oracle-first: test `memo_key="../other"` → `InvalidKey`; `tenant="../x"` → error;
a 0644 PAT file → `NotConfigured`.

### FP-5 — hugit-invariants proof rigor  [opus, branch `rem/invariants`]
Claims: `crates/hugit-invariants/x10/**`, `crates/hugit-invariants/x1/**`, and any
`x*/tests/*` named `*_not_vacuous`/`*_caught_red`.
Pre-decided:
1. Refactor `assert_scope_separation()` to take `&ScopeSeparation`; the anti-vacuity
   test calls it with the `broken` value and asserts `.is_err()` (so gutting the real
   function turns it RED). Keep the existing const-driven call for the live path.
2. **Sweep** every `*_not_vacuous`/`*_caught_red` test in the x-series: each MUST feed
   a mutated input to the REAL oracle function and assert it flips, not assert an
   inline re-implemented predicate. Fix each that doesn't (same pattern as #1).
3. `x1` leak-check (`isolation.rs:442-460`): replace the substring deny-scan with an
   **allow-list** assertion — shared fields must EQUAL the platform sentinels;
   anything else fails. (Catches encoded leaks the substring scan misses.)
Oracle-first: each rewritten anti-vacuity test must RED if its real oracle body is
gutted (verify by local mutation before declaring done).

### FP-6 — hugit-queue / policy / refstore / diag  [sonnet, branch `rem/misc-crates`]
Claims (disjoint files): `crates/hugit-queue/src/core/order.rs`,
`crates/hugit-queue/tests/acceptance_wp-b4b.rs`, `.../tests/acceptance_wp-c7.rs`,
`crates/hugit-policy/src/lib.rs`, `crates/hugit-policy/src/gates/secrets.rs`,
`crates/hugit-refstore/tests/acceptance_d1c.rs`, `crates/hugit-refstore/authz/mod.rs`,
`crates/hugit-diag/src/bisect/engine.rs`.
Pre-decided:
1. `order.rs:72` — pre-build `HashMap<&str, UnionOutcome>` from `outcomes` once, then
   O(1) lookup in the loop. Add `#[must_use]` to `land_in_order` + `LandingStep`.
2. `acceptance_wp-b4b.rs:338` — `SECRETS_DIR` const → `std::env::var("HUGIT_SECRETS_DIR")`
   with skip-with-printed-reason when unset (kill the hardcoded `/Users/...` path).
3. `acceptance_wp-c7.rs` — tighten `P95_WAIT_BOUND_MS` from 5000 to
   `3 * tick_period_ms * num_tenants` (make the fairness bound load-bearing).
4. `policy/lib.rs:174` — add a unit test: `Engine` with a `GateDescriptor` whose id has
   no `GateFn` → `eval` returns `Blocked` (the fail-closed `None` arm).
5. `gates/secrets.rs` — add boundary tests: `# token=not-a-secret` comment, a URL with
   `password=`, `Bearer` with no trailing space. Document/adjust any false positive.
6. `acceptance_d1c.rs` — raise `P99_BUDGET` 500→2000ms (it proves serialization, not a
   perf SLA); add a comment.
7. `authz/mod.rs` — add `denial_payload` JSON-shape pin tests per `DenyReason` variant.
8. `bisect/engine.rs:205` — bound the fixpoint loop (`const MAX_ITERS=8` +
   `debug_assert!`), and propagate the serde error instead of `.expect()`.

### FP-CONFIG — supply-chain / legal / CI  [sonnet, branch `rem/config`, SERIALIZED]
Runs alone (touches every manifest + CI). Claims: root `Cargo.toml`, all
`crates/**/Cargo.toml`, `.github/workflows/{ci,dco}.yml`, `.cargo/audit.toml`,
`deny.toml`, `LICENSE`.
Pre-decided:
1. **LICENSE = proprietary / closed source.** Add `LICENSE` (text below). Add
   `[workspace.package]` with `license-file = "LICENSE"`, `publish = false`,
   `rust-version = "1.85"`; every crate inherits via `license-file.workspace = true`,
   `publish.workspace = true`, `rust-version.workspace = true`. NO `license = "..."`
   SPDX (it's not an OSI license).
   LICENSE text:
   ```
   Copyright (c) 2026 HuGR / Gustavo Schneiter. All rights reserved.

   This software and its source code are proprietary and confidential. No license,
   express or implied, by estoppel or otherwise, is granted to any person to use,
   copy, modify, merge, publish, distribute, sublicense, or sell any part of this
   software, except under a separate written agreement signed by the copyright
   holder. Internal use within HuGR / CoreLink under common ownership is permitted.
   Unauthorized use, reproduction, or distribution is prohibited.
   ```
2. `.cargo/audit.toml` — `[advisories] severity-threshold = "low"` and treat
   `unmaintained`/`unsound`/`yanked` as deny; CI step `cargo audit --deny warnings`.
3. `deny.toml` — `[licenses]` allow `["MIT","Apache-2.0","Apache-2.0 WITH LLVM-exception",
   "Unicode-3.0","Unlicense","BSD-3-Clause","ISC"]` for deps; `private = { ignore = true }`
   (our crates are unlicensed-by-design/closed); `[bans]` deny multiple-versions with
   the wasm32-only `wit-bindgen` exception; `[sources]` allow only crates.io. Add a
   `cargo deny check` CI step before audit.
4. `[workspace.dependencies]` — hoist serde, serde_json, sha2, hmac, hex, thiserror,
   anyhow, base64, ed25519-dalek, ureq; crypto primitives **exact-pinned** (`=x.y.z`);
   per-crate deps become `{ workspace = true }`. Unify `thiserror` to `2`.
5. Pin CI actions by SHA: `actions/checkout@v4` →
   `actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683` (4.2.2);
   `taiki-e/install-action@v2` → its current 4.x.x SHA (resolve at edit time).
6. Add `permissions: contents: read` to both `ci.yml` and `dco.yml`.
7. Add `dtolnay/rust-toolchain@stable` (pin `1.96.0`) + `Swatinem/rust-cache@v2`
   (SHA-pinned) before the cargo steps.
DoD for this FP: `cargo metadata --locked` clean, full `cargo build --workspace
--locked` once (TechLead runs the heavy verify), `cargo deny check` green.

### FP-DOCS — documentation fidelity  [sonnet, branch `rem/docs`, parallel-safe]
Claims (docs only): `README.md`, `docs/plan/wp-contracts/WP-E6.md`,
`docs/handoff/2026-06-08-p2-go-live-runbook.md`, `CLAUDE.md`,
`docs/whitepaper/hugit-v1.md`, `CHANGELOG.md`,
`docs/review/2026-06-07-roadmap-gap-build-campaign.md`.
Pre-decided:
1. `README.md` — replace the "No code yet / research phase" status with the accurate
   build-complete summary; keep the "what hugit is/is not" content; point to CLAUDE.md.
2. `WP-E6.md` — header → `SUPERSEDED + BUILT 2026-06-08`; retire `⛔ NO DISPATCH`;
   fix the `## Claims` path `src/writeback/` → `src/sync/`; link the design doc.
3. `p2-go-live-runbook.md` — `HUGIT_QUEUE_AUTOTRIGGER` reads no code: remove the export
   and replace with prose "B5 auto-trigger is a code stub awaiting QueueApi live wiring
   (no env gate yet)".
4. `CLAUDE.md` — "14-crate" → "17-package workspace (15 crates + hugit-app/{ui,exit};
   sidecar auto-included)"; fix the `.techlead/profile mirrors` wording → "is enforced
   by the fence; the profile lists a subset".
5. `whitepaper` — add `**Status 2026-06-08:** L4+L5 hugit WPs complete; see CLAUDE.md`.
6. `CHANGELOG.md` — cut `## [0.1.0] — 2026-06-08`, move `[Unreleased]` entries under it.
7. roadmap-gap review — note X3③ "closes the cascade gap; stub-vs-prod partial tracked
   in the closure report"; update stale SHA reference.

### FP-7 — cosmetic (LOW, last)  [sonnet, folded into FP-DOCS or FP-CONFIG]
Explicit `"crates/hugit-app/sidecar"` in `members`; consolidate `docs/review` vs
`docs/reviews`; normalize acceptance-test filenames to the no-prefix convention (+
`[[test]]` name updates); `REDACTED` marker single-source (needs a hugit-contracts
waiver — **owner-gated, NOT in this fleet**).

## Out-of-fleet / owner-gated
- `REDACTED` marker → `hugit-contracts` (frozen-type edit; needs explicit waiver).
- `.claude/worktrees/agent-*` orphan dirs (72 MB) — destructive-guard blocks `rm`;
  owner runs out-of-band (`rm -rf .claude/worktrees/agent-adb07c4c378687831`).
- git author-email unification (history) — owner decision.
- D2b libgit2 real-client clone: **decision — downgrade the contract claim** to
  documented construction-equivalence (avoid adding a native libgit2 dep on a 96%
  disk); folded into FP-5 as a doc/comment honesty fix, real-clone deferred.
