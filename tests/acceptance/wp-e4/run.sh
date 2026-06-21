#!/usr/bin/env bash
# WP-E4 acceptance suite — Actions-YAML compatibility shim (items ① ② ③ ④).
# Acceptance harness for WP-E4.
# Owned items:
#   ① the SUPPORTED subset is a published contract; "supported" = proven-to-execute,
#      not merely documented
#   ② outside the contract → explicit actionable report — falsifiable boundary,
#      no silent skip
#   ③(+) missing/denied secret → fail CLOSED w/ named secret; material never
#        in logs/env (red-team)
#   ④ execution EQUIVALENCE: a DETERMINISTIC fixture workflow runs on real GitHub
#      Actions AND on the shim → equivalent observable outcomes
#      (steps, env, artifacts, exit states)
# Oracle: crates/hugit-runner/tests/acceptance_e4.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-runner/src/shim/ — disjoint from concurrency/, expiry/, etc.
# Box env: HUGIT_RUNNER_HOST=203.0.113.10 (exported before cargo invocations);
#   box-dependent tests FAIL — not skip — when env is set but box unreachable.
# GitHub-Actions-equivalence item reads HUGIT_GH_TEST_REPO
#   (suite exports humangr-labs/hugit-fleet-syn-1);
#   equivalence check FAILs when HUGIT_GH_TEST_REPO is needed but empty.
# RED on current tree (shim/ subtree not yet built).

SUITE_ID="wp-e4"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-runner"
CRATE_NAME="hugit-runner"
SHIM_DIR="$CRATE_ROOT/src/shim"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e4.rs"
SUPPORTED_SUBSET_DOC="crates/hugit-runner/src/shim/supported-subset.md"

export HUGIT_RUNNER_HOST=203.0.113.10
export HUGIT_GH_TEST_REPO="humangr-labs/hugit-fleet-syn-1"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-runner crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-runner Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-runner is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-runner"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "① shim/ subtree present (Claims: src/shim/)" \
  test -d "$SHIM_DIR"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_e4.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── barrel export: shim module exported from lib.rs ──────────────────────────
check "① lib.rs barrel exports shim module" \
  bash -c "grep -q 'shim' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

# ── published supported-subset doc committed ──────────────────────────────────
check "① published supported-subset contract doc committed ($SUPPORTED_SUBSET_DOC)" \
  test -f "$SUPPORTED_SUBSET_DOC"

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_e4 -- --list 2>/dev/null || true)"

# ── ① published supported-subset proven-to-execute ───────────────────────────
check "① item_1_supported_subset_proven_to_execute declared in acceptance_e4" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_supported_subset_proven_to_execute'"

# ── ② outside contract → explicit actionable report ──────────────────────────
check "② item_2_out_of_contract_actionable_report declared in acceptance_e4" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_out_of_contract_actionable_report'"

# ── ③ secrets fail CLOSED, material never in logs/env ────────────────────────
check "③ item_3_secrets_fail_closed_named declared in acceptance_e4" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_secrets_fail_closed_named'"

# ── ③ red-team: secret material never in logs or env ─────────────────────────
check "③ item_3_secrets_not_in_logs_env declared in acceptance_e4" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_secrets_not_in_logs_env'"

# ── ④ equivalence: deterministic fixture runs same on Actions and shim ────────
check "④ item_4_equivalence_deterministic_fixture declared in acceptance_e4" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_equivalence_deterministic_fixture'"

# ── ④ HUGIT_GH_TEST_REPO must be non-empty for equivalence item ───────────────
check "④ HUGIT_GH_TEST_REPO env set to non-empty value (equivalence gate)" \
  bash -c "test -n \"\$HUGIT_GH_TEST_REPO\""

# ── structural: shim/ has YAML parser referenced ─────────────────────────────
check "① YAML parser referenced in shim/ src" \
  bash -c "grep -rEq 'yaml|Yaml|YAML|parse_workflow|workflow_file' '$SHIM_DIR/' 2>/dev/null"

# ── structural: supported-subset contract referenced in shim/ ────────────────
check "① supported subset / contract boundary referenced in shim/ src" \
  bash -c "grep -rEq 'supported|Supported|contract|Contract|subset' '$SHIM_DIR/' 2>/dev/null"

# ── structural: out-of-contract report is explicit and actionable ─────────────
check "② actionable out-of-contract report referenced in shim/ src" \
  bash -c "grep -rEq 'out_of_contract|OutOfContract|unsupported_report|actionable' '$SHIM_DIR/' 2>/dev/null"

# ── structural: no silent skip on unsupported construct ───────────────────────
check "② no silent skip path present in shim/ src" \
  bash -c "! grep -rEq 'silent_skip|skip_silently|ignore_unsupported' '$SHIM_DIR/' 2>/dev/null"

# ── structural: secrets resolved via broker only ─────────────────────────────
check "③ secrets broker resolution referenced in shim/ src (C5 channel)" \
  bash -c "grep -rEq 'broker|Broker|FenceManifest|secret.*broker|resolve_secret' '$SHIM_DIR/' 2>/dev/null"

# ── structural: fail CLOSED on missing/denied secret ─────────────────────────
check "③ fail-CLOSED / named-secret pattern referenced in shim/ src" \
  bash -c "grep -rEq 'fail_closed|FailClosed|SecretDenied|missing_secret|named.*secret' '$SHIM_DIR/' 2>/dev/null"

# ── structural: determinism precondition stated for equivalence fixture ───────
check "④ determinism precondition pinned in shim/ src or acceptance_e4" \
  bash -c "grep -rEq 'determinism|deterministic|pinned|pin_toolchain|no_nondeterminism' \
    '$SHIM_DIR/' '$ACCEPTANCE_FILE' 2>/dev/null"

# ── structural: Claims disjointness — shim/ must not touch concurrency/ or expiry/ ──
check "① shim/ does not bleed into concurrency/ (disjoint from C2b)" \
  bash -c "! grep -rEq 'mod concurrency|use.*hugit_runner::concurrency' '$SHIM_DIR/' 2>/dev/null"

check "① shim/ does not bleed into expiry/ (disjoint from C2b)" \
  bash -c "! grep -rEq 'mod expiry|use.*hugit_runner::expiry' '$SHIM_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "①②③④ cargo test -p hugit-runner --test acceptance_e4 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e4

finish
