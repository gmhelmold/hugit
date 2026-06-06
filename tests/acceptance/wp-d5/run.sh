#!/usr/bin/env bash
# WP-D5 acceptance suite — ledger + watch + fleet.
# Contract: docs/plan/wp-contracts/WP-D5.md
# Owned items (verbatim from decomposition v2.0 — D5①–⑥):
#   ① asked→done→proven per campaign
#   ② watch: EventRecord-to-display p95 <2s (measured per event class)
#   ③ deep-links resolve to golden expected targets (not just non-error)
#   ④ planted secret renders REDACTED in ledger/verdict views
#   ⑤ two-zoom toggle: intent view ⇄ raw-commit view mutually consistent over the same fixture
#   ⑥ hugit fleet emits documented machine-readable schema reflecting true ws/agent state vs fixture
# Oracle: crates/hugit-ledger/tests/acceptance_d5.rs
#   Tests: item_1_asked_done_proven_per_campaign,
#          item_2_watch_latency_p95_under_2s,
#          item_3_deep_links_resolve_to_golden_targets,
#          item_4_planted_secret_renders_redacted,
#          item_5_two_zoom_toggle_mutually_consistent,
#          item_6_fleet_emits_valid_schema_vs_fixture

SUITE_ID="wp-d5"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-ledger

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-ledger crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: owned module paths exist ─────────────────────────────────────
check "ledger/ module present under src/" \
  test -d "$CRATE/src/ledger"
check "watch/ module present under src/" \
  test -d "$CRATE/src/watch"
check "fleet/ module present under src/" \
  test -d "$CRATE/src/fleet"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d5.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d5.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p hugit-ledger --test acceptance_d5 -- --list 2>/dev/null || true)"
export TESTLIST

check "① item_1_asked_done_proven_per_campaign declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1_asked_done_proven_per_campaign'"
check "② item_2_watch_latency_p95_under_2s declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2_watch_latency_p95_under_2s'"
check "③ item_3_deep_links_resolve_to_golden_targets declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_3_deep_links_resolve_to_golden_targets'"
check "④ item_4_planted_secret_renders_redacted declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_4_planted_secret_renders_redacted'"
check "⑤ item_5_two_zoom_toggle_mutually_consistent declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_5_two_zoom_toggle_mutually_consistent'"
check "⑥ item_6_fleet_emits_valid_schema_vs_fixture declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_6_fleet_emits_valid_schema_vs_fixture'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "D5①–⑥ all items (cargo test -p hugit-ledger --test acceptance_d5)" \
  cargo test -p hugit-ledger --test acceptance_d5

finish
