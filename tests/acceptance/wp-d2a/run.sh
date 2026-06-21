#!/usr/bin/env bash
# WP-D2a acceptance suite — pack assembly + clone/fetch core.
# Acceptance harness for WP-D2a.
# Owned items (verbatim from decomposition v2.0 — D2a):
#   ① clone byte-identical to mirror
#   ② delta-only fetch
# Oracle: crates/hugit-proto/tests/acceptance_d2a.rs
#   Tests: item_1_clone_byte_identical, item_2_delta_only_fetch

SUITE_ID="wp-d2a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-proto

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-proto crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: owned module paths exist ─────────────────────────────────────
check "read/negotiate/ module present under src/" \
  test -d "$CRATE/src/read/negotiate"
check "read/pack/ module present under src/" \
  test -d "$CRATE/src/read/pack"
check "read/serve/ module present under src/" \
  test -d "$CRATE/src/read/serve"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d2a.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d2a.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p hugit-proto --test acceptance_d2a -- --list 2>/dev/null || true)"
export TESTLIST

check "① item_1_clone_byte_identical declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1_clone_byte_identical'"
check "② item_2_delta_only_fetch declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2_delta_only_fetch'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "① clone byte-identical to mirror (cargo test -p hugit-proto --test acceptance_d2a)" \
  cargo test -p hugit-proto --test acceptance_d2a

finish
