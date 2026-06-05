#!/usr/bin/env bash
# WP-00 acceptance suite — hugit-contracts crate (all 15 frozen types).
# Contract: docs/plan/wp-contracts/WP-00.md. Green = every frozen contract
# ships as Rust type + committed JSON Schema + golden serde round-trip,
# with a schema-drift assertion. RED on an empty/absent crate.
#
# Naming convention (pre-decided by the lead, binding for the agent):
#   golden test  → golden_<snake_case_type>   (e.g. golden_check_def)
#   drift test   → schema_drift
#   schema file  → schemas/<TypeName>.json
#   golden file  → tests/golden/<TypeName>.json

SUITE_ID="wp-00"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

TYPES=(CheckDef CheckResult DiagnosisObject IntentSidecar RunnerLease
       FenceManifest EventRecord VerdictObject QueueApi AppWebhooks
       ShadowPolicy AttentionRank ExportSchema AttestationChain RegenGate)

CRATE=crates/hugit-contracts

# ── ① the crate exists in the workspace ─────────────────────────────────────
check "hugit-contracts crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"
check "crate is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 | jq -e \
   '"'"'.packages[] | select(.name == "hugit-contracts")'"'"' >/dev/null'

# ── ② 15 committed JSON Schemas, one per frozen type, valid JSON ────────────
for t in "${TYPES[@]}"; do
  check "schema committed + valid: $t" \
    bash -c "jq -e . $CRATE/schemas/$t.json >/dev/null"
done

# ── ③ 15 golden fixtures committed ──────────────────────────────────────────
for t in "${TYPES[@]}"; do
  check "golden fixture committed: $t" test -f "$CRATE/tests/golden/$t.json"
done

# ── ④ golden round-trip test per type + schema-drift test, all green ────────
snake() { echo "$1" | sed -E 's/([a-z0-9])([A-Z])/\1_\2/g' | tr '[:upper:]' '[:lower:]'; }
TESTLIST="$(cargo test -p hugit-contracts -- --list 2>/dev/null || true)"
for t in "${TYPES[@]}"; do
  name="golden_$(snake "$t")"
  check "golden round-trip test declared: $name" \
    bash -c "echo '$TESTLIST' | grep -q '$name'"
done
check "schema-drift test declared (schema_drift)" \
  bash -c "echo '$TESTLIST' | grep -q 'schema_drift'"
check "cargo test -p hugit-contracts green" cargo test -p hugit-contracts

# ── ⑤ conventions: deny_unknown_fields on every contract type ───────────────
check "serde(deny_unknown_fields) used in src" \
  bash -c "grep -rq 'deny_unknown_fields' $CRATE/src/"

finish
