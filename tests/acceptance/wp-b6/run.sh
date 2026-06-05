#!/usr/bin/env bash
# WP-B6 acceptance suite — hugit-app/sidecar, intent sidecar (items ①②③④).
# Contract: docs/plan/wp-contracts/WP-B6.md.
# Owned items: ① parsed/validated/rendered · ② malformed→actionable comment
#              · ③ corpus→CAS by intent_id · ④(R2) sidecar is non-authoritative
# Oracle: crates/hugit-app/sidecar/tests/acceptance_wp-b6.rs must exist with
#   item_1_*, item_2_*, item_3_*, item_4_* tests;
#   cargo test -p hugit-app-sidecar --test acceptance_wp-b6 green.
# RED on absent crate; structural checks are direct bash assertions.

SUITE_ID="wp-b6"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-app/sidecar
ACCEPTANCE_RS="$CRATE/tests/acceptance_wp-b6.rs"
WP_ID="wp-b6"
CRATE_NAME="hugit-app-sidecar"

# ── (a) claimed crate exists ─────────────────────────────────────────────────
check "① crate hugit-app/sidecar present (Cargo.toml)" \
  test -f "$CRATE/Cargo.toml"
check "① crate hugit-app/sidecar present (src/lib.rs)" \
  test -f "$CRATE/src/lib.rs"

# ── (a) acceptance test file committed ───────────────────────────────────────
check "① acceptance file committed: $ACCEPTANCE_RS" \
  test -f "$ACCEPTANCE_RS"

# ── (b) cargo test --list declares item_1..item_4 ────────────────────────────
export TESTLIST="$(cargo test -p "$CRATE_NAME" --test "acceptance_${WP_ID}" -- --list 2>/dev/null || true)"

check "① item_1 test declared (parsed_validated_rendered)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1'"
check "② item_2 test declared (malformed_actionable_comment)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2'"
check "③ item_3 test declared (corpus_cas_by_intent_id)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_3'"
check "④ item_4 test declared (non_authoritative_never_gates)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_4'"

# ── (c) cargo test green ─────────────────────────────────────────────────────
check "①②③④ cargo test -p hugit-app-sidecar --test acceptance_wp-b6 green" \
  cargo test -p "$CRATE_NAME" --test "acceptance_${WP_ID}"

# ── (d) structural: authoritative field is hard-false ────────────────────────
# ④(R2): the sidecar is NON-AUTHORITATIVE — no code path can set authoritative
# to true. Assert the literal `authoritative: true` never appears in sidecar src.
check "④ 'authoritative: true' absent from sidecar src (hard-false)" \
  bash -c "! grep -rq 'authoritative: true' '$CRATE/src/'"

# ── (d) structural: malformed path produces a comment (② — no silent drop) ──
check "② malformed-comment handler present in sidecar src" \
  bash -c "grep -rq 'malformed\|actionable\|validation_failure\|invalid_sidecar' '$CRATE/src/'"

# ── (d) structural: CAS write path references intent_id (③) ─────────────────
check "③ intent_id referenced in sidecar src (CAS keying)" \
  bash -c "grep -rq 'intent_id' '$CRATE/src/'"

# ── (d) structural: Claims boundary — no write to hugit-app root or ui ───────
check "① sidecar.rs not leaked into hugit-app root src (Claims disjointness)" \
  bash -c "! test -f 'crates/hugit-app/src/sidecar.rs'"
check "① sidecar does not reference hugit-app/ui module (Claims disjointness)" \
  bash -c "! grep -rq 'hugit-app/ui\|hugit_app_ui' '$CRATE/src/'"

finish
