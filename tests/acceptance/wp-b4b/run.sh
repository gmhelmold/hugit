#!/usr/bin/env bash
# WP-B4b acceptance suite — hugit-queue crate, GitHub integration surface (items ③④⑥).
# Contract: docs/plan/wp-contracts/WP-B4b.md.
# Owned items:
#   ③ force-push recompute — on force-push webhook, prior union invalidated, EventRecord written
#   ④ crash idempotent (kill-test) — worker killed mid-land; on restart: no double-merge,
#      no lost batch, no false green
#   ⑥ protected/required-review PR is HELD+reported, never force-merged; merge method honored
# Oracle: crates/hugit-queue/tests/acceptance_wp-b4b.rs must exist with
#   item_3_*, item_4_*, item_6_* tests; cargo test -p hugit-queue --test acceptance_wp-b4b green.
# RED on absent crate/module/oracle; live-GitHub items require HUGIT_GH_TEST_REPO (fail if unset).

SUITE_ID="wp-b4b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-queue
ACCEPTANCE_RS="$CRATE/tests/acceptance_wp-b4b.rs"
WP_ID="wp-b4b"

# Live-GitHub items are not skippable: fail immediately when env unset.
if [ -z "${HUGIT_GH_TEST_REPO:-}" ]; then
  fail "③④⑥ HUGIT_GH_TEST_REPO unset — live-GitHub items require a test repo (not skippable)"
else
  export HUGIT_GH_TEST_REPO
fi

# ── (a) claimed crate exists ─────────────────────────────────────────────────
check "③ crate hugit-queue present (Cargo.toml)" \
  test -f "$CRATE/Cargo.toml"
check "③ crate hugit-queue present (src/lib.rs)" \
  test -f "$CRATE/src/lib.rs"

# ── (a) GitHub integration module exists (Claims: src/github/) ───────────────
check "③ github/ module dir exists: src/github/" \
  test -d "$CRATE/src/github"

# ── (a) acceptance test file committed ───────────────────────────────────────
check "③ acceptance file committed: $ACCEPTANCE_RS" \
  test -f "$ACCEPTANCE_RS"

# ── (b) cargo test --list declares item_3, item_4, item_6 ────────────────────
export TESTLIST="$(cargo test -p hugit-queue --test "acceptance_${WP_ID}" -- --list 2>/dev/null || true)"

check "③ item_3 test declared (force_push_recompute)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3'"
check "④ item_4 test declared (crash_idempotent_kill_test)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4'"
check "⑥ item_6 test declared (protected_pr_held_not_force_merged)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6'"

# ── (c) cargo test -p hugit-queue --test acceptance_wp-b4b green ─────────────
check "③④⑥ cargo test -p hugit-queue --test acceptance_wp-b4b green" \
  cargo test -p hugit-queue --test "acceptance_${WP_ID}"

# ── (d) structural: EventRecord audit on recompute (③) ───────────────────────
check "③ force_push_recompute references EventRecord in github/ src" \
  bash -c "grep -rq 'EventRecord' '$CRATE/src/github/'"

# ── (d) structural: no stale-union bypass (③) ────────────────────────────────
check "③ no stale_union/skip_recompute/reuse_union bypass in github/ src" \
  bash -c "! grep -rEq 'stale_union|skip_recompute|reuse_union' '$CRATE/src/github/'"

# ── (d) structural: crash-recovery harness present in github/ (④) ─────────────
check "④ crash_recovery/idempotent/replay referenced in github/ src" \
  bash -c "grep -rEq 'crash_recover|idempotent|replay' '$CRATE/src/github/'"

# ── (d) structural: no force-merge/bypass_protection path (⑥) ────────────────
check "⑥ no force_merge/bypass_protection path in github/ src" \
  bash -c "! grep -rEq 'force_merge|bypass_protection|ignore_protection' '$CRATE/src/github/'"

# ── (d) structural: merge method honored — method dispatch present (⑥) ────────
check "⑥ merge method dispatch (squash|merge_method|MergeMethod) in github/ src" \
  bash -c "grep -rEq 'squash|merge_method|MergeMethod' '$CRATE/src/github/'"

finish
