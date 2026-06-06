#!/usr/bin/env bash
# WP-D12 acceptance suite — regen gate.
# Contract: docs/plan/wp-contracts/WP-D12.md
# Owned items:
#   ① regen only on opt-in scope; non-opted repo never regens
#   ② regen lands only if acceptance re-passes AND fresh independent adversarial verdict approves
#   ③ missing/failing either → blocked + reported
#   ④ every regen auditable as its own revision, whose attestation RECORDS the authorizing gate-verdict ref (provenance closure)
#   ⑤ (R5) anti-smuggling: a file not provably derived CANNOT be classified derived — bypass via false "derived" declaration is blocked + audited
# Claims: crates/hugit-checks/regen/gate/
# Oracle: crates/hugit-checks/tests/acceptance_d12.rs
#   one #[test] item_<n>_<slug> per owned item.
# Gate-binding items ②③④⑤ verified via state-machine + attestation fixture proofs (local).
# RED on current tree (crate paths not yet built).

export TESTLIST="item_1_optin_scope_only item_2_both_preconditions_land item_3_missing_either_blocked_reported item_4_regen_auditable_verdict_ref item_5_false_derived_blocked_audited"

SUITE_ID="wp-d12"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-checks"
CRATE_NAME="hugit-checks"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_d12.rs"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-checks crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-checks Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-checks is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-checks"'

# ── acceptance oracle file ────────────────────────────────────────────────────
check "acceptance_d12.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ─────────────────────────────────────────────────────────
check "regen/gate/ subtree present" \
  test -d "$CRATE_ROOT/src/regen/gate"

# ── ① opt-in scope only; non-opted repo never regens ─────────────────────────
# State-machine fixture: non-opted repo → assert zero regen actions.
check "① item_1_optin_scope_only declared in acceptance_d12" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d12 -- --list 2>/dev/null | \
   grep -q "item_1_optin_scope_only"'

printf '%s\n' "$TESTLIST" | grep -q item_1
check "① TESTLIST contains item_1_optin_scope_only" \
  printf '%s\n' "$TESTLIST" | grep -q "item_1_optin_scope_only"

# ── ② both preconditions required for land (acceptance re-pass + fresh verdict) ─
# State-machine fixture: acceptance re-passes AND independent verdict APPROVES → land.
check "② item_2_both_preconditions_land declared in acceptance_d12" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d12 -- --list 2>/dev/null | \
   grep -q "item_2_both_preconditions_land"'

printf '%s\n' "$TESTLIST" | grep -q item_2
check "② TESTLIST contains item_2_both_preconditions_land" \
  printf '%s\n' "$TESTLIST" | grep -q "item_2_both_preconditions_land"

# ── ③ missing/failing either → blocked + reported ────────────────────────────
# Fixture: failing re-pass → blocked; missing verdict → blocked; both reported.
check "③ item_3_missing_either_blocked_reported declared in acceptance_d12" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d12 -- --list 2>/dev/null | \
   grep -q "item_3_missing_either_blocked_reported"'

printf '%s\n' "$TESTLIST" | grep -q item_3
check "③ TESTLIST contains item_3_missing_either_blocked_reported" \
  printf '%s\n' "$TESTLIST" | grep -q "item_3_missing_either_blocked_reported"

# ── ④ every regen auditable as its own revision; AttestationChain records gate-verdict ref ─
# Attestation fixture proof: assert ref is present and resolves (provenance closure).
check "④ item_4_regen_auditable_verdict_ref declared in acceptance_d12" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d12 -- --list 2>/dev/null | \
   grep -q "item_4_regen_auditable_verdict_ref"'

printf '%s\n' "$TESTLIST" | grep -q item_4
check "④ TESTLIST contains item_4_regen_auditable_verdict_ref" \
  printf '%s\n' "$TESTLIST" | grep -q "item_4_regen_auditable_verdict_ref"

# ── ⑤ anti-smuggling: false "derived" declaration blocked + audited ───────────
# Fixture: drive a false-derived declaration; assert block + audit EventRecord.
check "⑤ item_5_false_derived_blocked_audited declared in acceptance_d12" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d12 -- --list 2>/dev/null | \
   grep -q "item_5_false_derived_blocked_audited"'

printf '%s\n' "$TESTLIST" | grep -q item_5
check "⑤ TESTLIST contains item_5_false_derived_blocked_audited" \
  printf '%s\n' "$TESTLIST" | grep -q "item_5_false_derived_blocked_audited"

# ── full acceptance suite green ───────────────────────────────────────────────
check "cargo test -p hugit-checks --test acceptance_d12 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_d12

finish
