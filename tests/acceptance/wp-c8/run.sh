#!/usr/bin/env bash
# WP-C8 acceptance suite — shadow checks (snapshot-cadence, budget-capped, non-gating).
# Contract: docs/plan/wp-contracts/WP-C8.md
# Owned items:
#   ① N writes in one snapshot window → exactly ONE shadow pass at boundary
#      (not N, not 0)
#   ② shadow runs decrement tenant budget; cap halts shadows, explicit jobs
#      proceed per policy
#   ③ default-off; per-repo opt-in flag scoped to repo, zero runs when off
#   ④(R2) a failing shadow surfaces as signal/event and NEVER gates/blocks/fails
#          any explicit job; a passing shadow produces an observable result
#   ⑤(R2) per-tenant cap isolation: tenant A exhausting its shadow budget leaves
#          tenant B's shadows unaffected
# Claims: crates/hugit-checks/shadow/
# Oracle: crates/hugit-checks/tests/acceptance_c8.rs
#   one #[test] item_<n>_<slug> per owned item.
# No box-env dependency for pure policy/scheduler logic (no HUGIT_RUNNER_HOST export).
# RED on current tree (crate path not yet built).

SUITE_ID="wp-c8"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-checks"
CRATE_NAME="hugit-checks"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c8.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-checks crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-checks Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-checks is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-checks"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_c8.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "shadow/ subtree present" \
  test -d "$CRATE_ROOT/src/shadow"

# ── structural negatives: sibling C4/B3 paths must be untouched by C8 ────────
check "regen/ subtree belongs to C4 — no new files from C8" bash -c \
  '! find '"$CRATE_ROOT"'/src/regen -name "*.rs" -newer '"$CRATE_ROOT"'/Cargo.toml \
   2>/dev/null | grep -q .'

check "affected/ subtree belongs to B3 — no new files from C8" bash -c \
  '! find '"$CRATE_ROOT"'/src/affected -name "*.rs" -newer '"$CRATE_ROOT"'/Cargo.toml \
   2>/dev/null | grep -q .'

# ── item declaration check via --list ────────────────────────────────────────
export TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_c8 -- --list 2>/dev/null || true)"

check "① item_1_one_shadow_pass_per_snapshot_window declared in acceptance_c8" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1'"

check "② item_2_shadow_decrements_budget_cap_halts_shadows declared in acceptance_c8" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2'"

check "③ item_3_default_off_per_repo_optin_zero_when_off declared in acceptance_c8" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3'"

check "④ item_4_failing_shadow_signal_only_never_gates_explicit_job declared in acceptance_c8" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4'"

check "⑤ item_5_per_tenant_cap_isolation declared in acceptance_c8" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5'"

# ── structural/contract asserts on shadow/ src ───────────────────────────────

# ① snapshot window boundary as the single trigger: N writes → one pass
check "① snapshot-window boundary trigger referenced in shadow/ src" bash -c \
  "grep -rEq 'snapshot_window|SnapshotWindow|snapshot_boundary|window_boundary' \
   '$CRATE_ROOT/src/shadow/'"

check "① collapse N writes to one pass — dedup/coalesce referenced in shadow/ src" bash -c \
  "grep -rEq 'dedup|coalesce|once_per_window|collapse|one_pass' \
   '$CRATE_ROOT/src/shadow/'"

# ② budget decrement + cap halt present; explicit jobs proceed
check "② budget decrement referenced in shadow/ src" bash -c \
  "grep -rEq 'decrement|budget_decrement|BudgetDecrement|draw_budget' \
   '$CRATE_ROOT/src/shadow/'"

check "② cap-halt: shadow halts when budget exhausted, explicit jobs proceed" bash -c \
  "grep -rEq 'cap_halt|Cap|shadow_halt|halt_shadow|explicit.*proceed|proceed.*explicit' \
   '$CRATE_ROOT/src/shadow/'"

# ③ default-off + ShadowPolicy.optin scoped per repo
check "③ ShadowPolicy / optin flag referenced in shadow/ src" bash -c \
  "grep -rEq 'ShadowPolicy|optin|opt_in|default_off' \
   '$CRATE_ROOT/src/shadow/'"

check "③ zero shadow runs when opt-in is off — guard present in shadow/ src" bash -c \
  "grep -rEq 'zero.*run|no.*shadow|skip.*shadow|off.*return|!optin|not.*optin' \
   '$CRATE_ROOT/src/shadow/'"

# ④ non-gating: failing shadow is signal/event only
check "④ failing shadow surfaces as signal/event (not gate) in shadow/ src" bash -c \
  "grep -rEq 'signal|event|CheckResult|check_result|non.?gating|never.*gate' \
   '$CRATE_ROOT/src/shadow/'"

check "④ no path where shadow failure gates/blocks/fails an explicit job" bash -c \
  "! grep -rEq 'gate_on_shadow|block_explicit|fail_explicit|shadow_gate' \
   '$CRATE_ROOT/src/shadow/' 2>/dev/null"

# ⑤ per-tenant isolation: tenant A budget does not affect tenant B
check "⑤ per-tenant budget scoping referenced in shadow/ src" bash -c \
  "grep -rEq 'per_tenant|tenant_id|TenantId|tenant_budget|isolated.*budget|budget.*isolated' \
   '$CRATE_ROOT/src/shadow/'"

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-checks --test acceptance_c8 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c8

finish
