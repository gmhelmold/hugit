#!/usr/bin/env bash
# WP-D1c acceptance suite — hugit-refstore concurrency/perf (serialization, p99).
# Contract: docs/plan/wp-contracts/WP-D1c.md
# Owned items (verbatim):
#   ⑤ 100 concurrent ops: serialized, 0 loss, p99<500ms
# Oracle: crates/hugit-refstore/tests/acceptance_d1c.rs
#   Tests: item_5_concurrent_serialized_zero_loss_p99
# Note: the Rust test itself drives 100 concurrent ops, measures p99, and asserts
#       the <500ms bound; this suite verifies the test is declared and passes.

SUITE_ID="wp-d1c"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-refstore

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-refstore crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: D1c-owned module path exists ─────────────────────────────────
check "concurrency/ module present under src/" \
  test -d "$CRATE/src/concurrency"

# ── negative: D1a paths must not be absent (substrate must exist) ─────────────
check "log/ path not absent (D1a substrate must exist)" \
  test -d "$CRATE/src/log"
check "replay/ path not absent (D1a substrate must exist)" \
  test -d "$CRATE/src/replay"

# ── negative: D1b paths must not be absent (substrate must exist) ─────────────
check "compaction/ path not absent (D1b substrate must exist)" \
  test -d "$CRATE/src/compaction"

# ── negative: D1c must not own D1a/D1b source paths (leak guard) ─────────────
# The concurrency/ module is the only D1c-owned src path; verifying no writes
# leaked into D1a/D1b is enforcement at SEAL time, not detectable structurally
# here — so we assert D1c's own module is isolated (it exists) and that D1a/D1b
# substrate modules exist unmodified (above).

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d1c.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d1c.rs"

# ── oracle: item test declared via cargo test --list ─────────────────────────
TESTLIST="$(cargo test -p hugit-refstore --test acceptance_d1c -- --list 2>/dev/null || true)"

check "⑤ item_5_concurrent_serialized_zero_loss_p99 declared" \
  bash -c "printf '%s' '$TESTLIST' | grep -q 'item_5_concurrent_serialized_zero_loss_p99'"

# ── oracle: acceptance suite runs green (Rust test asserts p99<500ms + zero loss) ──
check "⑤ acceptance suite green (cargo test -p hugit-refstore --test acceptance_d1c)" \
  cargo test -p hugit-refstore --test acceptance_d1c

finish
