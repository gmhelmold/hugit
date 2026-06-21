#!/usr/bin/env bash
# WP-E2b acceptance suite — PR/issue → proposed intents + fidelity contract.
# Acceptance harness for WP-E2b.
# Owned items:
#   ② PRs/issues → proposed intents w/ provenance
#   ⑥(R3) PR/issue fidelity contract: stated set (body, comment/review threads, state,
#          labels, cross-refs) preserved with per-element provenance; non-imported elements
#          explicitly enumerated; verified on a fixture containing each element
# Oracle: crates/hugit-mirror/tests/acceptance_e2b.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-mirror/src/import/prissue/ — disjoint from other mirror modules.
#
# All items use fixture proofs (no live GitHub calls required for correctness).
# Import boundary: bare commits MUST NOT produce intents — that law is E2a⑤;
#   E2b asserts only that PR/issue intents are flagged proposed/non-authoritative.
# RED on current tree (import/prissue/ subtree not yet built).

SUITE_ID="wp-e2b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-mirror"
CRATE_NAME="hugit-mirror"
PRISSUE_DIR="$CRATE_ROOT/src/import/prissue"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e2b.rs"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-mirror crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-mirror Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-mirror is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-mirror"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "② import/prissue/ subtree present (Claims: src/import/prissue/)" \
  test -d "$PRISSUE_DIR"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_e2b.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── barrel export: prissue module exported from lib.rs ───────────────────────
check "② lib.rs barrel exports import::prissue" \
  bash -c "grep -q 'prissue' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_e2b -- --list 2>/dev/null || true)"

# ── ② PRs/issues → proposed intents w/ provenance ────────────────────────────
check "② item_2_proposed_intents_with_provenance declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_proposed_intents_with_provenance'"

# ── ② proposed/non-authoritative flag on every imported PR/issue intent ───────
check "② item_2_intents_flagged_proposed_non_authoritative declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_intents_flagged_proposed_non_authoritative'"

# ── ⑥ fidelity: body preserved with provenance ───────────────────────────────
check "⑥ item_6_fidelity_body_preserved declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_fidelity_body_preserved'"

# ── ⑥ fidelity: comment/review threads preserved with provenance ──────────────
check "⑥ item_6_fidelity_comment_threads_preserved declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_fidelity_comment_threads_preserved'"

# ── ⑥ fidelity: state preserved with provenance ──────────────────────────────
check "⑥ item_6_fidelity_state_preserved declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_fidelity_state_preserved'"

# ── ⑥ fidelity: labels preserved with provenance ─────────────────────────────
check "⑥ item_6_fidelity_labels_preserved declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_fidelity_labels_preserved'"

# ── ⑥ fidelity: cross-refs preserved or recorded as residual ─────────────────
check "⑥ item_6_fidelity_crossrefs_preserved declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_fidelity_crossrefs_preserved'"

# ── ⑥ fidelity: non-imported elements explicitly enumerated (no silent drop) ──
check "⑥ item_6_non_imported_elements_enumerated declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6_non_imported_elements_enumerated'"

# ── ② import boundary: bare commit path produces no PR/issue intent ───────────
check "② item_2_bare_commit_produces_no_prissue_intent declared in acceptance_e2b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_bare_commit_produces_no_prissue_intent'"

# ── structural: IntentSidecar referenced in prissue/ src (contract dep) ───────
check "② IntentSidecar referenced in prissue/ src (contract dep)" \
  bash -c "grep -rEq 'IntentSidecar' '$PRISSUE_DIR/' 2>/dev/null"

# ── structural: proposed/non-authoritative flag in prissue/ src ───────────────
check "② proposed/non-authoritative state flag present in prissue/ src" \
  bash -c "grep -rEq 'proposed|non.authoritative|NonAuthoritative' '$PRISSUE_DIR/' 2>/dev/null"

# ── structural: per-element provenance attachment in prissue/ src ─────────────
check "② per-element provenance attached in prissue/ src" \
  bash -c "grep -rEq 'provenance|Provenance|source_url|element_origin' '$PRISSUE_DIR/' 2>/dev/null"

# ── structural: non-imported element enumeration published in prissue/ src ────
check "⑥ non-imported elements enumeration present in prissue/ src" \
  bash -c "grep -rEq 'NON_IMPORTED|non_imported|NotImported|excluded' '$PRISSUE_DIR/' 2>/dev/null"

# ── structural: dangling cross-ref residual (not fabricated) in prissue/ src ──
check "⑥ dangling cross-ref recorded as residual (not fabricated) in prissue/ src" \
  bash -c "grep -rEq 'dangling|residual|Residual|unresolved' '$PRISSUE_DIR/' 2>/dev/null"

# ── structural: Claims disjointness — prissue/ does not reference history/ ────
check "② prissue/ does not bleed into history module (disjoint Claims)" \
  bash -c "! grep -rEq 'import::history|mod history' '$PRISSUE_DIR/' 2>/dev/null"

# ── structural: prissue/ does not reference bare-commit import path ────────────
check "② prissue/ does not synthesize intents from bare commits" \
  bash -c "! grep -rEq 'bare.commit|BareCommit|from_commit\b' '$PRISSUE_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "②⑥ cargo test -p hugit-mirror --test acceptance_e2b green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e2b

finish
