# Test Quality Sweep — hugit engine `crates/`

**Date:** 2026-06-12  
**HEAD:** `5457730` (branch `integ/web-spine`)  
**Scope:** All Rust test code under `crates/` — 21 test directories,
~1250 tests / 142 suites (acceptance_* external suites + inline
`#[cfg(test)]` modules). Read-only analysis; no mutations.

**Lens:** Are the tests SOTA, or theater?

---

## Findings

### P1 — False confidence / critical gap

#### T-1: Tautological shadow test
- **File:** `crates/hugit-checks/src/shadow/tests.rs:96,139,141`
- **Why not load-bearing:** `run_explicit_job` is literally
  `if job_passes { Passed } else { Failed }`. The two tests call
  `run_explicit_job(true)` → assert `Passed` and `run_explicit_job(false)`
  → assert `Failed`. No real computation, no scheduler, no shadow
  interaction. These tests would pass even if every other line of the shadow
  scheduler were deleted.
- **SOTA test:** Test that a `ShadowDecision::CapHalted` returned by
  `admit_shadow` does NOT affect the explicit-job outcome — both paths run
  through `admit_shadow` → `run_explicit_job` and the explicit result is
  independent of the shadow branch. Currently zero tests exercise the
  scheduler ↔ explicit-job interaction.

#### T-2: Policy gate silently diverges from ledger engine
- **File:** `crates/hugit-policy/src/gates/secrets.rs:13–33`
- **Why not load-bearing:** The policy gate carries 16 hardcoded
  `(&str, &str)` substring patterns. Missing vs. the ledger engine
  (`hugit-ledger/src/secret_shape.rs`): the `clp_` prefix, `github_pat_`
  prefix, `xoxo-`/`xoxa-`/`xoxs-` Slack token prefixes, entropy scan,
  connection-string detection, and the keyword-context
  whitespace-around-separator path (`password = value`). The `cas:` exemption
  hole fixed in K-SCRUB is also absent from the policy gate.  
  No test asserts parity: a credential matching a `secret_shape`
  `is_structural_secret` pattern (e.g. `clp_live_…`) passes the policy gate
  while being redacted by the ledger engine. Silent divergence between the
  guard that governs what is *accepted* and the guard that governs what is
  *stored*.
- **SOTA test:** A cross-gate parity test: for each prefix in
  `secret_shape::KNOWN_STRUCTURAL_PREFIXES`, assert
  `secrets::eval(ctx_with_field("f", sample_token)).is_fail()`. Also add a
  negative: for each `cas:`-prefixed valid CAS address, assert the policy gate
  does NOT reject it (mirror the K-SCRUB exemption). This test should live in
  `crates/hugit-policy/tests/` and import from both crates.

#### T-3: Global env mutation in parallel test binaries
- **Files:**
  - `crates/hugit-checks/tests/ac_http.rs:514–517`
    (`ENV_AC_URL`, `ENV_TENANT`, `ENV_PAT_FILE`)
  - `crates/hugit-cli/src/checks/run.rs:1679,1694,1704,1718,1732`
    (allowlisted-var test with save/restore)
- **Why not load-bearing:** `unsafe { std::env::set_var(...) }` is
  process-global. The `PAT_PERM_LOCK: Mutex<()>` in `ac_http.rs` serializes
  within that test file, but other tests in the same binary can observe the
  mutation window. Cargo parallelizes tests within a binary by default. If a
  concurrent test reads an env var that is momentarily set to a test value
  (e.g., `ENV_AC_URL` pointing to a test base), it may silently pass or fail
  for the wrong reason — a flaky interaction that is invisible in sequential
  runs.
- **SOTA fix:** Use the `serial_test` crate (`#[serial]`) for all tests that
  mutate env, or spawn a subprocess with the env set only for that child, or
  replace env vars with dependency injection (pass the config struct directly
  to the function under test).

---

### P2 — Weak tests

#### T-4: `AcError::Busy` path never exercised end-to-end
- **File:** `crates/hugit-checks/tests/acceptance_b2a.rs`
- **Why weak:** `InMemoryAc` (`crates/hugit-checks/src/client/ac.rs`) is the
  backend for the memo key acceptance suite. It is a `Mutex<HashMap<…>>` with
  no failure path; it never returns `AcError::Busy`. Wave K's `K-ERRLAW2`
  closed the taxonomy collapse (`ac_busy` → `ac_error`) and stress-verified
  the gate 10/10, but that verification is only through an inline unit test
  (`ac_busy_lock_exhaustion_maps_to_retryable_kind` in `run.rs`) — not through
  any acceptance path where an AC backend actually contends under load.
- **SOTA test:** A test that drives `HttpAcClient<MockTransport>` with a
  transport that returns HTTP 429, and asserts (a) `AcError::Busy` is
  returned, (b) the caller maps it to the `retryable` error kind, and (c) a
  second call after the mock clears succeeds. This closes the seam between the
  taxonomy and the live HTTP path.

#### T-5: Scratch dir collision risk in `acceptance_wj_matrix.rs`
- **File:** `crates/hugit-cli/tests/acceptance_wj_matrix.rs:47`
- **Why weak:** `scratch(tag)` incorporates only `process::id()`. Static
  per-test tags (`"camp-addr"`, `"intent-ulid"`, `"pr-addr"`,
  `"verdict-freetext"`, `"check-ac-closure"`, `"la-intent-id"`,
  `"la-intent-camp"`, `"la-pr-principal"`) are each used in exactly one test
  function, but cargo parallelizes test functions within a binary. The
  `remove_dir_all` + `create_dir_all` sequence inside `scratch` is not atomic.
  Two functions with the same tag that both run during a `cargo test` on a
  multi-core machine share a scratch directory — the second `remove_dir_all`
  deletes work in progress for the first, causing a non-deterministic failure.
  (Contrast: `acceptance_wave_m_readpath.rs` uses an `AtomicU64` counter for
  per-invocation uniqueness.)
- **SOTA fix:** Add a per-call nonce to `scratch` — e.g., an
  `AtomicU64::fetch_add` counter at module scope, as already done in the
  read-path suite — so every invocation gets a unique directory regardless of
  the tag string.

#### T-6: `acceptance_wg_scrub.rs` presence-not-field assertions
- **File:** `crates/hugit-cli/tests/acceptance_wg_scrub.rs:154–157`
- **Why weak:** The key assertion pattern is
  `assert!(!bytes.contains(PAT))` + `assert!(bytes.contains(REDACTED))`.
  The second assert confirms the redaction sentinel appears *somewhere* in the
  serialized log, but not that it replaced the specific field that contained
  the secret (e.g. `principal_chain`). If a different field was accidentally
  redacted and the targeted field leaked, both asserts would still pass as long
  as any `REDACTED` sentinel appears anywhere in the bytes.
- **SOTA test:** Parse the log JSON (the same format `hugit log` emits) and
  assert the specific field (e.g. `principal_chain[0]`) equals the
  `REDACTED` sentinel verbatim, not just that the sentinel is present.

#### T-7: Source-invariant test is textual grep
- **File:** `crates/hugit-cli/tests/acceptance_wave_m_readpath.rs:252–300`
  (`canonical_log_loaders_route_through_the_chokepoint`)
- **Why weak:** This test greps source files for the string `verify_chain(`
  and `.push_record(` to assert that all log-reading code paths route through
  the chokepoint. This proves text, not behavior. A rename of `verify_chain`
  to `check_chain` would make the test vacuously pass (the grep finds zero
  calls, so the "assert callers exist" condition is never violated — or the
  test skips silently). A behavioral mutation to bypass the call at runtime
  would also not be caught.
- **SOTA test:** Instrument `verify_chain` with a call counter (a
  `std::sync::atomic::AtomicUsize`) and assert the counter increments for
  every read verb under test. This proves the chokepoint is *called*, not just
  that its name appears in source.

---

### P3 — Nits / enhancement opportunities

#### T-8: No fuzz or property tests anywhere in workspace
- **Scope:** Zero usage of `proptest`, `quickcheck`, or `cargo fuzz` across
  all 21 test directories.
- **Why it matters:** The scrubber (`hugit-ledger/src/redact.rs`), the entropy
  function (`shannon_entropy`), and `is_safe_identifier_shape` are
  security-critical and are only tested with hand-crafted fixtures. A fuzz run
  over `apply(random_str)` would find: panic paths, threshold edge cases,
  false-positive regressions on benign identifiers, and UTF-8 boundary issues.
  The memo key formula (`compute_memo_key`) would benefit from a property test
  asserting that any change to any of the three axes produces a distinct key
  (collision resistance at the property level, not just with fixed fixtures).
- **SOTA:** Add `proptest` to `hugit-ledger` dev-deps; write a property test
  that `apply` never panics on arbitrary UTF-8 and that `is_structural_secret`
  is stable (same input → same output). Add a property test in
  `hugit-refstore` that `compute_memo_key(a,b,c) != compute_memo_key(a',b,c)`
  whenever `a != a'` (and analogously for b, c).

#### T-9: `has_keyword_context_secret` whitespace path untested at shared-primitive level
- **File:** `crates/hugit-ledger/src/secret_shape.rs` (tests module)
- **Why it matters:** The `tests` block covers `structural_secret_classes`,
  `sk_key_gate`, `content_address_shapes`, `ulid_and_entropy`, and
  `safe_identifier_shape_splits_addresses_from_blobs`. It does not directly
  test `has_keyword_context_secret` with the whitespace-around-separator
  variant (`password = value` with spaces). That path is only tested in
  `redact.rs`'s inline `tests` module, one abstraction layer up. A regression
  in the shared primitive would not be caught by the primitive's own tests.
- **SOTA fix:** Add a dedicated `keyword_context_whitespace_variants` test in
  `secret_shape.rs::tests` covering `password=value`, `password = value`,
  `password =value`, and `password= value` — asserting `true` for all four —
  plus the negative `notapassword=value` asserting `false`.

---

## Strongest suites (honest)

These suites are genuinely load-bearing and would catch regressions in the
invariants they cover:

| Suite | Invariant covered |
|---|---|
| `crates/hugit-cli/tests/acceptance_wj_matrix.rs` | Exhaustive per-verb × per-secret-class matrix against real binary; 10 secret classes × all identifier fields |
| `crates/hugit-refstore/tests/canonical_format_pin.rs` | Cross-crate hash formula + memo key pinned to independent Python reference; `canonical_json` sort + framing proven |
| `crates/hugit-cli/tests/acceptance_k_verdict.rs` | Reject-laundering closed (K-VERDICT A/B/C/D) against real binary |
| `crates/hugit-cli/tests/acceptance_wave_m_readpath.rs` | All read verbs fail-closed on tampered logs; chain tamper repro |
| `crates/hugit-cli/tests/acceptance_round8_readpath.rs` | Chain tamper → explicit binary-level repro |
| `crates/hugit-ledger/tests/acceptance_wi_proven2.rs` | Proven/rejected mutual exclusion; multi-revision invariant in-process |
| `crates/hugit-refstore/tests/acceptance_d14.rs` | Authz matrix golden-tested + wired onto mutation primitive (WA2); denial audit proven |
| `crates/hugit-mirror/tests/acceptance_e1a.rs` | Real bare git repo (RealGitMirror) end-to-end push/verify |
| `crates/hugit-checks/tests/acceptance_b2a.rs` | Three-axis memo key sensitivity proven (all three axes required) |
| `crates/hugit-cli/tests/acceptance_wg_scrub.rs` | Scrub-on-append for all verb/field vectors against real binary |

---

## Summary

The test corpus is **not theater at the spine**: hash chain integrity,
verdict reject-stickiness, authz matrix, memo-key sensitivity, and
scrub-on-append are all covered by acceptance tests against the real binary or
in-process primitives. The corpus earns its green count.

However, **three P1 gaps exist** that could mask shipped defects:

1. **T-1 (tautological):** The shadow scheduler tests exercise zero
   computation — passing unconditionally whether the scheduler is correct or
   deleted.
2. **T-2 (policy divergence):** The policy gate and the ledger engine's
   scrubber have diverged silently. No test asserts parity. A credential class
   known to the ledger engine (e.g. `clp_` prefix) passes the policy gate.
3. **T-3 (env mutation):** Global env mutation in parallel test binaries is a
   latent flake and a source of false-pass results under concurrent load.

P2 findings (T-4 through T-7) represent meaningful coverage gaps that a
fresh adversarial round (Round 8) could exploit: `AcError::Busy` untested
end-to-end, scratch dir race in the largest matrix suite, field-level
specificity missing from scrub assertions, and a behavioral chokepoint test
replaced by a fragile textual grep.

The absence of any fuzz or property testing (T-8) is the single highest
leverage opportunity: the scrubber is security-critical and
hand-crafted-fixture-only coverage leaves the entropy and shape boundaries
untested against adversarial input.
