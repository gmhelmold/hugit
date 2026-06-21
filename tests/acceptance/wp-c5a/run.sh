#!/usr/bin/env bash
# WP-C5a acceptance suite — sparse fence materialization + path enforcement.
# Acceptance harness for WP-C5a.
# Owned items: ① outside path_set → ENOENT
# Claims: crates/hugit-fence/{materialize,enforce}
# Oracle: crates/hugit-fence/tests/acceptance_c5a.rs
#   one #[test] item_<n>_<slug> per owned item.
# Box env: HUGIT_RUNNER_HOST=203.0.113.10 (exported before cargo invocations;
#   box-dependent tests FAIL — not skip — when env set but box unreachable).
#   Tests may skip only when HUGIT_RUNNER_HOST is entirely unset.
# RED on current tree (crate not yet built).

SUITE_ID="wp-c5a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=203.0.113.10

CRATE_ROOT="crates/hugit-fence"
CRATE_NAME="hugit-fence"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c5a.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-fence crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-fence Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-fence is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-fence"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_c5a.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "materialize/ subtree present" \
  test -d "$CRATE_ROOT/src/materialize"

check "enforce/ subtree present" \
  test -d "$CRATE_ROOT/src/enforce"

# C5b broker-absence guard RETIRED (lead, wave D2 integration): a wave-isolation
# fence that expires once C5b legitimately lands src/broker/ (same class as the
# b1/d1a/d1b/b4a guard expiries). C5a↔C5b disjointness is enforced at integration
# via `git diff --name-only main..HEAD` (scope-clean gate).

# ── ① outside path_set → ENOENT ──────────────────────────────────────────────
check "① item_1_outside_path_set_enoent declared in acceptance_c5a" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c5a -- --list 2>/dev/null | \
   grep -q "item_1_outside_path_set_enoent"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-fence --test acceptance_c5a green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c5a

finish
