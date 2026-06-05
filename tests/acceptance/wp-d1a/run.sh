#!/usr/bin/env bash
# WP-D1a acceptance suite — hugit-refstore log core (append, hash chain, replay, tamper).
# Contract: docs/plan/wp-contracts/WP-D1a.md
# Owned items (verbatim):
#   ① 10k-event replay identical
#   ② tamper detected
# Oracle: crates/hugit-refstore/tests/acceptance_d1a.rs
#   Tests: item_1_replay_identical, item_2_tamper_detected

SUITE_ID="wp-d1a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-refstore

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-refstore crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: owned module paths exist ─────────────────────────────────────
check "log/ module present under src/" \
  test -d "$CRATE/src/log"
check "replay/ module present under src/" \
  test -d "$CRATE/src/replay"
check "tamper/ module present under src/" \
  test -d "$CRATE/src/tamper"

# ── D1b/D1c leak guards RETIRED (lead, wave D1.2 integration): they were
# D1a-wave isolation fences asserting sibling modules absent; D1b/D1c now
# legitimately own compaction//recovery//undo//concurrency/. D1a's
# functional items (replay-identity, tamper) below remain the living suite.

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d1a.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d1a.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p hugit-refstore --test acceptance_d1a -- --list 2>/dev/null || true)"

check "① item_1_replay_identical declared" \
  bash -c "echo '$TESTLIST' | grep -q 'item_1_replay_identical'"
check "② item_2_tamper_detected declared" \
  bash -c "echo '$TESTLIST' | grep -q 'item_2_tamper_detected'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "① 10k-event replay identical (cargo test -p hugit-refstore --test acceptance_d1a)" \
  cargo test -p hugit-refstore --test acceptance_d1a

finish
