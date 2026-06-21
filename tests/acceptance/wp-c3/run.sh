#!/usr/bin/env bash
# WP-C3 acceptance suite — cache-warm boot.
# Acceptance harness for WP-C3.
# Owned items:
#   ① warm ≤10s vs cold ≥60s
#   ② toolchain layers shared (one physical copy per content hash, across jobs)
#   ③ (+) CAS/AC down mid-job → fail CLOSED: zero poisoned writes, no hang, no false green
# Claims: crates/hugit-runner/boot/
# Oracle: crates/hugit-runner/tests/acceptance_c3.rs
#   one #[test] item_<n>_<slug> per owned item.
# CoreLink tenant env:
#   HUGIT_CORELINK_PAT_PATH — path to PAT file (default: ~/.hugit/secrets/corelink-tenant/pat)
#   HUGIT_CORELINK_API      — CoreLink API base URL
# Live checks FAIL (not skip) when the PAT file is absent.
# This suite is EXPECTED RED-with-live-fails until the tenant exists (P2 arrival).

export TESTLIST="item_1_warm_le10s_cold_ge60s item_2_toolchain_layers_shared item_3_cas_ac_down_fail_closed"

SUITE_ID="wp-c3"
export SUITE_ID
# shellcheck source=../lib.sh
source "$(dirname "$0")/../lib.sh"

# CoreLink tenant defaults — live checks FAIL (not skip) when PAT is absent.
export HUGIT_CORELINK_PAT_PATH="${HUGIT_CORELINK_PAT_PATH:-${HOME}/.hugit/secrets/corelink-tenant/pat}"
export HUGIT_CORELINK_API="${HUGIT_CORELINK_API:-https://api.corelink.humangrforgedev.com}"

CRATE_ROOT="crates/hugit-runner"
CRATE_NAME="hugit-runner"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c3.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-runner crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-runner Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-runner is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-runner"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_c3.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "boot/ subtree present" \
  test -d "$CRATE_ROOT/src/boot"

# ── structural negatives: C2a/C2b/C5a/C5b/E4 paths must be untouched ────────
# [lease/ subtree belongs to C2a — no new files from C3] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [concurrency/ subtree belongs to C2b — no new files from C3] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# ── PAT file present (live checks fail when absent) ─────────────────────────
check "CoreLink PAT file present at HUGIT_CORELINK_PAT_PATH" \
  test -f "$HUGIT_CORELINK_PAT_PATH"

# ── ① warm ≤10s vs cold ≥60s ────────────────────────────────────────────────
check "① item_1_warm_le10s_cold_ge60s declared in acceptance_c3" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c3 -- --list 2>/dev/null | \
   grep -q "item_1_warm_le10s_cold_ge60s"'

# ── ② toolchain layers shared across jobs ────────────────────────────────────
check "② item_2_toolchain_layers_shared declared in acceptance_c3" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c3 -- --list 2>/dev/null | \
   grep -q "item_2_toolchain_layers_shared"'

# ── ③ CAS/AC down mid-job → fail CLOSED ─────────────────────────────────────
# Asserts: zero poisoned writes to CAS/AC, no indefinite hang, never false green.
check "③ item_3_cas_ac_down_fail_closed declared in acceptance_c3" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c3 -- --list 2>/dev/null | \
   grep -q "item_3_cas_ac_down_fail_closed"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-runner --test acceptance_c3 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c3

finish
