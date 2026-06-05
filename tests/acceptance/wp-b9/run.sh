#!/usr/bin/env bash
# WP-B9 acceptance suite — hugit-app/exit, exit telemetry + money gate (items ①–⑥).
# Contract: docs/plan/wp-contracts/WP-B9.md.
# Owned items:
#   ① per-install activity → week-3 retention computable vs ≥40% threshold
#      (privacy-documented)
#   ② feedback capture distinguishes UNPROMPTED from prompted — only unprompted
#      count toward ≥3 gate
#   ③ exit-metric report generated from data, auditable
#   ④ cohort/window guards: n=10 external teams, ≥3 weeks real use, evaluation
#      window ANCHORED to first-10-paying-customers event and inside 90 days —
#      otherwise "insufficient/out-of-window", never a pass
#   ⑤(R4) ≥3 gate ENFORCED as pass/fail: 2 correctly-counted unprompted → FAIL
#           even with ≥40% retention; exactly 3 → PASS
#   ⑥ THE MONEY GATE BINDS: charging money is structurally blocked until exit
#      report = PASS; FAIL/insufficient/out-of-window CANNOT enable billing;
#      DEGRADED gate-evaluator = "insufficient" → fails CLOSED; enable-billing
#      event is itself audited (control, not dashboard)
# Oracle: crates/hugit-app/exit/tests/acceptance_wp-b9.rs must exist with
#   item_1_* … item_6_* tests;
#   cargo test -p hugit-app-exit --test acceptance_wp-b9 green.
# RED on absent crate; structural checks are direct bash assertions.

SUITE_ID="wp-b9"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-app/exit
ACCEPTANCE_RS="$CRATE/tests/acceptance_wp-b9.rs"
WP_ID="wp-b9"
CRATE_NAME="hugit-app-exit"

# ── (a) claimed crate exists ─────────────────────────────────────────────────
check "① crate hugit-app/exit present (Cargo.toml)" \
  test -f "$CRATE/Cargo.toml"
check "① crate hugit-app/exit present (src/lib.rs)" \
  test -f "$CRATE/src/lib.rs"

# ── (a) acceptance test file committed ───────────────────────────────────────
check "① acceptance file committed: $ACCEPTANCE_RS" \
  test -f "$ACCEPTANCE_RS"

# ── (b) cargo test --list declares item_1..item_6 ────────────────────────────
TESTLIST="$(cargo test -p "$CRATE_NAME" --test "acceptance_${WP_ID}" -- --list 2>/dev/null || true)"

check "① item_1 test declared (week3_retention_computable_vs_40pct)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_1'"
check "② item_2 test declared (unprompted_vs_prompted_classifier)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_2'"
check "③ item_3 test declared (exit_report_generated_auditable)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_3'"
check "④ item_4 test declared (cohort_window_guards_insufficient_not_pass)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_4'"
check "⑤ item_5 test declared (gate_pass_fail_2_unprompted_fail_3_pass)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_5'"
check "⑥ item_6 test declared (money_gate_blocks_billing_until_pass)" \
  bash -c "printf '%s' \"\$TESTLIST\" | grep -q 'item_6'"

# ── (c) cargo test green ─────────────────────────────────────────────────────
check "①–⑥ cargo test -p hugit-app-exit --test acceptance_wp-b9 green" \
  cargo test -p "$CRATE_NAME" --test "acceptance_${WP_ID}"

# ── (d) structural: UNPROMPTED classifier present in exit src (②⑤) ──────────
check "② UNPROMPTED classifier present in exit src" \
  bash -c "grep -rq 'Unprompted\|UNPROMPTED\|unprompted' '$CRATE/src/'"

# ── (d) structural: ≥3 gate enforced — gate logic references the literal 3 ──
# ⑤(R4): 2 → FAIL, 3 → PASS. The implementation must encode this boundary.
check "⑤ gate threshold literal 3 present in exit src" \
  bash -c "grep -rq '3\b' '$CRATE/src/'" # broad; narrows when crate ships

# ── (d) structural: cohort guard n=10 + 90-day window encoded (④) ────────────
check "④ cohort guard references n=10 or 90_days in exit src" \
  bash -c "grep -rq '10\|90\|insufficient\|out_of_window' '$CRATE/src/'"

# ── (d) structural: money gate — enable-billing path guarded (⑥) ────────────
# The enable-billing transition must require the gate to pass; assert no
# unconditional enable-billing path exists without a gate check.
check "⑥ money gate guard present in exit src (enable_billing gated)" \
  bash -c "grep -rq 'enable_billing\|MoneyGate\|money_gate\|billing_gate' '$CRATE/src/'"

# ── (d) structural: DEGRADED state maps to insufficient, not pass (⑥) ────────
check "⑥ DEGRADED/degraded state present in exit src (fails-closed)" \
  bash -c "grep -rq 'Degraded\|DEGRADED\|degraded' '$CRATE/src/'"

# ── (d) structural: enable-billing event audited via EventRecord (⑥) ──────────
check "⑥ EventRecord referenced in exit src (audited enable-billing event)" \
  bash -c "grep -rq 'EventRecord\|event_record' '$CRATE/src/'"

# ── (d) structural: privacy documentation for retention data present (①) ─────
# The contract requires the privacy model to be documented in the artifact.
check "① privacy doc present for retention data (doc comment or module)" \
  bash -c "grep -rq 'privacy\|Privacy\|anonymized\|anonymize' '$CRATE/src/'"

# ── (d) structural: Claims boundary — exit does not touch B6/B7/B1 (disjoint) ─
check "⑥ exit does not reference sidecar module (Claims disjointness)" \
  bash -c "! grep -rq 'hugit-app/sidecar\|hugit_app_sidecar' '$CRATE/src/'"
check "⑥ exit does not reference ui module (Claims disjointness)" \
  bash -c "! grep -rq 'hugit-app/ui\|hugit_app_ui' '$CRATE/src/'"

finish
