#!/usr/bin/env bash
# WP-D14 acceptance suite — hugit-refstore forge authz.
# Acceptance harness for WP-D14.
# Owned items (verbatim):
#   ① mutating endpoints (push/land/undo/policy) reject unauthorized principals
#   ② permission model documented + golden-tested per principal class
#   ③ authz denials audited
# Oracle: crates/hugit-refstore/tests/acceptance_d14.rs
#   Tests: item_1_mutating_endpoints_reject_unauthorized,
#          item_2_permission_model_golden_per_principal_class,
#          item_3_authz_denials_audited

SUITE_ID="wp-d14"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-refstore

# ── structural: crate exists ─────────────────────────────────────────────────
check "hugit-refstore crate present" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── structural: D14-owned module path exists ─────────────────────────────────
check "authz/ module present under crate root" \
  test -d "$CRATE/authz"

# ── structural: golden per-principal fixtures directory exists ────────────────
check "authz/tests/ directory present (golden fixtures)" \
  test -d "$CRATE/authz/tests"

# ── structural: authz covers all four mutating endpoints (grep portable) ──────
check "① push endpoint referenced in authz module" \
  bash -c "grep -r 'push' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"
check "① land endpoint referenced in authz module" \
  bash -c "grep -r 'land' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"
check "① undo endpoint referenced in authz module" \
  bash -c "grep -r 'undo' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"
check "① policy endpoint referenced in authz module" \
  bash -c "grep -r 'policy' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"

# ── structural: all four principal classes referenced (grep portable) ─────────
check "② human principal class referenced in authz module" \
  bash -c "grep -r 'human' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"
check "② orchestrator principal class referenced in authz module" \
  bash -c "grep -r 'orchestrator' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"
check "② worker principal class referenced in authz module" \
  bash -c "grep -r 'worker' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"
check "② model principal class referenced in authz module" \
  bash -c "grep -r 'model' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"

# ── structural: denial audit event emission referenced ───────────────────────
check "③ denial audit EventRecord emission referenced in authz module" \
  bash -c "grep -r 'EventRecord' $CRATE/authz/ --include='*.rs' -l | grep -q '.'"

# ── negative: D14 must not own D1/D3/D4/D6 internals (leak guard) ────────────
check "authz/ does not contain log/ internals (no leak into D1a)" \
  bash -c "! test -d $CRATE/authz/log"
check "authz/ does not contain compaction/ internals (no leak into D1b)" \
  bash -c "! test -d $CRATE/authz/compaction"
check "authz/ does not contain concurrency/ internals (no leak into D1c)" \
  bash -c "! test -d $CRATE/authz/concurrency"

# ── oracle: acceptance file committed ────────────────────────────────────────
check "acceptance_d14.rs oracle committed" \
  test -f "$CRATE/tests/acceptance_d14.rs"

# ── oracle: item tests declared via cargo test --list ────────────────────────
TESTLIST="$(cargo test -p hugit-refstore --test acceptance_d14 -- --list 2>/dev/null || true)"

check "① item_1_mutating_endpoints_reject_unauthorized declared" \
  bash -c "printf '%s' '$TESTLIST' | grep -q 'item_1_mutating_endpoints_reject_unauthorized'"
check "② item_2_permission_model_golden_per_principal_class declared" \
  bash -c "printf '%s' '$TESTLIST' | grep -q 'item_2_permission_model_golden_per_principal_class'"
check "③ item_3_authz_denials_audited declared" \
  bash -c "printf '%s' '$TESTLIST' | grep -q 'item_3_authz_denials_audited'"

# ── oracle: acceptance suite runs green ──────────────────────────────────────
check "①②③ acceptance suite green (cargo test -p hugit-refstore --test acceptance_d14)" \
  cargo test -p hugit-refstore --test acceptance_d14

finish
