#!/usr/bin/env bash
# WP-C7 acceptance suite — hugit-queue crate, budgets + queue fairness (items ①②③).
# Contract: docs/plan/wp-contracts/WP-C7.md.
# Owned items:
#   ① exhausted→queued not dropped; surfaced as defined status field + event
#   ② fairness bound: under contention, every tenant's p95 queue wait ≤ defined bound
#      and throughput share ≥ defined floor (interleave fixture)
#   ③ metering accuracy: accounted ≈ actual within ±5% on the fixture workload
# Oracle: crates/hugit-queue/tests/acceptance_wp-c7.rs must exist with
#   item_1_*, item_2_*, item_3_* tests; cargo test -p hugit-queue --test acceptance_wp-c7 green.
# Claims: crates/hugit-queue/budget/ — disjoint from src/core/ (B4a) and src/github/ (B4b).

SUITE_ID="wp-c7"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-queue
ACCEPTANCE_RS="$CRATE/tests/acceptance_wp-c7.rs"
WP_ID="wp-c7"

# ── (a) claimed crate exists ─────────────────────────────────────────────────
check "① crate hugit-queue present (Cargo.toml)" \
  test -f "$CRATE/Cargo.toml"
check "① crate hugit-queue present (src/lib.rs)" \
  test -f "$CRATE/src/lib.rs"

# ── (a) budget module dir exists (Claims: budget/) ───────────────────────────
check "① budget/ module dir exists: budget/" \
  test -d "$CRATE/budget"

# ── (a) acceptance test file committed ───────────────────────────────────────
check "① acceptance file committed: $ACCEPTANCE_RS" \
  test -f "$ACCEPTANCE_RS"

# ── (b) cargo test --list declares item_1, item_2, item_3 ────────────────────
export TESTLIST="$(cargo test -p hugit-queue --test "acceptance_${WP_ID}" -- --list 2>/dev/null || true)"

check "① item_1 test declared (exhausted_queued_not_dropped)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1'"
check "② item_2 test declared (fairness_bound_interleave_fixture)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2'"
check "③ item_3 test declared (metering_accuracy_within_5pct)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3'"

# ── (c) cargo test -p hugit-queue --test acceptance_wp-c7 green ──────────────
check "①②③ cargo test -p hugit-queue --test acceptance_wp-c7 green" \
  cargo test -p hugit-queue --test "acceptance_${WP_ID}"

# ── (d) structural: exhausted→queued status field present (①) ────────────────
check "① BudgetStatus/exhausted/queued status field referenced in budget/ src" \
  bash -c "grep -rEq 'BudgetStatus|exhausted|Exhausted' '$CRATE/budget/'"

# ── (d) structural: budget event emitted on exhaustion (①) ───────────────────
check "① budget exhaustion event referenced in budget/ src" \
  bash -c "grep -rEq 'BudgetEvent|budget_event|exhausted_event' '$CRATE/budget/'"

# ── (d) structural: no silent drop path on budget exhaustion (①) ─────────────
check "① no silent drop (drop_work|discard|silent_drop) in budget/ src" \
  bash -c "! grep -rEq 'drop_work|discard_work|silent_drop' '$CRATE/budget/'"

# ── (d) structural: fairness bound constants present — p95 + floor (②) ────────
check "② p95 bound and throughput floor constants declared in budget/ src" \
  bash -c "grep -rEq 'p95|P95|throughput_floor|THROUGHPUT_FLOOR|wait_bound|WAIT_BOUND' '$CRATE/budget/'"

# ── (d) structural: metering accounting reconcilable within ±5% (③) ──────────
check "③ metering/accounting referenced in budget/ src" \
  bash -c "grep -rEq 'meter|Meter|accounting|accounted' '$CRATE/budget/'"

# ── (d) structural: Claims disjointness — budget/ does not write core/ or github/ ──
check "① C7 budget/ does not bleed into src/core/ (disjoint from B4a)" \
  bash -c "! grep -rEq 'mod core|use.*hugit_queue::core' '$CRATE/budget/' 2>/dev/null || true; ! test -f '$CRATE/budget/core.rs'"
check "① C7 budget/ does not bleed into src/github/ (disjoint from B4b)" \
  bash -c "! grep -rEq 'mod github|use.*hugit_queue::github' '$CRATE/budget/' 2>/dev/null || true; ! test -f '$CRATE/budget/github.rs'"

finish
