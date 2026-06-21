#!/usr/bin/env bash
# WP-D9 acceptance suite — attention queue.
# Acceptance harness for WP-D9.
# Owned items (verbatim from decomposition v2.0 — D9①–⑤):
#   ① fixture with known policy/blast/confidence → documented composite
#      ordering reproduced
#   ② perturbing one input moves entry to expected position
#   ③ policy-mandatory items can never be ranked out of the human's view
#   ④(R2) fast-approve (90s) affordance is BLOCKED for high-risk/policy-mandatory
#      items — forced through full review; permitted for policy-low-risk
#   ⑤(R7) up-zoom under degradation: with ranking inputs (blast/confidence)
#      unavailable, policy-mandatory items STILL surface with an honest
#      "ranking degraded" state — the queue never goes silently dark on items
#      that must reach a human
# Claims: crates/hugit-cli/attention/ (entire module + tests/)
# Oracle: crates/hugit-cli/attention/tests/acceptance_d9.rs
#   one #[test] item_<n>_<slug> per owned item:
#     item_1_composite_ordering_reproduced
#     item_2_perturbation_moves_entry
#     item_3_mandatory_floor_never_ranked_out
#     item_4_fast_approve_blocked_high_risk_mandatory_permitted_low_risk
#     item_5_degradation_mandatory_still_surface_labeled
# RED on current tree (crate paths not yet built).

SUITE_ID="wp-d9"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-cli"
CRATE_NAME="hugit-cli"
MODULE_ROOT="$CRATE_ROOT/attention"
ACCEPTANCE_FILE="$CRATE_ROOT/attention/tests/acceptance_d9.rs"

# ── structural: crate present ────────────────────────────────────────────────
check "hugit-cli crate present" \
  bash -c "test -f $CRATE_ROOT/Cargo.toml && test -f $CRATE_ROOT/src/lib.rs"

check "hugit-cli is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-cli"'

# ── structural: owned module path present ────────────────────────────────────
check "attention/ module present under hugit-cli/" \
  test -d "$MODULE_ROOT"

check "attention/tests/ directory present" \
  test -d "$CRATE_ROOT/attention/tests"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d9.rs oracle committed" \
  test -f "$ACCEPTANCE_FILE"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_d9 -- --list 2>/dev/null || true)"
export TESTLIST

check "① item_1_composite_ordering_reproduced declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_composite_ordering_reproduced'"

check "② item_2_perturbation_moves_entry declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_perturbation_moves_entry'"

check "③ item_3_mandatory_floor_never_ranked_out declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_mandatory_floor_never_ranked_out'"

check "④ item_4_fast_approve_blocked_high_risk_mandatory_permitted_low_risk declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_fast_approve_blocked_high_risk_mandatory_permitted_low_risk'"

check "⑤ item_5_degradation_mandatory_still_surface_labeled declared" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5_degradation_mandatory_still_surface_labeled'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "①②③④⑤ acceptance suite green (cargo test -p hugit-cli --test acceptance_d9)" \
  cargo test -p "$CRATE_NAME" --test acceptance_d9

finish
