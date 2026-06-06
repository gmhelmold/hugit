#!/usr/bin/env bash
# WP-D10 acceptance suite — why + impact.
# Contract: docs/plan/wp-contracts/WP-D10.md
# Owned items (verbatim):
#   ① hugit why <line|symbol> → originating intent + charter/author/model/cost, matching event log
#   ② hugit impact <path|change> → golden affected-set on known build graph
#   ③ impact feeds verdict-panel ground truth (cross-check)
#   ④ (R6) why on regenerated/derived bytes resolves honestly to the regen/derivation event — never fabricates or mis-attributes a human author
# Oracle: crates/hugit-cli/tests/acceptance_d10.rs
#   Tests: item_1_why_matches_event_log, item_2_impact_golden_affected_set,
#          item_3_impact_feeds_verdict_ground_truth, item_4_why_derived_bytes_honest

SUITE_ID="wp-d10"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-cli

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-cli crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: owned module paths exist ─────────────────────────────────────
check "why/ module present under src/" \
  test -d "$CRATE/src/why"
check "impact/ module present under src/" \
  test -d "$CRATE/src/impact"

# ── structural: sub-modules expected by contract ─────────────────────────────
check "why/resolver module present" \
  bash -c "test -f $CRATE/src/why/resolver.rs || test -d $CRATE/src/why/resolver"
check "impact/blast_radius module present" \
  bash -c "test -f $CRATE/src/impact/blast_radius.rs || test -d $CRATE/src/impact/blast_radius"
check "impact/ground_truth_export module present" \
  bash -c "test -f $CRATE/src/impact/ground_truth_export.rs || test -d $CRATE/src/impact/ground_truth_export"

# ── structural: fixtures committed ───────────────────────────────────────────
# Golden affected-set and why-attribution are fixture proofs over the
# refstore/intent substrate on main (not live queries).
check "golden affected-set fixture committed" \
  bash -c "find $CRATE/src/impact -name '*golden*' 2>/dev/null | grep -q . \
        || find $CRATE/src/impact/tests -name '*golden*' 2>/dev/null | grep -q ."
check "derived-bytes fixture committed" \
  bash -c "find $CRATE/src/why -name '*derived*' 2>/dev/null | grep -q . \
        || find $CRATE/src/why/tests -name '*derived*' 2>/dev/null | grep -q ."
check "event-log fixture committed" \
  bash -c "find $CRATE/src/why -name '*event*log*' 2>/dev/null | grep -q . \
        || find $CRATE/src/why -name '*fixture*' 2>/dev/null | grep -q ."

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d10.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d10.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
export TESTLIST
TESTLIST="$(cargo test -p hugit-cli --test acceptance_d10 -- --list 2>/dev/null || true)"

check "① item_1_why_matches_event_log declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1_why_matches_event_log'"
check "② item_2_impact_golden_affected_set declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2_impact_golden_affected_set'"
check "③ item_3_impact_feeds_verdict_ground_truth declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_3_impact_feeds_verdict_ground_truth'"
check "④ item_4_why_derived_bytes_honest declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_4_why_derived_bytes_honest'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "①–④ all items (cargo test -p hugit-cli --test acceptance_d10)" \
  cargo test -p hugit-cli --test acceptance_d10

finish
