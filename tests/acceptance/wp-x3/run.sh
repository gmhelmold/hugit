#!/usr/bin/env bash
# WP-X3 acceptance suite — context privacy.
# Acceptance harness for WP-X3.
# Owned items:
#   ① context/journals tenant-scoped (cross-tenant fetch denied)
#   ② redaction at capture AND export
#   ③ retention/deletion purges (verified absent)
#   ④ training/eval exclusion: documented control + audit trail
# Claims: crates/hugit-invariants/x3/
# Oracle: crates/hugit-invariants/x3/tests/acceptance_x3.rs
#   one #[test] item_<n>_<slug> per owned item.
# RED on current tree (hugit-context / ExportSchema surfaces not yet built).

export TESTLIST="item_1_tenant_scope_cross_fetch_denied item_2_redaction_at_capture_and_export item_3_retention_deletion_purge item_4_training_exclusion_control_and_audit"

SUITE_ID="wp-x3"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-invariants"
CRATE_NAME="hugit-invariants"
ACCEPTANCE_FILE="$CRATE_ROOT/x3/tests/acceptance_x3.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-invariants crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-invariants Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-invariants is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-invariants"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_x3.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "x3/ subtree present" \
  test -d "$CRATE_ROOT/x3"

# ── ① tenant-scoped: cross-fetch denied ─────────────────────────────────────
check "① item_1_tenant_scope_cross_fetch_denied declared in acceptance_x3" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x3 -- --list 2>/dev/null | \
   grep -q "item_1_tenant_scope_cross_fetch_denied"'

# ── ② redaction at capture and export ───────────────────────────────────────
check "② item_2_redaction_at_capture_and_export declared in acceptance_x3" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x3 -- --list 2>/dev/null | \
   grep -q "item_2_redaction_at_capture_and_export"'

# ── ③ retention/deletion purge verified absent ───────────────────────────────
check "③ item_3_retention_deletion_purge declared in acceptance_x3" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x3 -- --list 2>/dev/null | \
   grep -q "item_3_retention_deletion_purge"'

# ── ④ training exclusion control and audit trail ─────────────────────────────
check "④ item_4_training_exclusion_control_and_audit declared in acceptance_x3" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x3 -- --list 2>/dev/null | \
   grep -q "item_4_training_exclusion_control_and_audit"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-invariants --test acceptance_x3 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_x3

finish
