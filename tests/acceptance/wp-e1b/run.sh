#!/usr/bin/env bash
# WP-E1b acceptance suite — verified mirror: failure modes (outage, partial divergence, one-way) (items ② ④ ⑤ ⑥ ⑦).
# Contract: docs/plan/wp-contracts/WP-E1b.md
# Owned items:
#   ②    divergence → alarm + repair + incident
#   ④(+) GitHub 429/5xx for N hours: durable queue, bounded backoff, no drop/reorder;
#          on recovery drains to verified sync + incident records gap
#   ⑤(+) force-push/branch-delete/tag ops replicate; deleted refs absent; no false divergence from orphans
#   ⑥(+) partial divergence: repair scoped to broken ref only; webhook loss → poll fallback detects within SLA
#   ⑦(R2) ONE-WAY enforced: direct write on GitHub mirror → divergence (alarm → forge-auth repair → incident), zero reverse sync
# Oracle: crates/hugit-mirror/tests/acceptance_e1b.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-mirror/src/divergence/, src/outage/, src/refops/, src/poll/
# Outage/divergence/one-way items are local fixture proofs over mirror logic.
# RED on current tree (divergence/ outage/ refops/ poll/ subtrees not yet built).

SUITE_ID="wp-e1b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-mirror"
CRATE_NAME="hugit-mirror"
DIVERGENCE_DIR="$CRATE_ROOT/src/divergence"
OUTAGE_DIR="$CRATE_ROOT/src/outage"
REFOPS_DIR="$CRATE_ROOT/src/refops"
POLL_DIR="$CRATE_ROOT/src/poll"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e1b.rs"

export HUGIT_GH_TEST_REPO="humangr-labs/hugit-fleet-syn-1"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-mirror crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-mirror Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-mirror is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-mirror"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "② divergence/ subtree present (Claims: src/divergence/)" \
  test -d "$DIVERGENCE_DIR"

check "④ outage/ subtree present (Claims: src/outage/)" \
  test -d "$OUTAGE_DIR"

check "⑤ refops/ subtree present (Claims: src/refops/)" \
  test -d "$REFOPS_DIR"

check "⑥ poll/ subtree present (Claims: src/poll/)" \
  test -d "$POLL_DIR"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_e1b.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── barrel export: modules exported from lib.rs ───────────────────────────────
check "② lib.rs barrel exports divergence module" \
  bash -c "grep -q 'divergence' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "④ lib.rs barrel exports outage module" \
  bash -c "grep -q 'outage' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "⑤ lib.rs barrel exports refops module" \
  bash -c "grep -q 'refops' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "⑥ lib.rs barrel exports poll module" \
  bash -c "grep -q 'poll' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

# ── HUGIT_GH_TEST_REPO must be non-empty (suite always sets it) ───────────────
check "⑦ HUGIT_GH_TEST_REPO env set to non-empty value" \
  bash -c "test -n \"\$HUGIT_GH_TEST_REPO\""

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_e1b -- --list 2>/dev/null || true)"

# ── ② divergence → alarm + repair + incident ─────────────────────────────────
check "② item_2_divergence_alarm_repair_incident declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_divergence_alarm_repair_incident'"

# ── ④ GitHub outage: durable queue, bounded backoff, no drop/reorder ─────────
check "④ item_4_outage_queue_bounded_backoff_no_drop declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_outage_queue_bounded_backoff_no_drop'"

check "④ item_4_outage_recovery_drain_verified_gap_incident declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_outage_recovery_drain_verified_gap_incident'"

# ── ⑤ ref ops replicate; deleted refs absent; no false divergence from orphans ─
check "⑤ item_5_refops_replicate_force_push_branch_delete_tag declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5_refops_replicate_force_push_branch_delete_tag'"

check "⑤ item_5_deleted_ref_absent_on_mirror declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5_deleted_ref_absent_on_mirror'"

check "⑤ item_5_no_false_divergence_from_orphans declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5_no_false_divergence_from_orphans'"

# ── ⑥ partial divergence scoped; poll fallback detects within SLA ─────────────
check "⑥ item_6_partial_divergence_repair_scoped_to_broken_ref declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_partial_divergence_repair_scoped_to_broken_ref'"

check "⑥ item_6_webhook_loss_poll_fallback_detects_within_sla declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_webhook_loss_poll_fallback_detects_within_sla'"

# ── ⑦ one-way enforced: reverse write → divergence, zero reverse sync ─────────
check "⑦ item_7_reverse_write_treated_as_divergence declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_7_reverse_write_treated_as_divergence'"

check "⑦ item_7_zero_reverse_sync_codepath_absent declared in acceptance_e1b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_7_zero_reverse_sync_codepath_absent'"

# ── structural: divergence handler emits alarm + repair + incident triple ─────
check "② divergence/ references alarm, repair, and incident emission" \
  bash -c "grep -rEq 'alarm|Alarm' '$DIVERGENCE_DIR/' 2>/dev/null && grep -rEq 'repair|Repair' '$DIVERGENCE_DIR/' 2>/dev/null && grep -rEq 'incident|Incident' '$DIVERGENCE_DIR/' 2>/dev/null"

# ── structural: forge-authoritative repair (mirror is overwritten, not merged) ─
check "② divergence/ references forge-authoritative repair (forge wins)" \
  bash -c "grep -rEq 'forge_authoritative|ForgeAuthoritative|forge_wins|authoritative_repair' '$DIVERGENCE_DIR/' 2>/dev/null"

# ── structural: bounded backoff in outage/ ────────────────────────────────────
check "④ outage/ references bounded exponential backoff" \
  bash -c "grep -rEq 'backoff|Backoff|back_off|exponential' '$OUTAGE_DIR/' 2>/dev/null"

# ── structural: gap incident emitted in outage/ ───────────────────────────────
check "④ outage/ references gap incident on recovery drain" \
  bash -c "grep -rEq 'gap_incident|GapIncident|gap.*incident|incident.*gap' '$OUTAGE_DIR/' 2>/dev/null"

# ── structural: orphan-aware diff in refops/ (compare ref tips, not loose objects) ──
check "⑤ refops/ references orphan-aware divergence logic (ref tips, not loose objects)" \
  bash -c "grep -rEq 'orphan|Orphan|ref_tip|RefTip|tip.*diff|loose_object' '$REFOPS_DIR/' 2>/dev/null"

# ── structural: poll/ detects divergence via polling (webhook-loss path) ──────
check "⑥ poll/ references polling / webhook-loss fallback detector" \
  bash -c "grep -rEq 'poll|Poll|webhook_loss|WebhookLoss|fallback' '$POLL_DIR/' 2>/dev/null"

# ── structural: fail-CLOSED — undecidable state → divergent, never synced ────
check "② divergence/ is fail-CLOSED (undecidable → divergent)" \
  bash -c "grep -rEq 'fail_closed|FailClosed|degraded|Degraded|undecidable|Undecidable' '$DIVERGENCE_DIR/' 2>/dev/null"

# ── structural: no reverse-sync codepath anywhere in E1b Claims ───────────────
check "⑦ Claims paths contain no reverse-sync import or entry point" \
  bash -c "! grep -rEq 'reverse_sync|ReverseSync|sync_from_github|from_mirror' \
    '$DIVERGENCE_DIR/' '$OUTAGE_DIR/' '$REFOPS_DIR/' '$POLL_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "②④⑤⑥⑦ cargo test -p hugit-mirror --test acceptance_e1b green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e1b

finish
