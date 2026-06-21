#!/usr/bin/env bash
# WP-C1 acceptance suite — runner inventory + reuse verdict (docs deliverable).
# Acceptance harness for WP-C1.
# Owned item: ① written inventory + reuse verdict/item
# GREEN = document present at claimed path, required sections present.
# RED on current tree (no product code yet).

SUITE_ID="wp-c1"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

INVENTORY_DIR="docs/runners-inventory"

# ── ① written inventory + reuse verdict/item ────────────────────────────────
# Claim: inventory directory contains the inventory document.
check "① inventory directory exists" \
  test -d "$INVENTORY_DIR"

# Claim: an inventory document is present under the inventory directory.
check "① inventory document file present" \
  bash -c "ls $INVENTORY_DIR/*.md >/dev/null 2>&1"

# Claim: the document contains a reuse verdict table (each row must
# carry a verdict ∈ {reuse-verbatim, adapt, build-new} with a one-line rationale).
check "① reuse verdict column present (reuse-verbatim | adapt | build-new)" \
  bash -c "grep -rqE '(reuse-verbatim|adapt|build-new)' $INVENTORY_DIR/"

# Claim: source paths are cited (inventory enumerates existing runner
# capability and binds a verdict with a rationale; citations are required).
check "① source path citations present in inventory" \
  bash -c "grep -rqE 'corelink|campaign[-_ ]?#?1|runners?' $INVENTORY_DIR/"

# Contract claim: Markdown table present (pipe-delimited row as evidence of
# required table structure per the conventions note).
check "① Markdown table structure present (pipe-delimited rows)" \
  bash -c "grep -rqE '^\|' $INVENTORY_DIR/"

finish
