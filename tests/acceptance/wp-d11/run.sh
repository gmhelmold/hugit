#!/usr/bin/env bash
# WP-D11 acceptance suite — journals + resume.
# Contract: docs/plan/wp-contracts/WP-D11.md
# Owned items (verbatim):
#   ① journal persisted as tenant-private object bound to ws/intent
#   ② post-crash ctx resume reconstructs session within supported horizon
#   ③ beyond-horizon resume refused/degraded as documented
# Oracle: crates/hugit-ledger/tests/acceptance_d11.rs
#   Tests: item_1_journal_tenant_private_bound, item_2_resume_within_horizon,
#          item_3_beyond_horizon_refused_or_degraded

SUITE_ID="wp-d11"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-ledger

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-ledger crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: owned module path exists ─────────────────────────────────────
check "journal/ module present under src/" \
  test -d "$CRATE/src/journal"

# ── structural: sub-modules expected by contract ─────────────────────────────
check "journal/persist module present" \
  bash -c "test -f $CRATE/src/journal/persist.rs || test -d $CRATE/src/journal/persist"
check "journal/resume module present" \
  bash -c "test -f $CRATE/src/journal/resume.rs || test -d $CRATE/src/journal/resume"
check "journal/horizon module present" \
  bash -c "test -f $CRATE/src/journal/horizon.rs || test -d $CRATE/src/journal/horizon"

# ── structural: fixtures committed ───────────────────────────────────────────
check "within-horizon crash fixture committed" \
  bash -c "find $CRATE/src/journal -name '*within*horizon*' 2>/dev/null | grep -q . \
        || find $CRATE/src/journal -name '*fixture*' 2>/dev/null | grep -q ."
check "beyond-horizon fixture committed" \
  bash -c "find $CRATE/src/journal -name '*beyond*horizon*' 2>/dev/null | grep -q . \
        || find $CRATE/src/journal -name '*horizon*' 2>/dev/null | grep -q ."

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d11.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d11.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
export TESTLIST
TESTLIST="$(cargo test -p hugit-ledger --test acceptance_d11 -- --list 2>/dev/null || true)"

check "① item_1_journal_tenant_private_bound declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1_journal_tenant_private_bound'"
check "② item_2_resume_within_horizon declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2_resume_within_horizon'"
check "③ item_3_beyond_horizon_refused_or_degraded declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_3_beyond_horizon_refused_or_degraded'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "①–③ all items (cargo test -p hugit-ledger --test acceptance_d11)" \
  cargo test -p hugit-ledger --test acceptance_d11

finish
