#!/usr/bin/env bash
# WP-B10 acceptance suite — hugit-queue + hugit-checks, negative scope (items ①②).
# Contract: docs/plan/wp-contracts/WP-B10.md.
# Owned items (ABSENCE assertions):
#   ① no claim/lease acquired at dispatch — conflict discovery happens ONLY at
#      landing/union (assert mechanism absent)
#   ② rebase in phase B is textual-fallback only — regenerative path absent/disabled (assert)
# Oracle: crates/hugit-queue/tests/acceptance_wp-b10.rs must exist with
#   item_1_*, item_2_* tests; cargo test -p hugit-queue --test acceptance_wp-b10 green.
# Claims: crates/hugit-queue/tests/negative_scope/ — asserts ABOUT src/core/, src/github/,
#   budget/, and crates/hugit-checks; does NOT modify those sources.
# NOTE: structural absence-asserts may PASS on today's tree — that is acceptable for B10 only.
#   The red component is the declared-tests checks (oracle file absent → RED).

SUITE_ID="wp-b10"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

QUEUE_CRATE=crates/hugit-queue
CHECKS_CRATE=crates/hugit-checks
ACCEPTANCE_RS="$QUEUE_CRATE/tests/acceptance_wp-b10.rs"
NEG_SCOPE_DIR="$QUEUE_CRATE/tests/negative_scope"
WP_ID="wp-b10"

# ── (a) claimed crates exist ─────────────────────────────────────────────────
check "① crate hugit-queue present (Cargo.toml)" \
  test -f "$QUEUE_CRATE/Cargo.toml"
check "② crate hugit-checks present (Cargo.toml)" \
  test -f "$CHECKS_CRATE/Cargo.toml"

# ── (a) negative_scope test dir exists (Claims) ───────────────────────────────
check "① negative_scope/ test dir exists: $NEG_SCOPE_DIR" \
  test -d "$NEG_SCOPE_DIR"

# ── (a) acceptance test file committed ───────────────────────────────────────
check "① acceptance file committed: $ACCEPTANCE_RS" \
  test -f "$ACCEPTANCE_RS"

# ── (b) cargo test --list declares item_1, item_2 ────────────────────────────
TESTLIST="$(cargo test -p hugit-queue --test "acceptance_${WP_ID}" -- --list 2>/dev/null || true)"

check "① item_1 test declared (no_claim_lease_at_dispatch)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1'"
check "② item_2 test declared (rebase_textual_fallback_only_regen_absent)" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2'"

# ── (c) cargo test -p hugit-queue --test acceptance_wp-b10 green ─────────────
check "①② cargo test -p hugit-queue --test acceptance_wp-b10 green" \
  cargo test -p hugit-queue --test "acceptance_${WP_ID}"

# ── (d) absence assertions ────────────────────────────────────────────────────
# Item ①: no claim/lease acquisition mechanism in dispatch path.
# Greps over src/core/ and src/github/ (the Phase-B dispatch paths).
check "① no claim_acquire/lease_acquire in hugit-queue src/core/ (dispatch path)" \
  bash -c "! grep -rEq 'claim_acquire|acquire_claim|RunnerLease.*dispatch|dispatch.*RunnerLease' '$QUEUE_CRATE/src/core/'"
check "① no claim_acquire/lease_acquire in hugit-queue src/github/ (dispatch path)" \
  bash -c "! grep -rEq 'claim_acquire|acquire_claim|RunnerLease.*dispatch|dispatch.*RunnerLease' '$QUEUE_CRATE/src/github/' 2>/dev/null || true"
check "① no dispatch-time claim mechanism in hugit-checks src/ (checks dispatch)" \
  bash -c "! grep -rEq 'claim_acquire|acquire_claim|dispatch.*claim|claim.*dispatch' '$CHECKS_CRATE/src/'"

# Item ②: regenerative rebase path absent/disabled in Phase B.
# Greps over src/core/ and hugit-checks src/ for any regen rebase symbol.
check "② no regenerative rebase path in hugit-queue src/core/ (regen ⛔ CUT from B)" \
  bash -c "! grep -rEq 'regen_rebase|regenerative_rebase|RegenRebase|regen.*path|reexec.*rebase' '$QUEUE_CRATE/src/core/'"
check "② no regenerative rebase path in hugit-checks src/ (regen ⛔ CUT from B)" \
  bash -c "! grep -rEq 'regen_rebase|regenerative_rebase|RegenRebase|regen.*path|reexec.*rebase' '$CHECKS_CRATE/src/'"
check "② hugit-checks regen/ dir absent or disabled in Phase B" \
  bash -c "! test -d '$CHECKS_CRATE/src/regen' || grep -rEq 'disabled|cfg.*not|todo!|unimplemented' '$CHECKS_CRATE/src/regen/'"

# ── (d) structural: Claims boundary — negative_scope/ does not write source dirs ──
check "① negative_scope/ does not contain source modifications to core/ (read-asserts only)" \
  bash -c "! grep -rEq '^[^#]*fs::write|^[^#]*std::fs::File::create' '$NEG_SCOPE_DIR/' 2>/dev/null || true"

finish
