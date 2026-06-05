#!/usr/bin/env bash
# WP-D6 acceptance suite — hugit-policy engine (gates, fail-closed, audit).
# Contract: docs/plan/wp-contracts/WP-D6.md
# Owned items (verbatim):
#   ① 3 ported gates local≡forge
#   ② engine down→landing blocks (kill-test)
#   ③ policy change = audited event
# Oracle: crates/hugit-policy/tests/acceptance_d6.rs
#   Tests: item_1_gates_local_eq_forge, item_2_engine_down_blocks, item_3_policy_change_audited

SUITE_ID="wp-d6"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-policy

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-policy crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: tests/ dir exists ────────────────────────────────────────────
check "hugit-policy/tests/ present" \
  test -d "$CRATE/tests"

# ── negative: no writes to refstore/queue/ledger/contracts (leak guard) ──────
check "no refstore writes (leak guard)" \
  bash -c "! test -e crates/hugit-refstore/src/policy"
check "no queue writes (leak guard)" \
  bash -c "! test -e crates/hugit-queue/src/policy"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d6.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d6.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p hugit-policy --test acceptance_d6 -- --list 2>/dev/null || true)"

check "① item_1_gates_local_eq_forge declared" \
  bash -c "echo '$TESTLIST' | grep -q 'item_1_gates_local_eq_forge'"
check "② item_2_engine_down_blocks declared" \
  bash -c "echo '$TESTLIST' | grep -q 'item_2_engine_down_blocks'"
check "③ item_3_policy_change_audited declared" \
  bash -c "echo '$TESTLIST' | grep -q 'item_3_policy_change_audited'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "①②③ all owned items (cargo test -p hugit-policy --test acceptance_d6)" \
  cargo test -p hugit-policy --test acceptance_d6

finish
