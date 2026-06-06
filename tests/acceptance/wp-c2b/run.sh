#!/usr/bin/env bash
# WP-C2b acceptance suite — ephemeral runner: concurrency/throughput + crash recovery + expiry.
# Contract: docs/plan/wp-contracts/WP-C2b.md
# Owned items: ③ expiry hard-kill
#              ④ ≥8 concurrent/box
#              ⑤(R2) box crash mid-job: job detected lost → requeued/surfaced,
#                    no silent drop, no false green, lease/fence cleaned up
# Claims: crates/hugit-runner/{concurrency,expiry,recovery}
# Oracle: crates/hugit-runner/tests/acceptance_c2b.rs
#   one #[test] item_<n>_<slug> per owned item.
# Box env: HUGIT_RUNNER_HOST=91.99.11.196 (exported before cargo invocations;
#   box-dependent tests FAIL — not skip — when env set but box unreachable).
#   Tests may skip only when HUGIT_RUNNER_HOST is entirely unset.
# RED on current tree (crate paths not yet built).

SUITE_ID="wp-c2b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=91.99.11.196

CRATE_ROOT="crates/hugit-runner"
CRATE_NAME="hugit-runner"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c2b.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-runner crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-runner Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-runner is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-runner"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_c2b.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "concurrency/ subtree present" \
  test -d "$CRATE_ROOT/src/concurrency"

check "expiry/ subtree present" \
  test -d "$CRATE_ROOT/src/expiry"

check "recovery/ subtree present" \
  test -d "$CRATE_ROOT/src/recovery"

# ── structural negatives: C2a paths must be untouched ────────────────────────
# [lease/ subtree belongs to C2a — no new files from C2b] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [isolation/ subtree belongs to C2a — no new files from C2b] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [teardown/ subtree belongs to C2a — no new files from C2b] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# ── ③ expiry hard-kill ───────────────────────────────────────────────────────
check "③ item_3_expiry_hard_kill declared in acceptance_c2b" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c2b -- --list 2>/dev/null | \
   grep -q "item_3_expiry_hard_kill"'

# ── ④ ≥8 concurrent/box ─────────────────────────────────────────────────────
check "④ item_4_concurrent_ge8_per_box declared in acceptance_c2b" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c2b -- --list 2>/dev/null | \
   grep -q "item_4_concurrent_ge8_per_box"'

# ── ⑤ box crash mid-job: lost → requeued/surfaced, no silent drop/false green ─
check "⑤ item_5_crash_recovery_lost_detected declared in acceptance_c2b" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_c2b -- --list 2>/dev/null | \
   grep -q "item_5_crash_recovery_lost_detected"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-runner --test acceptance_c2b green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c2b

finish
