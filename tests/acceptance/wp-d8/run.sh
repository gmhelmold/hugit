#!/usr/bin/env bash
# WP-D8 acceptance suite — experiment harness + gate binding.
# Contract: docs/plan/wp-contracts/WP-D8.md
# Owned items:
#   ① every wave auto-contributes datapoints
#   ② dashboard: disjointness %, regen agree/disagree, n
#   ③ gate report generated, never hand-written
#   ④ (R2) anti-gaming: corpus pre-registered + SEALED before evaluation; sample selection auditable
#   ⑤ (R2) post-hoc removal detected → invalidates the verdict
#   ⑥ THE GATE BINDS: claims-as-oracle pinned advisory/OFF; regen promotion blocked until PASS;
#      FAIL/insufficient-n CANNOT flip either; DEGRADED evaluator = insufficient → CLOSED;
#      promotion event audited
#   ⑦ (R6) degradation honesty: degraded-window waves excluded or explicitly marked; never silent
#   ⑧ (R9) source-eligibility wired to focus gate: corelink-server rejected at INGESTION, fail-closed + audited
#   ⑨ (R10) gate report is attested tamper-evident object (X2-class); forged PASS cannot promote
# Claims: crates/hugit-diag/experiment/
# Oracle: crates/hugit-diag/tests/acceptance_d8.rs
#   one #[test] item_<n>_<slug> per owned item.
# Gate-binding items ⑥⑧⑨ verified via state-machine + attestation fixture proofs (local).
# RED on current tree (crate paths not yet built).

export TESTLIST="item_1_wave_auto_contributes item_2_dashboard_fields item_3_report_generated_not_handwritten item_4_corpus_presealed_auditable item_5_posthoc_removal_invalidates item_6_gate_binds_fail_closed item_7_degradation_honesty item_8_ineligible_source_rejected item_9_report_attested_tamper_evident"

SUITE_ID="wp-d8"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-diag"
CRATE_NAME="hugit-diag"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_d8.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-diag crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-diag Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-diag is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-diag"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_d8.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "experiment/ subtree present" \
  test -d "$CRATE_ROOT/src/experiment"

# ── structural negative: no writes outside experiment/ ──────────────────────
check "no WP-D8 files leaked outside experiment/ subtree" bash -c \
  '! find '"$CRATE_ROOT"'/src -maxdepth 1 -name "*.rs" -newer '"$CRATE_ROOT"'/Cargo.toml \
   2>/dev/null | grep -q .'

# ── ① wave auto-contributes datapoints ──────────────────────────────────────
check "① item_1_wave_auto_contributes declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_1_wave_auto_contributes"'

# ── ② dashboard fields ───────────────────────────────────────────────────────
check "② item_2_dashboard_fields declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_2_dashboard_fields"'

# ── ③ report generated, never hand-written ───────────────────────────────────
check "③ item_3_report_generated_not_handwritten declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_3_report_generated_not_handwritten"'

# ── ④ corpus pre-sealed + auditable ─────────────────────────────────────────
check "④ item_4_corpus_presealed_auditable declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_4_corpus_presealed_auditable"'

# ── ⑤ post-hoc removal detected → invalidates ───────────────────────────────
check "⑤ item_5_posthoc_removal_invalidates declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_5_posthoc_removal_invalidates"'

# ── ⑥ gate binds fail-closed (state-machine fixture) ────────────────────────
# Verifies: FAIL-report cannot promote, insufficient-n cannot promote,
# DEGRADED evaluator = insufficient, promotion event audited.
check "⑥ item_6_gate_binds_fail_closed declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_6_gate_binds_fail_closed"'

# ── ⑦ degradation honesty ────────────────────────────────────────────────────
check "⑦ item_7_degradation_honesty declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_7_degradation_honesty"'

# ── ⑧ ineligible source rejected at ingestion (focus-gate wired, fail-closed) ─
# State-machine fixture: corelink-server ingestion attempt → rejected + audited.
check "⑧ item_8_ineligible_source_rejected declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_8_ineligible_source_rejected"'

# ── ⑨ gate report attested tamper-evident (attestation fixture) ──────────────
# Fixture: present a forged PASS report; assert promotion/billing rejected.
check "⑨ item_9_report_attested_tamper_evident declared in acceptance_d8" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d8 -- --list 2>/dev/null | \
   grep -q "item_9_report_attested_tamper_evident"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-diag --test acceptance_d8 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_d8

finish
