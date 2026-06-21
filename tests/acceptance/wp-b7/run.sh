#!/usr/bin/env bash
# WP-B7 acceptance suite — hugit-app/ui, surface v0 (items ①②③④).
# Acceptance harness for WP-B7.
# Owned items: ① live status page · ② exactly one edited comment/PR
#              · ③ saved-minutes links to CheckResult set (auditable)
#              · ④(R3) "$ saved" derived from minutes via versioned, auditable
#                       cost model (rates stated), reconcilable
# Oracle: crates/hugit-app/ui/tests/acceptance_wp-b7.rs must exist with
#   item_1_*, item_2_*, item_3_*, item_4_* tests;
#   cargo test -p hugit-app-ui --test acceptance_wp-b7 green.
# RED on absent crate; structural checks are direct bash assertions.

SUITE_ID="wp-b7"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-app/ui
ACCEPTANCE_RS="$CRATE/tests/acceptance_wp-b7.rs"
WP_ID="wp-b7"
CRATE_NAME="hugit-app-ui"

# ── (a) claimed crate exists ─────────────────────────────────────────────────
check "① crate hugit-app/ui present (Cargo.toml)" \
  test -f "$CRATE/Cargo.toml"
check "① crate hugit-app/ui present (src/lib.rs)" \
  test -f "$CRATE/src/lib.rs"

# ── (a) acceptance test file committed ───────────────────────────────────────
check "① acceptance file committed: $ACCEPTANCE_RS" \
  test -f "$ACCEPTANCE_RS"

# ── (b) cargo test --list declares item_1..item_4 ────────────────────────────
export TESTLIST="$(cargo test -p "$CRATE_NAME" --test "acceptance_${WP_ID}" -- --list 2>/dev/null || true)"

check "① item_1 test declared (live_status_page)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1'"
check "② item_2 test declared (exactly_one_edited_comment_per_pr)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2'"
check "③ item_3 test declared (saved_minutes_links_to_checkresult_set)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_3'"
check "④ item_4 test declared (cost_model_versioned_auditable_reconcilable)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_4'"

# ── (c) cargo test green ─────────────────────────────────────────────────────
check "①②③④ cargo test -p hugit-app-ui --test acceptance_wp-b7 green" \
  cargo test -p "$CRATE_NAME" --test "acceptance_${WP_ID}"

# ── (d) structural: exactly-one comment — stable marker must be present ──────
# ②: upsert-by-stable-marker ensures comment count == 1 per PR; assert a
# marker constant or function is present in ui src.
check "② stable upsert marker present in ui src (exactly-one-comment)" \
  bash -c "grep -rq 'marker\|COMMENT_MARKER\|upsert\|stable_marker' '$CRATE/src/'"

# ── (d) structural: cost model version recorded ──────────────────────────────
# ④(R3): the model version must be stored; assert a version field or constant
# referencing the cost model exists in ui src.
check "④ cost_model version field/const present in ui src" \
  bash -c "grep -rq 'cost_model_version\|CostModelVersion\|model_version' '$CRATE/src/'"

# ── (d) structural: CheckResult linkage present in ui src (③) ────────────────
check "③ CheckResult referenced in ui src (saved-minutes audit link)" \
  bash -c "grep -rq 'CheckResult\|check_result' '$CRATE/src/'"

# ── (d) structural: no new CLI binary introduced (command-catalog) ────────────
# B7 surface = App dashboard + PR comment; no new binary crate may appear.
check "② no new CLI binary in hugit-app/ui (no new human CLI)" \
  bash -c "! grep -q '^\[\[bin\]\]' '$CRATE/Cargo.toml'"

# ── (d) structural: Claims boundary — ui does not touch sidecar (B6) ─────────
check "① ui does not reference sidecar module (Claims disjointness)" \
  bash -c "! grep -rq 'hugit-app/sidecar\|hugit_app_sidecar' '$CRATE/src/'"

finish
