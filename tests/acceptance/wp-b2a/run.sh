#!/usr/bin/env bash
# WP-B2a acceptance suite — checks client: CheckDef format + local executor + memo key.
# Acceptance harness for WP-B2a.
# Owned items (B2a owns ①②⑥⑦ of B2; B2b owns ③④⑤):
#   ① repeat tree+def → AC hit, 0 exec, <500ms
#   ② glob sensitivity: in-glob edit → rerun; out-of-glob edit → hit
#   ⑥ (R2) toolchain sensitivity: different toolchain → MISS, never false hit
#   ⑦ (R3) def sensitivity: changed def, same tree+toolchain → MISS + re-execute (3rd key axis)
# Claims: crates/hugit-checks/src/client/
# Oracle: crates/hugit-checks/tests/acceptance_b2a.rs
#   one #[test] item_<n>_<slug> per owned item.
# CoreLink tenant env:
#   HUGIT_CORELINK_PAT_PATH — path to PAT file (default: ~/.hugit/secrets/corelink-tenant/pat)
#   HUGIT_CORELINK_API      — CoreLink API base URL
# Live checks FAIL (not skip) when the PAT file is absent.
# This suite is EXPECTED RED-with-live-fails until the tenant exists (P2 arrival).

export TESTLIST="item_1_ac_hit_zero_exec_lt500ms item_2_glob_sensitivity item_6_toolchain_sensitivity_never_false_hit item_7_def_sensitivity_miss_reexec"

SUITE_ID="wp-b2a"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

# CoreLink tenant defaults — live checks FAIL (not skip) when PAT is absent.
export HUGIT_CORELINK_PAT_PATH="${HUGIT_CORELINK_PAT_PATH:-${HOME}/.hugit/secrets/corelink-tenant/pat}"
export HUGIT_CORELINK_API="${HUGIT_CORELINK_API:-https://api.corelink.humangrforgedev.com}"

CRATE_ROOT="crates/hugit-checks"
CRATE_NAME="hugit-checks"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_b2a.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-checks crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-checks Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-checks is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-checks"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_b2a.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "client/ subtree present" \
  test -d "$CRATE_ROOT/src/client"

# ── structural negatives: B2b/B3/C4 paths must be untouched ─────────────────
# [runner/ subtree belongs to B2b — no new files from B2a] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [affected/ subtree belongs to B3 — no new files from B2a] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [regen/ subtree belongs to C4 — no new files from B2a] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# ── PAT file present (live checks fail when absent) ─────────────────────────
check "CoreLink PAT file present at HUGIT_CORELINK_PAT_PATH" \
  test -f "$HUGIT_CORELINK_PAT_PATH"

# ── ① AC hit, 0 exec, <500ms ────────────────────────────────────────────────
check "① item_1_ac_hit_zero_exec_lt500ms declared in acceptance_b2a" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_b2a -- --list 2>/dev/null | \
   grep -q "item_1_ac_hit_zero_exec_lt500ms"'

# ── ② glob sensitivity ───────────────────────────────────────────────────────
check "② item_2_glob_sensitivity declared in acceptance_b2a" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_b2a -- --list 2>/dev/null | \
   grep -q "item_2_glob_sensitivity"'

# ── ⑥ toolchain sensitivity, never false hit ────────────────────────────────
check "⑥ item_6_toolchain_sensitivity_never_false_hit declared in acceptance_b2a" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_b2a -- --list 2>/dev/null | \
   grep -q "item_6_toolchain_sensitivity_never_false_hit"'

# ── ⑦ def sensitivity → MISS + re-execute ───────────────────────────────────
check "⑦ item_7_def_sensitivity_miss_reexec declared in acceptance_b2a" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_b2a -- --list 2>/dev/null | \
   grep -q "item_7_def_sensitivity_miss_reexec"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-checks --test acceptance_b2a green" \
  cargo test -p "$CRATE_NAME" --test acceptance_b2a

finish
