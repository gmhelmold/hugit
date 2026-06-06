#!/usr/bin/env bash
# WP-D13 acceptance suite — tournament.
# Contract: docs/plan/wp-contracts/WP-D13.md
# Owned items:
#   ① -n N produces N independent candidates
#   ② judge panel selects per documented criteria (fixture w/ known-best)
#   ③ losers remain addressable as evidence
#   ④ (R7) budget-bounded fan-out: N is policy-capped; respects per-tenant caps + fairness (C7); ZERO overage under flat plan
# Claims: crates/hugit-cli/tournament/
# Oracle: crates/hugit-cli/tests/acceptance_d13.rs
#   one #[test] item_<n>_<slug> per owned item.
# Budget-bounded items ④ verified via cap-overrun fixture (local).
# RED on current tree (crate paths not yet built).

export TESTLIST="item_1_n_independent_candidates item_2_panel_selects_known_best item_3_losers_addressable_as_evidence item_4_budget_bounded_zero_overage"

SUITE_ID="wp-d13"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-cli"
CRATE_NAME="hugit-cli"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_d13.rs"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-cli crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-cli Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-cli is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-cli"'

# ── acceptance oracle file ────────────────────────────────────────────────────
check "acceptance_d13.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ─────────────────────────────────────────────────────────
check "tournament/ subtree present" \
  test -d "$CRATE_ROOT/src/tournament"

# ── ① -n N produces N independent candidates ─────────────────────────────────
# Fixture: N candidates — assert count equals N and no shared mutation.
check "① item_1_n_independent_candidates declared in acceptance_d13" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d13 -- --list 2>/dev/null | \
   grep -q "item_1_n_independent_candidates"'

printf '%s\n' "$TESTLIST" | grep -q item_1
check "① TESTLIST contains item_1_n_independent_candidates" \
  printf '%s\n' "$TESTLIST" | grep -q "item_1_n_independent_candidates"

# ── ② judge panel selects per documented criteria; known-best fixture ─────────
# Fixture with known-best candidate: assert panel picks it per written criteria.
check "② item_2_panel_selects_known_best declared in acceptance_d13" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d13 -- --list 2>/dev/null | \
   grep -q "item_2_panel_selects_known_best"'

printf '%s\n' "$TESTLIST" | grep -q item_2
check "② TESTLIST contains item_2_panel_selects_known_best" \
  printf '%s\n' "$TESTLIST" | grep -q "item_2_panel_selects_known_best"

# ── ③ losers remain addressable as evidence objects (not discarded) ───────────
# After selection: assert all losing candidates are resolvable as evidence.
check "③ item_3_losers_addressable_as_evidence declared in acceptance_d13" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d13 -- --list 2>/dev/null | \
   grep -q "item_3_losers_addressable_as_evidence"'

printf '%s\n' "$TESTLIST" | grep -q item_3
check "③ TESTLIST contains item_3_losers_addressable_as_evidence" \
  printf '%s\n' "$TESTLIST" | grep -q "item_3_losers_addressable_as_evidence"

# ── ④ budget-bounded fan-out: policy cap + zero overage ──────────────────────
# Fixture: cap-overrun attempt → assert capped/refused + zero overage charge.
check "④ item_4_budget_bounded_zero_overage declared in acceptance_d13" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d13 -- --list 2>/dev/null | \
   grep -q "item_4_budget_bounded_zero_overage"'

printf '%s\n' "$TESTLIST" | grep -q item_4
check "④ TESTLIST contains item_4_budget_bounded_zero_overage" \
  printf '%s\n' "$TESTLIST" | grep -q "item_4_budget_bounded_zero_overage"

# ── full acceptance suite green ───────────────────────────────────────────────
check "cargo test -p hugit-cli --test acceptance_d13 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_d13

finish
