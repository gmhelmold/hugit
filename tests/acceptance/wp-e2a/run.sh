#!/usr/bin/env bash
# WP-E2a acceptance suite — git history import (byte-identity, LFS, resumable, idempotency).
# Acceptance harness for WP-E2a.
# Owned items:
#   ① 1k-commit public import byte-identical
#   ③ idempotent re-import
#   ④(+) private repo via installation auth; LFS objects materialized (not pointers);
#        >1-timeout repo resumes and completes byte-identical
#   ⑤(R2) import boundary: commit history → opaque change-events, NO intent from bare commit
#   ⑦ 🔧 idempotency defined: unchanged → no-op; changed → incremental re-sync (no dupes)
# Oracle: crates/hugit-mirror/tests/acceptance_e2a.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-mirror/src/import/history/, import/lfs/, import/resume/, import/auth.rs
#
# Live-repo items (①③): may clone a public repo via git shell-out OR use a committed fixture
# bundle. Items needing HUGIT_GH_TEST_REPO FAIL — not skip — when the variable is set but empty.
# Private-repo items (④): require HUGIT_GH_INSTALL_TOKEN; FAIL-not-skip when needed-but-absent.
# RED on current tree (import/ subtree not yet built).

SUITE_ID="wp-e2a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-mirror"
CRATE_NAME="hugit-mirror"
IMPORT_DIR="$CRATE_ROOT/src/import"
HISTORY_DIR="$IMPORT_DIR/history"
LFS_DIR="$IMPORT_DIR/lfs"
RESUME_DIR="$IMPORT_DIR/resume"
AUTH_FILE="$IMPORT_DIR/auth.rs"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e2a.rs"

export HUGIT_GH_TEST_REPO="HumanGuardrail/hugit-fleet-syn-1"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-mirror crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-mirror Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-mirror is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-mirror"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "① import/history/ subtree present (Claims: src/import/history/)" \
  test -d "$HISTORY_DIR"

check "④ import/lfs/ subtree present (Claims: src/import/lfs/)" \
  test -d "$LFS_DIR"

check "⑦ import/resume/ subtree present (Claims: src/import/resume/)" \
  test -d "$RESUME_DIR"

check "④ import/auth.rs present (Claims: src/import/auth.rs)" \
  test -f "$AUTH_FILE"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_e2a.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── barrel export: import modules exported from lib.rs ───────────────────────
check "① lib.rs barrel exports import::history" \
  bash -c "grep -q 'history' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "④ lib.rs barrel exports import::lfs" \
  bash -c "grep -q 'lfs' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "⑦ lib.rs barrel exports import::resume" \
  bash -c "grep -q 'resume' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "④ lib.rs barrel exports import::auth" \
  bash -c "grep -q 'auth' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

# ── HUGIT_GH_TEST_REPO must be non-empty (suite always sets it) ───────────────
check "① HUGIT_GH_TEST_REPO env set to non-empty value" \
  bash -c "test -n \"\$HUGIT_GH_TEST_REPO\""

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_e2a -- --list 2>/dev/null || true)"

# ── ① 1k-commit public import byte-identical ─────────────────────────────────
check "① item_1_public_import_byte_identical declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_public_import_byte_identical'"

# ── ③ idempotent re-import ────────────────────────────────────────────────────
check "③ item_3_idempotent_reimport declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_idempotent_reimport'"

# ── ④ private repo via installation auth ─────────────────────────────────────
check "④ item_4_private_repo_install_auth declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_private_repo_install_auth'"

# ── ④ LFS objects materialized (not pointers) ────────────────────────────────
check "④ item_4_lfs_objects_materialized declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_lfs_objects_materialized'"

# ── ④ resume across timeout, completes byte-identical ────────────────────────
check "④ item_4_resume_byte_identical declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4_resume_byte_identical'"

# ── ⑤ import boundary: no intent from bare commit ────────────────────────────
check "⑤ item_5_no_intent_from_bare_commit declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5_no_intent_from_bare_commit'"

# ── ⑦ idempotency: unchanged source → no-op ──────────────────────────────────
check "⑦ item_7_unchanged_source_noop declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_7_unchanged_source_noop'"

# ── ⑦ idempotency: changed source → incremental re-sync, no dupes ────────────
check "⑦ item_7_changed_source_incremental_no_dupes declared in acceptance_e2a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_7_changed_source_incremental_no_dupes'"

# ── structural: EventRecord referenced in history/ src (import boundary law ⑤) ─
check "⑤ EventRecord referenced in history/ src (contract dep)" \
  bash -c "grep -rEq 'EventRecord' '$HISTORY_DIR/' 2>/dev/null"

# ── structural: no intent synthesis from bare commit ─────────────────────────
check "⑤ no intent synthesis in history/ src (import boundary)" \
  bash -c "! grep -rEq 'IntentSidecar|mint_intent|synthesize_intent' '$HISTORY_DIR/' 2>/dev/null"

# ── structural: LFS pointer detection referenced in lfs/ src ─────────────────
check "④ LFS pointer detection referenced in lfs/ src" \
  bash -c "grep -rEq 'pointer|lfs_pointer|LfsPointer' '$LFS_DIR/' 2>/dev/null"

# ── structural: byte-identity verification (object-hash compare) in history/ ──
check "① byte-identity hash comparison referenced in history/ src" \
  bash -c "grep -rEq 'object_hash|hash.*compare|byte.ident|oid' '$HISTORY_DIR/' 2>/dev/null"

# ── structural: resume cursor persisted in resume/ src ───────────────────────
check "④⑦ resume cursor persisted in resume/ src" \
  bash -c "grep -rEq 'cursor|Cursor|checkpoint|Checkpoint' '$RESUME_DIR/' 2>/dev/null"

# ── structural: idempotency no-dupes guard in resume/ src ────────────────────
check "⑦ idempotency no-dupes guard referenced in resume/ src" \
  bash -c "grep -rEq 'no.op|noop|no_op|idempoten|dedup|no.dup' '$RESUME_DIR/' 2>/dev/null"

# ── structural: installation auth (App token, not PAT) in auth.rs ────────────
check "④ installation auth (App installation token) referenced in auth.rs" \
  bash -c "grep -Eq 'installation|InstallationToken|install_token' '$AUTH_FILE' 2>/dev/null"

# ── structural: Claims disjointness — history/ does not bleed into prissue/ ──
check "⑤ history/ does not reference prissue module (disjoint Claims)" \
  bash -c "! grep -rEq 'prissue|pr_issue|PrIssue' '$HISTORY_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "①③④⑤⑦ cargo test -p hugit-mirror --test acceptance_e2a green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e2a

finish
