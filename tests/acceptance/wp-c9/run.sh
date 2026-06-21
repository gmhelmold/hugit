#!/usr/bin/env bash
# WP-C9 acceptance suite — workspace lifecycle (spawn/attach/resume).
# Contract: docs/plan/wp-contracts/WP-C9.md
# Owned items:
#   ① attach joins live workspace (same fence/materialization) without respawn
#   ② resume restores state+fence; resumed ws cannot exceed original path_set
#   ③ spawn <1s; identical concurrent spawns dedup to one materialization
#   ④ local vs remote execution: identical observable results
# Claims: crates/hugit-runner/ws/
# Oracle: crates/hugit-runner/tests/acceptance_c9.rs
#   one #[test] item_<n>_<slug> per owned item.
# Box env: HUGIT_RUNNER_HOST=203.0.113.10 (exported before cargo invocations;
#   box-dependent tests FAIL — not skip — when env set but box unreachable).
#   Tests may skip only when HUGIT_RUNNER_HOST is entirely unset.
# RED on current tree (ws/ subtree not yet built).

SUITE_ID="wp-c9"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=203.0.113.10

CRATE_ROOT="crates/hugit-runner"
CRATE_NAME="hugit-runner"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c9.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-runner crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-runner Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-runner is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-runner"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_c9.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "ws/ subtree present" \
  test -d "$CRATE_ROOT/src/ws"

# ── structural negatives: sibling C2a/C2b/C3/E4 paths must be untouched ──────
# [concurrency/ belongs to C2b — no new files from C9] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [boot/ belongs to C3 — no new files from C9] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [shim/ belongs to E4 — no new files from C9] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# ── item declaration check via --list ────────────────────────────────────────
export TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_c9 -- --list 2>/dev/null || true)"

check "① item_1_attach_joins_live_workspace_no_respawn declared in acceptance_c9" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1'"

check "② item_2_resume_restores_state_fence_path_set_ceiling declared in acceptance_c9" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2'"

check "③ item_3_spawn_lt_1s_concurrent_dedup_one_materialization declared in acceptance_c9" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3'"

check "④ item_4_local_remote_identical_observable_results declared in acceptance_c9" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4'"

# ── structural/contract asserts on ws/ src ───────────────────────────────────

# ① attach without respawn: no re-hydrate/re-spawn on attach
check "① attach joins live workspace — no re-spawn on attach in ws/ src" bash -c \
  "grep -rEq 'attach|Attach|join_live|live_workspace' '$CRATE_ROOT/src/ws/'"

check "① attach does not trigger respawn — no respawn_on_attach path in ws/ src" bash -c \
  "! grep -rEq 'respawn_on_attach|re_spawn_attach|respawn.*attach' \
   '$CRATE_ROOT/src/ws/' 2>/dev/null"

# ② resume: state + fence restore; path_set ceiling enforced
check "② resume restores state+fence referenced in ws/ src" bash -c \
  "grep -rEq 'resume|Resume|restore_state|restore_fence|state_restore' \
   '$CRATE_ROOT/src/ws/'"

check "② FenceManifest / path_set ceiling on resume referenced in ws/ src" bash -c \
  "grep -rEq 'FenceManifest|path_set|PathSet|fence_ceiling|cannot_exceed' \
   '$CRATE_ROOT/src/ws/'"

check "② resume cannot widen path_set — ceiling enforced in ws/ src" bash -c \
  "! grep -rEq 'widen_path_set|expand_fence|fence_bypass_resume' \
   '$CRATE_ROOT/src/ws/' 2>/dev/null"

# ③ spawn <1s timing assertion + concurrent dedup to one materialization
check "③ spawn timing bound (<1s) referenced in ws/ src or tests" bash -c \
  "grep -rEq 'lt_1s|sub_second|spawn_ms|Duration.*1000|1_000.*ms|1s|1000ms' \
   '$CRATE_ROOT/src/ws/' '$CRATE_ROOT/tests/' 2>/dev/null"

check "③ concurrent spawn dedup to one materialization in ws/ src" bash -c \
  "grep -rEq 'dedup|dedup_spawn|concurrent_spawn|one_materialization|single_mat' \
   '$CRATE_ROOT/src/ws/'"

# ③ concurrent spawn test must use held commands so spawns overlap in time
check "③ held commands (sleep/join) used in concurrent-spawn fixture — overlap in time" bash -c \
  "grep -rEq 'sleep|join|tokio::join|spawn.*held|barrier|latch' \
   '$CRATE_ROOT/src/ws/' '$CRATE_ROOT/tests/' 2>/dev/null"

# ④ local ≡ remote: RunnerLease consumed; result-identity asserted
check "④ RunnerLease consumed in ws/ src (lifecycle runs under lease)" bash -c \
  "grep -rEq 'RunnerLease|runner_lease|lease' '$CRATE_ROOT/src/ws/'"

check "④ local/remote result-identity referenced in ws/ src or tests" bash -c \
  "grep -rEq 'local.*remote|remote.*local|result_identity|identical.*result|local_eq_remote' \
   '$CRATE_ROOT/src/ws/' '$CRATE_ROOT/tests/' 2>/dev/null"

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-runner --test acceptance_c9 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c9

finish
