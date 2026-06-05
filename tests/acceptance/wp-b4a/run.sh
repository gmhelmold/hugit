#!/usr/bin/env bash
# WP-B4a acceptance suite — hugit-queue crate, core engine (items ①②⑤).
# Contract: docs/plan/wp-contracts/WP-B4a.md.
# Owned items: ① A+B-red pair excluded+named · ② 5 disjoint greens land, 0 re-runs
#              · ⑤ lands in queue order; out-of-order structurally prevented.
# Oracle: crates/hugit-queue/tests/acceptance_wp-b4a.rs must exist with
#   item_1_*, item_2_*, item_5_* tests; cargo test -p hugit-queue --test acceptance_wp-b4a green.
# RED on absent crate; structural checks are direct bash assertions.

SUITE_ID="wp-b4a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-queue
ACCEPTANCE_RS="$CRATE/tests/acceptance_wp-b4a.rs"
WP_ID="wp-b4a"

# ── (a) claimed crate exists ─────────────────────────────────────────────────
check "① crate hugit-queue present (Cargo.toml)" \
  test -f "$CRATE/Cargo.toml"
check "① crate hugit-queue present (src/lib.rs)" \
  test -f "$CRATE/src/lib.rs"

# ── (a) core module exists (Claims: src/core/) ───────────────────────────────
check "① core module dir exists: src/core/" \
  test -d "$CRATE/src/core"

# ── (a) acceptance test file committed ───────────────────────────────────────
check "① acceptance file committed: $ACCEPTANCE_RS" \
  test -f "$ACCEPTANCE_RS"

# ── (b) cargo test --list declares item_1, item_2, item_5 ────────────────────
TESTLIST="$(cargo test -p hugit-queue --test "acceptance_${WP_ID}" -- --list 2>/dev/null || true)"

check "① item_1 test declared (minimal_failing_pair)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_1'"
check "② item_2 test declared (disjoint_greens_zero_reruns)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_2'"
check "⑤ item_5 test declared (queue_order_out_of_order_prevented)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_5'"

# ── (c) cargo test -p hugit-queue --test acceptance_wp-b4a green ─────────────
check "①②⑤ cargo test -p hugit-queue --test acceptance_wp-b4a green" \
  cargo test -p hugit-queue --test "acceptance_${WP_ID}"

# ── (d) Claims-boundary guard RETIRED (lead, wave D1.2 integration): the
# "no github/" assert was a B4a-wave isolation fence; B4b legitimately owns
# src/github/ now. The living boundary check: core/ stays GitHub-free. ──────
check "① core/ contains no GitHub API calls (B4a engine purity)" \
  bash -c "! grep -rEq 'api\.github\.com|octocrab|github_api' '$CRATE/src/core/'"

# ── (d) structural: QueueApi.minimal_failing_pair field present in contracts ──
check "① QueueApi.minimal_failing_pair referenced in hugit-queue src" \
  bash -c "grep -rq 'minimal_failing_pair' '$CRATE/src/'"

# ── (d) structural: ordering invariant — no force-land / skip-predecessor path ─
check "⑤ no skip_predecessor / force_land bypass in core src" \
  bash -c "! grep -rEq 'skip_predecessor|force_land|bypass_order' '$CRATE/src/core/'"

finish
