#!/usr/bin/env bash
# WP-B3 acceptance suite — affected-targets v0 (all 3 owned items).
# Contract: docs/plan/wp-contracts/WP-B3.md. Green = golden sets cargo/pnpm/turbo,
# root edit→full set, unknown ecosystem→full set fail-open. RED on absent crate.
#
# Naming convention (pre-decided by the lead, binding for the agent):
#   acceptance test  → item_<n>_<slug>   (e.g. item_1_golden_sets)
#   acceptance file  → crates/hugit-checks/tests/acceptance_wp_b3.rs

SUITE_ID="wp-b3"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-checks
AFFECTED_DIR="$CRATE/affected"
ACCEPTANCE_FILE="$CRATE/tests/acceptance_wp_b3.rs"

# ── (a) the claimed crate path exists ────────────────────────────────────────
check "① hugit-checks/affected/ dir present" \
  bash -c "test -d $AFFECTED_DIR"
check "① hugit-checks crate present (Cargo.toml)" \
  bash -c "test -f $CRATE/Cargo.toml"

# ── (b) cargo test --list declares item_<n> for every owned item ─────────────
TESTLIST="$(cargo test -p hugit-checks --test acceptance_wp_b3 -- --list 2>/dev/null || true)"

check "① item_1 declared (golden sets cargo/pnpm/turbo)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_1'"
check "② item_2 declared (root edit→full set)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_2'"
check "③ item_3 declared (unknown ecosystem→full set fail-open)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_3'"

# ── (c) cargo test -p hugit-checks --test acceptance_wp_b3 is green ──────────
check "cargo test -p hugit-checks --test acceptance_wp_b3 green" \
  cargo test -p hugit-checks --test acceptance_wp_b3

# ── (d) structural / negative asserts from the contract ──────────────────────

# ① per-ecosystem adapters must have fixture files committed
check "① cargo golden fixture committed" \
  bash -c "find $AFFECTED_DIR -name '*.json' -o -name '*.toml' | xargs grep -l 'cargo\|workspace' 2>/dev/null | grep -q ."
check "① pnpm golden fixture committed" \
  bash -c "find $AFFECTED_DIR -name '*.json' -o -name 'pnpm*' | xargs grep -l 'pnpm\|packages' 2>/dev/null | grep -q ."
check "① turbo golden fixture committed" \
  bash -c "find $AFFECTED_DIR -name 'turbo*' -o -name '*.json' | xargs grep -l 'turbo\|pipeline\|tasks' 2>/dev/null | grep -q ."

# ② root-edit full-set path: root manifest handling present in src
check "② root manifest invalidation logic present in src" \
  bash -c "grep -rq 'root\|workspace.*Cargo\.toml\|pnpm-workspace\|full.set\|full_set' $AFFECTED_DIR/"

# ③ fail-open policy present in src (unknown ecosystem returns full set)
check "③ fail-open policy present in src (unknown ecosystem)" \
  bash -c "grep -rq 'fail.open\|fail_open\|unknown.*full\|full.*unknown\|Unknown' $AFFECTED_DIR/"

# Claims boundary: client/, runner/, regen/ must NOT be authored by this WP
check "src/client/ not authored by this WP (reserved for B2a)" \
  bash -c "! test -d $CRATE/src/client"
check "src/runner/ not authored by this WP (reserved for B2b)" \
  bash -c "! test -d $CRATE/src/runner"
check "regen/ not authored by this WP (reserved for C4)" \
  bash -c "! test -d $CRATE/regen"

finish
