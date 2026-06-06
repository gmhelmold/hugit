#!/usr/bin/env bash
# WP-X2 acceptance suite — attestation end-to-end.
# Contract: docs/plan/wp-contracts/WP-X2.md
# Owned items:
#   ① artifact attestation resolves full chain (tree+def+runner+model+principal) cryptographically
#   ② tampered/unsigned attestation rejected at promotion
#   ③ verification is a public, documented procedure
#   ④(R7) cross-tenant-shared hit honesty: anonymized PLATFORM attestation — no tenant leak
# Claims: crates/hugit-invariants/x2/
# Oracle: crates/hugit-invariants/x2/tests/acceptance_x2.rs
#   one #[test] item_<n>_<slug> per owned item.
# RED on current tree (hugit-contracts attestation surface not yet built).

export TESTLIST="item_1_full_chain_resolves item_2_tampered_unsigned_rejected item_3_public_verification_procedure item_4_cross_tenant_shared_hit_honesty"

SUITE_ID="wp-x2"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-invariants"
CRATE_NAME="hugit-invariants"
ACCEPTANCE_FILE="$CRATE_ROOT/x2/tests/acceptance_x2.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-invariants crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-invariants Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-invariants is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-invariants"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_x2.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "x2/ subtree present" \
  test -d "$CRATE_ROOT/x2"

# ── ① full chain resolves cryptographically ──────────────────────────────────
check "① item_1_full_chain_resolves declared in acceptance_x2" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x2 -- --list 2>/dev/null | \
   grep -q "item_1_full_chain_resolves"'

# ── ② tampered/unsigned rejected at promotion ────────────────────────────────
check "② item_2_tampered_unsigned_rejected declared in acceptance_x2" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x2 -- --list 2>/dev/null | \
   grep -q "item_2_tampered_unsigned_rejected"'

# ── ③ public verification procedure ─────────────────────────────────────────
check "③ item_3_public_verification_procedure declared in acceptance_x2" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x2 -- --list 2>/dev/null | \
   grep -q "item_3_public_verification_procedure"'

# ── ④ cross-tenant shared-hit honesty ───────────────────────────────────────
check "④ item_4_cross_tenant_shared_hit_honesty declared in acceptance_x2" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x2 -- --list 2>/dev/null | \
   grep -q "item_4_cross_tenant_shared_hit_honesty"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-invariants --test acceptance_x2 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_x2

finish
