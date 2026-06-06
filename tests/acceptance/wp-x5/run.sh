#!/usr/bin/env bash
# WP-X5 acceptance suite — namespace laws.
# Contract: docs/plan/wp-contracts/WP-X5.md
# Owned items:
#   ① no hugit CLI verb shadows a git verb (mechanized check against `git help -a`)
#   ② managed refs (refs/hugit/…) never collide with arbitrary user branches/tags (property test)
# Claims: crates/hugit-invariants/x5/
# Oracle: crates/hugit-invariants/x5/tests/acceptance_x5.rs
#   one #[test] item_<n>_<slug> per owned item.
# RED on current tree (hugit-cli verb surface + hugit-refstore not yet built).

export TESTLIST="item_1_no_hugit_verb_shadows_git_verb item_2_managed_refs_no_collision"

SUITE_ID="wp-x5"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-invariants"
CRATE_NAME="hugit-invariants"
ACCEPTANCE_FILE="$CRATE_ROOT/x5/tests/acceptance_x5.rs"

# ── git availability ─────────────────────────────────────────────────────────
# Item ① shells out to git; confirm git is present before running cargo tests.
check "git is available (required by item ①)" \
  git help -a

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-invariants crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-invariants Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-invariants is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-invariants"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_x5.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "x5/ subtree present" \
  test -d "$CRATE_ROOT/x5"

# ── ① no hugit verb shadows a git verb ──────────────────────────────────────
# Load-bearing: oracle shells out to git help -a at test time — no hand list.
check "① item_1_no_hugit_verb_shadows_git_verb declared in acceptance_x5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x5 -- --list 2>/dev/null | \
   grep -q "item_1_no_hugit_verb_shadows_git_verb"'

# ── ② managed refs never collide with user refs ──────────────────────────────
check "② item_2_managed_refs_no_collision declared in acceptance_x5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_x5 -- --list 2>/dev/null | \
   grep -q "item_2_managed_refs_no_collision"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-invariants --test acceptance_x5 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_x5

finish
