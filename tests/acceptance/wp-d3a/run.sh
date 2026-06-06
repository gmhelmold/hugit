#!/usr/bin/env bash
# WP-D3a acceptance suite — receive-pack → CAS + log (push core).
# Contract: docs/plan/wp-contracts/WP-D3a.md
# Owned item (verbatim from decomposition v2.0 — D3a):
#   ① push→clone round-trip identical
# Red-team items (D3a source-of-truth bar, owned by D3a):
#   redteam_malformed_pack_rejected
#   redteam_oversized_pack_rejected
#   redteam_ref_update_tamper_rejected
# Oracle: crates/hugit-proto/tests/acceptance_d3a.rs
#   Tests: item_1_push_clone_roundtrip_identical,
#          redteam_malformed_pack_rejected,
#          redteam_oversized_pack_rejected,
#          redteam_ref_update_tamper_rejected

SUITE_ID="wp-d3a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-proto

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-proto crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: owned module paths exist ─────────────────────────────────────
check "write/receive/ module present under src/" \
  test -d "$CRATE/src/write/receive"
check "write/store/ module present under src/" \
  test -d "$CRATE/src/write/store"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d3a.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d3a.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p hugit-proto --test acceptance_d3a -- --list 2>/dev/null || true)"
export TESTLIST

check "① item_1_push_clone_roundtrip_identical declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1_push_clone_roundtrip_identical'"
check "redteam_malformed_pack_rejected declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'redteam_malformed_pack_rejected'"
check "redteam_oversized_pack_rejected declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'redteam_oversized_pack_rejected'"
check "redteam_ref_update_tamper_rejected declared" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'redteam_ref_update_tamper_rejected'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "① push→clone round-trip identical (cargo test -p hugit-proto --test acceptance_d3a)" \
  cargo test -p hugit-proto --test acceptance_d3a

finish
