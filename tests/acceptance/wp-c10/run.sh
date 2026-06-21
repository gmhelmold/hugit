#!/usr/bin/env bash
# WP-C10 acceptance suite — pricing no-shock guard (items ① ②).
# Acceptance harness for WP-C10.
# Owned items:
#   ① driving a tenant to budget exhaustion on EACH metered surface
#      (runner minutes, shadow spend, storage) → system caps/degrades
#      (pauses or falls back) with pre-exhaustion warning
#   ② zero overage charge generated — flat means flat (billing fixture assert)
# Oracle: crates/hugit-queue/tests/acceptance_c10.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-queue/budget/no_shock/ — disjoint from budget/ core modules.
# All checks are local/fixture-only (no box required).
# RED on current tree (no_shock/ subtree not yet built).

SUITE_ID="wp-c10"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-queue"
CRATE_NAME="hugit-queue"
NO_SHOCK_DIR="$CRATE_ROOT/budget/no_shock"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c10.rs"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-queue crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-queue Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-queue is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-queue"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "① no_shock/ subtree present (Claims: budget/no_shock/)" \
  test -d "$NO_SHOCK_DIR"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_c10.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_c10 -- --list 2>/dev/null || true)"

# ── ① runner-minutes surface: item_1_runner_minutes_cap_degrade ──────────────
check "① item_1_runner_minutes_cap_degrade declared in acceptance_c10" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_runner_minutes_cap_degrade'"

# ── ① shadow-spend surface: item_1_shadow_spend_cap_degrade ──────────────────
check "① item_1_shadow_spend_cap_degrade declared in acceptance_c10" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_shadow_spend_cap_degrade'"

# ── ① storage surface: item_1_storage_cap_degrade ────────────────────────────
check "① item_1_storage_cap_degrade declared in acceptance_c10" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_storage_cap_degrade'"

# ── ① pre-exhaustion warning emitted (all three surfaces) ────────────────────
check "① item_1_pre_exhaustion_warning_emitted declared in acceptance_c10" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_pre_exhaustion_warning'"

# ── ② zero-overage billing fixture: item_2_zero_overage_flat_means_flat ───────
check "② item_2_zero_overage_flat_means_flat declared in acceptance_c10" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_zero_overage'"

# ── structural: no_shock/ has cap/degrade guard referenced ───────────────────
check "① cap/degrade guard present in no_shock/ src" \
  bash -c "grep -rEq 'cap|degrade|pause|fallback|fall_back' '$NO_SHOCK_DIR/' 2>/dev/null"

# ── structural: pre-exhaustion warning referenced ────────────────────────────
check "① pre-exhaustion warning referenced in no_shock/ src" \
  bash -c "grep -rEq 'warn|Warning|pre_exhaust|PreExhaust|threshold' '$NO_SHOCK_DIR/' 2>/dev/null"

# ── structural: zero-overage / billing fixture referenced ────────────────────
check "② zero overage / billing fixture referenced in no_shock/ src" \
  bash -c "grep -rEq 'overage|Overage|zero_overage|billing|flat' '$NO_SHOCK_DIR/' 2>/dev/null"

# ── structural: Claims disjointness — no_shock/ must not write to budget core ──
check "② no_shock/ does not bleed into budget/ core (disjoint claims)" \
  bash -c "! grep -rEq 'mod core|mod engine|BudgetManager' '$NO_SHOCK_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "①② cargo test -p hugit-queue --test acceptance_c10 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c10

finish
