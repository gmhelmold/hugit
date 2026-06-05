#!/usr/bin/env bash
# WP-C2a acceptance suite — ephemeral runner: lease lifecycle + isolation.
# Contract: docs/plan/wp-contracts/WP-C2a.md
# Owned items: ① destroy leaves nothing (forensic re-scan)
#              ② lease isolation (tmp/net)
# Claims: crates/hugit-runner/{lease,isolation,teardown}
# Oracle: crates/hugit-runner/tests/acceptance_c2a.rs
#   one #[test] item_<n>_<slug> per owned item.
# Box env: HUGIT_RUNNER_HOST=91.99.11.196 (exported before cargo invocations;
#   box-dependent tests FAIL — not skip — when unreachable).
# RED on current tree (crate not yet built).

SUITE_ID="wp-c2a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=91.99.11.196

CRATE_ROOT="crates/hugit-runner"
CRATE_NAME="hugit-runner"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c2a.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-runner crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-runner Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-runner is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   jq -e '"'"'.packages[] | select(.name == "hugit-runner")'"'"' >/dev/null'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_c2a.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── ① destroy leaves nothing (forensic re-scan) ─────────────────────────────
check "① item_1_destroy_leaves_nothing declared in acceptance_c2a" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c2a -- --list 2>/dev/null | \
   grep -q "item_1_destroy_leaves_nothing"'

# ── ② lease isolation (tmp/net) ─────────────────────────────────────────────
check "② item_2_lease_isolation declared in acceptance_c2a" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c2a -- --list 2>/dev/null | \
   grep -q "item_2_lease_isolation"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-runner --test acceptance_c2a green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c2a

finish
