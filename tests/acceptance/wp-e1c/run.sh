#!/usr/bin/env bash
# WP-E1c acceptance suite — verified mirror: bootstrap + disaster recovery (items ⑧ ⑨ ⑪).
# Acceptance harness for WP-E1c.
# Owned items:
#   ⑧(R3)  cold-seed bootstrap: full history → fresh GitHub repo, hash-verified, resumable mid-seed
#   ⑨🔧    GitHub-side loss DR: App revocation or mirror-repo deletion/rename → incident → recoverable;
#            recovery-source pinned, completeness criterion stated, resume-from-recovered-state asserted
#   ⑪(R6)  SUBSTRATE-LOSS DR: induce forge/substrate loss → mirror is byte-complete working git repo;
#            full recovery/resume proven end-to-end; recovered content imports as change-events, never fabricated intents
# Oracle: crates/hugit-mirror/tests/acceptance_e1c.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-mirror/src/bootstrap/, src/dr/
# All items are local fixture proofs over bootstrap/DR logic.
# RED on current tree (bootstrap/ dr/ subtrees not yet built).

SUITE_ID="wp-e1c"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-mirror"
CRATE_NAME="hugit-mirror"
BOOTSTRAP_DIR="$CRATE_ROOT/src/bootstrap"
DR_DIR="$CRATE_ROOT/src/dr"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e1c.rs"

export HUGIT_GH_TEST_REPO="humangr-labs/hugit-fleet-syn-1"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-mirror crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-mirror Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-mirror is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-mirror"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "⑧ bootstrap/ subtree present (Claims: src/bootstrap/)" \
  test -d "$BOOTSTRAP_DIR"

check "⑨⑪ dr/ subtree present (Claims: src/dr/)" \
  test -d "$DR_DIR"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_e1c.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── barrel export: modules exported from lib.rs ───────────────────────────────
check "⑧ lib.rs barrel exports bootstrap module" \
  bash -c "grep -q 'bootstrap' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "⑨⑪ lib.rs barrel exports dr module" \
  bash -c "grep -q 'dr' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

# ── HUGIT_GH_TEST_REPO must be non-empty (suite always sets it) ───────────────
check "⑧ HUGIT_GH_TEST_REPO env set to non-empty value" \
  bash -c "test -n \"\$HUGIT_GH_TEST_REPO\""

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_e1c -- --list 2>/dev/null || true)"

# ── ⑧ cold-seed bootstrap: full history, hash-verified, resumable ─────────────
check "⑧ item_8_cold_seed_full_history_hash_verified declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_8_cold_seed_full_history_hash_verified'"

check "⑧ item_8_cold_seed_resumable_mid_seed declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_8_cold_seed_resumable_mid_seed'"

# ── ⑨ GitHub-side loss DR: App revocation + repo deletion/rename ──────────────
check "⑨ item_9_github_app_revocation_detected_incident declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_9_github_app_revocation_detected_incident'"

check "⑨ item_9_mirror_repo_deletion_rename_detected_incident declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_9_mirror_repo_deletion_rename_detected_incident'"

check "⑨ item_9_recovery_source_pinned_completeness_stated declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_9_recovery_source_pinned_completeness_stated'"

check "⑨ item_9_resume_from_recovered_state declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_9_resume_from_recovered_state'"

# ── ⑪ substrate-loss DR: byte-complete working git repo + end-to-end recovery ─
check "⑪ item_11_substrate_loss_mirror_is_working_git_repo declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_11_substrate_loss_mirror_is_working_git_repo'"

check "⑪ item_11_substrate_loss_full_recovery_resume_end_to_end declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_11_substrate_loss_full_recovery_resume_end_to_end'"

check "⑪ item_11_recovered_content_imports_as_change_events_not_fabricated declared in acceptance_e1c" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_11_recovered_content_imports_as_change_events_not_fabricated'"

# ── structural: resumable seed position persisted in bootstrap/ ───────────────
check "⑧ bootstrap/ references seed-progress / resume position (last verified ref/pack offset)" \
  bash -c "grep -rEq 'seed_progress|SeedProgress|resume|Resume|pack_offset|PackOffset|last_verified' '$BOOTSTRAP_DIR/' 2>/dev/null"

# ── structural: byte-identity hash verification in bootstrap/ ─────────────────
check "⑧ bootstrap/ references byte-identity hash verification" \
  bash -c "grep -rEq 'byte_identity|ByteIdentity|hash_verify|HashVerify|content_hash|object_hash' '$BOOTSTRAP_DIR/' 2>/dev/null"

# ── structural: DR controller states recovery-source explicitly ───────────────
check "⑨ dr/ references pinned recovery-source (substrate is authoritative)" \
  bash -c "grep -rEq 'recovery_source|RecoverySource|pinned_source|PinnedSource|authoritative' '$DR_DIR/' 2>/dev/null"

# ── structural: completeness criterion stated in dr/ ─────────────────────────
check "⑨⑪ dr/ references completeness criterion (byte-identity of recovered mirror)" \
  bash -c "grep -rEq 'completeness|Completeness|complete_criterion|byte_identity|ByteIdentity' '$DR_DIR/' 2>/dev/null"

# ── structural: App-revocation detection (auth failure class) in dr/ ─────────
check "⑨ dr/ references App revocation / auth failure detection" \
  bash -c "grep -rEq 'revocation|Revocation|app_revoc|auth_failure|AuthFailure|installation.*fail' '$DR_DIR/' 2>/dev/null"

# ── structural: repo-deletion/rename detection (404/redirect) in dr/ ─────────
check "⑨ dr/ references repo deletion/rename detection (404/redirect class)" \
  bash -c "grep -rEq 'deletion|Deletion|repo_delete|repo_rename|RepoRename|redirect|not_found|404' '$DR_DIR/' 2>/dev/null"

# ── structural: fail-CLOSED — unverifiable recovery → incident, never silent ──
check "⑨⑪ dr/ is fail-CLOSED (unverifiable recovery → incident, never silent)" \
  bash -c "grep -rEq 'fail_closed|FailClosed|incident|Incident|unverifiable|Unverifiable' '$DR_DIR/' 2>/dev/null"

# ── structural: recovered content goes through change-event boundary (not fabricated intents) ──
check "⑪ dr/ references import boundary / change-events (no fabricated intents)" \
  bash -c "grep -rEq 'change_event|ChangeEvent|import_boundary|ImportBoundary|change_events|no_fabricat' '$DR_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "⑧⑨⑪ cargo test -p hugit-mirror --test acceptance_e1c green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e1c

finish
