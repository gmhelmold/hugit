#!/usr/bin/env bash
# WP-X4 acceptance suite — supply chain: image pinning + integrity + fail-closed.
# Acceptance harness for WP-X4.
# Owned items:
#   ① runner images content-pinned + integrity-verified at spawn
#   ② App dependencies pinned + verified in CI
#   ③ tampered/unpinned image → fail CLOSED before any tenant work
# Claims: crates/hugit-invariants/x4/
# Oracle: crates/hugit-invariants/x4/tests/acceptance_x4.rs
#   one #[test] item_<n>_<slug> per owned item.
# Box env: HUGIT_RUNNER_HOST=203.0.113.10 (exported before cargo invocations;
#   box-dependent tests FAIL — not skip — when env set but box unreachable).
#   Tests may skip only when HUGIT_RUNNER_HOST is entirely unset.
# RED on current tree (crate paths not yet built).

export TESTLIST="item_1_image_content_pinned_verified item_2_app_deps_pinned_ci item_3_tampered_unpinned_fail_closed"

SUITE_ID="wp-x4"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=203.0.113.10

CRATE_ROOT="crates/hugit-invariants"
CRATE_NAME="hugit-invariants"
ACCEPTANCE_FILE="$CRATE_ROOT/x4/tests/acceptance_x4.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-invariants crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-invariants Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-invariants is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-invariants"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_x4.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "x4/ subtree present" \
  test -d "$CRATE_ROOT/x4"

# ── structural negative: only x4/ paths owned ───────────────────────────────
check "no WP-X4 files written outside x4/ subtree (disjointness also at integration)" \
  bash -c '! find '"$CRATE_ROOT"' -maxdepth 1 -name "*.rs" ! -name lib.rs ! -name main.rs 2>/dev/null | grep -q .'

# ── ① image content-pinned + integrity-verified at spawn ────────────────────
check "① item_1_image_content_pinned_verified declared in acceptance_x4" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x4 -- --list 2>/dev/null | \
   grep -q "item_1_image_content_pinned_verified"'

# ── ② App deps pinned + verified in CI ──────────────────────────────────────
check "② item_2_app_deps_pinned_ci declared in acceptance_x4" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x4 -- --list 2>/dev/null | \
   grep -q "item_2_app_deps_pinned_ci"'

# ── ③ tampered/unpinned → fail CLOSED before tenant work ────────────────────
# Load-bearing: verify-before-work ordering; post-hoc detection is a FAIL.
check "③ item_3_tampered_unpinned_fail_closed declared in acceptance_x4" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x4 -- --list 2>/dev/null | \
   grep -q "item_3_tampered_unpinned_fail_closed"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-invariants --test acceptance_x4 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_x4

finish
