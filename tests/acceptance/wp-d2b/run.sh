#!/usr/bin/env bash
# WP-D2b acceptance suite — client matrix + jj stacks + CPU/chunked fallback
#                            + degradation kill-test + scale ceilings.
# Contract: docs/plan/wp-contracts/WP-D2b.md
# Owned items (verbatim from decomposition v2.0 — D2b):
#   ③ clients: git 2.40+/jj/libgit2
#   ④ 500MB fixture: per-request CPU-time p95 ≤70% of the platform per-request
#      CPU limit; beyond → chunked fallback path exercised+passing
#   ⑤ degradation kill-test: smart layers disabled (both steady-state AND
#      injected mid-operation) → vanilla git clone/fetch still serves valid repo
#   ⑥ scale ceilings defined+tested per dimension (repo size, ref count,
#      concurrent clients, pack size): at each limit → documented bounded
#      behavior, never silent failure
#   ⑦(R2) jj FIRST-CLASS: stacked-changes series round-trips via jj with
#      change-ids stable across forge ops; stack reconstructs identically
# Claims: crates/hugit-proto/src/read/{clients,fallback,limits}/
#         crates/hugit-proto/tests/clients_jj_limits/
# Oracle: crates/hugit-proto/tests/acceptance_d2b.rs
#   one #[test] item_<n>_<slug> per owned item:
#     item_3_client_matrix_git_jj_libgit2
#     item_4_cpu_budget_p95_chunked_fallback
#     item_5_degradation_kill_test
#     item_6_scale_ceilings_bounded_behavior
#     item_7_jj_stack_roundtrip_change_ids_stable
# Box env: HUGIT_RUNNER_HOST=203.0.113.10 (box-dependent tests FAIL when env
#   set but box unreachable; skip only when HUGIT_RUNNER_HOST entirely unset).
# jj/git clients: shelled to local binaries if available; FAIL if unavailable
#   (FAIL-not-skip per contract).
# RED on current tree (crate paths not yet built).

SUITE_ID="wp-d2b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=203.0.113.10

CRATE_ROOT="crates/hugit-proto"
CRATE_NAME="hugit-proto"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_d2b.rs"

# ── structural: crate present ────────────────────────────────────────────────
check "hugit-proto crate present" \
  bash -c "test -f $CRATE_ROOT/Cargo.toml && test -f $CRATE_ROOT/src/lib.rs"

check "hugit-proto is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-proto"'

# ── structural: owned module paths exist ─────────────────────────────────────
check "read/clients/ module present under src/" \
  test -d "$CRATE_ROOT/src/read/clients"

check "read/fallback/ module present under src/" \
  test -d "$CRATE_ROOT/src/read/fallback"

check "read/limits/ module present under src/" \
  test -d "$CRATE_ROOT/src/read/limits"

# ── structural: owned test directory present ─────────────────────────────────
check "tests/clients_jj_limits/ directory present" \
  test -d "$CRATE_ROOT/tests/clients_jj_limits"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d2b.rs oracle committed" \
  test -f "$ACCEPTANCE_FILE"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_d2b -- --list 2>/dev/null || true)"
export TESTLIST

check "③ item_3_client_matrix_git_jj_libgit2 declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_client_matrix_git_jj_libgit2'"

check "④ item_4_cpu_budget_p95_chunked_fallback declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_cpu_budget_p95_chunked_fallback'"

check "⑤ item_5_degradation_kill_test declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5_degradation_kill_test'"

check "⑥ item_6_scale_ceilings_bounded_behavior declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_scale_ceilings_bounded_behavior'"

check "⑦ item_7_jj_stack_roundtrip_change_ids_stable declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_7_jj_stack_roundtrip_change_ids_stable'"

# ── client availability: git 2.40+ ───────────────────────────────────────────
# Contract: shell to local git if available else FAIL (FAIL-not-skip).
check "③ git binary available (client matrix prerequisite)" \
  bash -c 'command -v git >/dev/null 2>&1'

check "③ git version ≥ 2.40 (client matrix prerequisite)" \
  bash -c 'ver=$(git --version 2>/dev/null | grep -o "[0-9]*\.[0-9]*" | head -1); major=${ver%%.*}; minor=${ver##*.}; [ "$major" -gt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -ge 40 ]; }'

# ── client availability: jj ──────────────────────────────────────────────────
# Contract: shell to local jj if available else FAIL (FAIL-not-skip).
check "⑦ jj binary available (jj first-class prerequisite)" \
  bash -c 'command -v jj >/dev/null 2>&1'

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "③④⑤⑥⑦ acceptance suite green (cargo test -p hugit-proto --test acceptance_d2b)" \
  cargo test -p "$CRATE_NAME" --test acceptance_d2b

finish
