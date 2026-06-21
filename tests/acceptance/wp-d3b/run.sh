#!/usr/bin/env bash
# WP-D3b acceptance suite — push concurrency + total order + external-change
#                            + flag/negatives.
# Acceptance harness for WP-D3b.
# Owned items (verbatim from decomposition v2.0 — D3b):
#   ② concurrent pushes: total order, correct stale rejection
#   ③ raw push = external-change w/ attribution
#   ④ flag off unless self-hosted-alpha
#   ⑤(+) negative: NO synthetic intent fabricated for a raw push (intent log clean)
# Claims: crates/hugit-proto/src/write/{order,external,flag}/
#         crates/hugit-proto/tests/push_concurrency_negatives/
# Oracle: crates/hugit-proto/tests/acceptance_d3b.rs
#   one #[test] item_<n>_<slug> per owned item:
#     item_2_concurrent_pushes_total_order_stale_rejection
#     item_3_raw_push_external_change_with_attribution
#     item_4_flag_off_unless_self_hosted_alpha
#     item_5_no_synthetic_intent_for_raw_push
# Box env: HUGIT_RUNNER_HOST=203.0.113.10 (box-dependent tests FAIL when env
#   set but box unreachable; skip only when HUGIT_RUNNER_HOST entirely unset).
# RED on current tree (crate paths not yet built).

SUITE_ID="wp-d3b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=203.0.113.10

CRATE_ROOT="crates/hugit-proto"
CRATE_NAME="hugit-proto"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_d3b.rs"

# ── structural: crate present ────────────────────────────────────────────────
check "hugit-proto crate present" \
  bash -c "test -f $CRATE_ROOT/Cargo.toml && test -f $CRATE_ROOT/src/lib.rs"

check "hugit-proto is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-proto"'

# ── structural: owned module paths exist ─────────────────────────────────────
check "write/order/ module present under src/" \
  test -d "$CRATE_ROOT/src/write/order"

check "write/external/ module present under src/" \
  test -d "$CRATE_ROOT/src/write/external"

check "write/flag/ module present under src/" \
  test -d "$CRATE_ROOT/src/write/flag"

# ── structural: owned test directory present ─────────────────────────────────
check "tests/push_concurrency_negatives/ directory present" \
  test -d "$CRATE_ROOT/tests/push_concurrency_negatives"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d3b.rs oracle committed" \
  test -f "$ACCEPTANCE_FILE"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_d3b -- --list 2>/dev/null || true)"
export TESTLIST

check "② item_2_concurrent_pushes_total_order_stale_rejection declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_concurrent_pushes_total_order_stale_rejection'"

check "③ item_3_raw_push_external_change_with_attribution declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_raw_push_external_change_with_attribution'"

check "④ item_4_flag_off_unless_self_hosted_alpha declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_flag_off_unless_self_hosted_alpha'"

check "⑤ item_5_no_synthetic_intent_for_raw_push declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5_no_synthetic_intent_for_raw_push'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "②③④⑤ acceptance suite green (cargo test -p hugit-proto --test acceptance_d3b)" \
  cargo test -p "$CRATE_NAME" --test acceptance_d3b

finish
