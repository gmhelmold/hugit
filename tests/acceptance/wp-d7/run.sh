#!/usr/bin/env bash
# WP-D7 acceptance suite — verdict panels + review Q&A.
# Acceptance harness for WP-D7.
# Owned items (verbatim):
#   ① lenses isolated (prompt audit)
#   ② valid VerdictObject[] + evidence refs
#   ③ planted bug of a NON-author-visible class (semantic/logic) caught by ≥1 lens
#   ④ (R2) human review Q&A: answers = citations to real evidence objects; no grounding → explicit refusal
#   ⑤ (R3) DIVERSITY enforced: homogeneous panel rejected/flagged; real panel dispatches distinct prompts + ≥2 distinct models
#   ⑥ (R3) no-self-defense negative: planted persuasive false self-justification is unreachable; verdict identical with vs without it
# Oracle: crates/hugit-cli/tests/acceptance_d7.rs
#   Tests: item_1_lenses_isolated, item_2_verdict_object_valid, item_3_planted_bug_caught,
#          item_4_qa_grounded_or_refused, item_5_diversity_enforced, item_6_no_self_defense

SUITE_ID="wp-d7"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-cli

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-cli crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: owned module path exists ─────────────────────────────────────
check "verdict/ module present under src/" \
  test -d "$CRATE/src/verdict"

# ── structural: sub-modules expected by contract ─────────────────────────────
check "verdict/lens_audit module present" \
  bash -c "test -f $CRATE/src/verdict/lens_audit.rs || test -d $CRATE/src/verdict/lens_audit"
check "verdict/diversity module present" \
  bash -c "test -f $CRATE/src/verdict/diversity.rs || test -d $CRATE/src/verdict/diversity"
check "verdict/panel_dispatch module present" \
  bash -c "test -f $CRATE/src/verdict/panel_dispatch.rs || test -d $CRATE/src/verdict/panel_dispatch"
check "verdict/qa module present" \
  bash -c "test -f $CRATE/src/verdict/qa.rs || test -d $CRATE/src/verdict/qa"

# ── structural: fixtures committed ───────────────────────────────────────────
# Items ①⑤⑥ are structural proofs over VerdictObject fixtures + panel-dispatch
# logic (local; no live model calls in the acceptance lane — lens isolation and
# prompt audit are fixture asserts, not live calls).
check "lens-isolation fixture committed" \
  bash -c "find $CRATE/src/verdict -name '*lens*isolation*' 2>/dev/null | grep -q ."
check "planted-bug fixture committed" \
  bash -c "find $CRATE/src/verdict -name '*planted*bug*' 2>/dev/null | grep -q ."
check "persuasion-channel fixture committed" \
  bash -c "find $CRATE/src/verdict -name '*persuasion*' 2>/dev/null | grep -q ."

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d7.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d7.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
export TESTLIST
TESTLIST="$(cargo test -p hugit-cli --test acceptance_d7 -- --list 2>/dev/null || true)"

check "① item_1_lenses_isolated declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1_lenses_isolated'"
check "② item_2_verdict_object_valid declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2_verdict_object_valid'"
check "③ item_3_planted_bug_caught declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_3_planted_bug_caught'"
check "④ item_4_qa_grounded_or_refused declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_4_qa_grounded_or_refused'"
check "⑤ item_5_diversity_enforced declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_5_diversity_enforced'"
check "⑥ item_6_no_self_defense declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_6_no_self_defense'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "①–⑥ all items (cargo test -p hugit-cli --test acceptance_d7)" \
  cargo test -p hugit-cli --test acceptance_d7

finish
