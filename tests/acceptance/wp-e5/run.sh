#!/usr/bin/env bash
# WP-E5 acceptance suite — export + exit proofs (anti-lock-in guarantee).
# Contract: docs/plan/wp-contracts/WP-E5.md
# Owned items:
#   ① one-command dump git + documented JSON
#   ② restore round-trip reproduces refs+intents+events
#   ③ export validates against versioned ExportSchema (machine check)
#   ④ export applies context/journal redaction policy (no secret material emitted); multi-GB streams without OOM
#   ⑤ (R2) THE EXIT PROOF: exported git artifact (and mirror) fully usable with ZERO hugit/forge dependency
#   ⑥ (R3) completeness: export+restore reproduces ALL first-class object classes object-for-object; out-of-scope classes explicitly enumerated in ExportSchema
#   ⑦ (R3) redaction red-team: seeded secrets appear NOWHERE in exported git/JSON; redacted artifact still passes exit proof, removals manifested
#   ⑧ (R3) exit under exit conditions: export succeeds on suspended/past-due/offboarding account (read-only terminating path)
#   ⑨ (R4) "any moment" consistency: export under concurrent mutation yields ONE point-in-time-consistent cut — no dangling provenance link, no event referencing absent object; restore is self-consistent
# Claims: crates/hugit-cli/src/export/ + tests/export/
# Oracle: crates/hugit-cli/tests/acceptance_e5.rs
#   one #[test] item_<n>_<slug> per owned item.
# Exit proof (⑤) runs fixture with NO hugit tooling on PATH (local proof).
# RED on current tree (crate paths not yet built).

export TESTLIST="item_1_one_command_dump item_2_restore_roundtrip item_3_schema_validates item_4_redaction_no_oom item_5_exit_proof_zero_hugit item_6_completeness_all_classes item_7_redaction_redteam item_8_terminating_account item_9_live_consistency_cut"

SUITE_ID="wp-e5"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-cli"
CRATE_NAME="hugit-cli"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e5.rs"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-cli crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-cli Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-cli is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-cli"'

# ── acceptance oracle file ────────────────────────────────────────────────────
check "acceptance_e5.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ─────────────────────────────────────────────────────────
check "export/ subtree present under hugit-cli/src" \
  test -d "$CRATE_ROOT/src/export"

# ── ① one-command dump: git + documented JSON ─────────────────────────────────
check "① item_1_one_command_dump declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_1_one_command_dump"'

printf '%s\n' "$TESTLIST" | grep -q item_1
check "① TESTLIST contains item_1_one_command_dump" \
  printf '%s\n' "$TESTLIST" | grep -q "item_1_one_command_dump"

# ── ② restore round-trip: refs+intents+events reproduced ─────────────────────
check "② item_2_restore_roundtrip declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_2_restore_roundtrip"'

printf '%s\n' "$TESTLIST" | grep -q item_2
check "② TESTLIST contains item_2_restore_roundtrip" \
  printf '%s\n' "$TESTLIST" | grep -q "item_2_restore_roundtrip"

# ── ③ export validates against versioned ExportSchema (machine check) ─────────
check "③ item_3_schema_validates declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_3_schema_validates"'

printf '%s\n' "$TESTLIST" | grep -q item_3
check "③ TESTLIST contains item_3_schema_validates" \
  printf '%s\n' "$TESTLIST" | grep -q "item_3_schema_validates"

# ── ④ redaction policy applied; multi-GB streams without OOM ─────────────────
check "④ item_4_redaction_no_oom declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_4_redaction_no_oom"'

printf '%s\n' "$TESTLIST" | grep -q item_4
check "④ TESTLIST contains item_4_redaction_no_oom" \
  printf '%s\n' "$TESTLIST" | grep -q "item_4_redaction_no_oom"

# ── ⑤ THE EXIT PROOF: zero hugit/forge dependency on PATH ────────────────────
# Local proof: run clone/log/branch/push-elsewhere on exported artifact with
# no hugit tooling on PATH; assert all git operations succeed.
check "⑤ item_5_exit_proof_zero_hugit declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_5_exit_proof_zero_hugit"'

printf '%s\n' "$TESTLIST" | grep -q item_5
check "⑤ TESTLIST contains item_5_exit_proof_zero_hugit" \
  printf '%s\n' "$TESTLIST" | grep -q "item_5_exit_proof_zero_hugit"

# ── ⑥ completeness: all first-class classes reproduced object-for-object ──────
# refs, intents, events, ledger, verdicts, journals, policy, provenance links;
# out-of-scope classes explicitly enumerated in ExportSchema.
check "⑥ item_6_completeness_all_classes declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_6_completeness_all_classes"'

printf '%s\n' "$TESTLIST" | grep -q item_6
check "⑥ TESTLIST contains item_6_completeness_all_classes" \
  printf '%s\n' "$TESTLIST" | grep -q "item_6_completeness_all_classes"

# ── ⑦ redaction red-team: seeded secrets absent; redacted artifact passes exit proof; removals manifested ─
check "⑦ item_7_redaction_redteam declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_7_redaction_redteam"'

printf '%s\n' "$TESTLIST" | grep -q item_7
check "⑦ TESTLIST contains item_7_redaction_redteam" \
  printf '%s\n' "$TESTLIST" | grep -q "item_7_redaction_redteam"

# ── ⑧ terminating account: export succeeds on suspended/past-due/offboarding ──
# Read-only terminating path; exit is never blocked by account state.
check "⑧ item_8_terminating_account declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_8_terminating_account"'

printf '%s\n' "$TESTLIST" | grep -q item_8
check "⑧ TESTLIST contains item_8_terminating_account" \
  printf '%s\n' "$TESTLIST" | grep -q "item_8_terminating_account"

# ── ⑨ live consistency: one point-in-time-consistent cut under concurrent mutation ─
# No dangling provenance link, no event referencing absent object; restore self-consistent.
check "⑨ item_9_live_consistency_cut declared in acceptance_e5" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_e5 -- --list 2>/dev/null | \
   grep -q "item_9_live_consistency_cut"'

printf '%s\n' "$TESTLIST" | grep -q item_9
check "⑨ TESTLIST contains item_9_live_consistency_cut" \
  printf '%s\n' "$TESTLIST" | grep -q "item_9_live_consistency_cut"

# ── full acceptance suite green ───────────────────────────────────────────────
check "cargo test -p hugit-cli --test acceptance_e5 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e5

finish
