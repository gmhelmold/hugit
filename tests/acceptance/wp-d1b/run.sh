#!/usr/bin/env bash
# WP-D1b acceptance suite — hugit-refstore compaction/cold-tier + recovery + undo.
# Contract: docs/plan/wp-contracts/WP-D1b.md
# Owned items (verbatim):
#   ③ compaction replay-equivalent, hot log bounded
#   ④ undo restores + preserves history
#   ⑥(+) recovery: hot-DO loss → full ref state rebuilt from cold tier (and/or mirror) replay-identical
# Oracle: crates/hugit-refstore/tests/acceptance_d1b.rs
#   Tests: item_3_compaction_replay_equivalent, item_4_undo_restores_preserves_history,
#          item_6_recovery_hot_do_loss_rebuild

SUITE_ID="wp-d1b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-refstore

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-refstore crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: D1b-owned module paths exist ─────────────────────────────────
check "compaction/ module present under src/" \
  test -d "$CRATE/src/compaction"
check "coldtier/ module present under src/" \
  test -d "$CRATE/src/coldtier"
check "recovery/ module present under src/" \
  test -d "$CRATE/src/recovery"
check "undo/ module present under src/" \
  test -d "$CRATE/src/undo"

# ── negative: D1a paths must not be modified by D1b (leak guard) ─────────────
check "log/ path not absent (D1a substrate must exist)" \
  test -d "$CRATE/src/log"
check "replay/ path not absent (D1a substrate must exist)" \
  test -d "$CRATE/src/replay"
check "tamper/ path not absent (D1a substrate must exist)" \
  test -d "$CRATE/src/tamper"

# ── negative: D1c path must NOT exist (leak guard) ───────────────────────────
check "no concurrency/ path (D1c — must not be owned by D1b)" \
  bash -c "! test -e $CRATE/src/concurrency"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d1b.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d1b.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p hugit-refstore --test acceptance_d1b -- --list 2>/dev/null || true)"

check "③ item_3_compaction_replay_equivalent declared" \
  bash -c "printf '%s' '$TESTLIST' | grep -q 'item_3_compaction_replay_equivalent'"
check "④ item_4_undo_restores_preserves_history declared" \
  bash -c "printf '%s' '$TESTLIST' | grep -q 'item_4_undo_restores_preserves_history'"
check "⑥ item_6_recovery_hot_do_loss_rebuild declared" \
  bash -c "printf '%s' '$TESTLIST' | grep -q 'item_6_recovery_hot_do_loss_rebuild'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "③④⑥ acceptance suite green (cargo test -p hugit-refstore --test acceptance_d1b)" \
  cargo test -p hugit-refstore --test acceptance_d1b

finish
