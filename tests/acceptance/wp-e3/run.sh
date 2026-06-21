#!/usr/bin/env bash
# WP-E3 acceptance suite — status / badge compatibility emitter (items ① ② ③).
# Acceptance harness for WP-E3.
# Owned items:
#   ① checks appear as GitHub statuses
#   ② badge reflects true state within a stated staleness bound;
#      status-API down → last-known + observable staleness, never silent wrong
#   ③(+) status-API 429/5xx: retry w/ backoff → eventually true state;
#         no stuck-pending; failures observable
# Oracle: crates/hugit-mirror/tests/acceptance_e3.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-mirror/src/status/ — disjoint from other mirror modules.
# GitHub-status items require HUGIT_GH_TEST_REPO (suite exports humangr-labs/hugit-fleet-syn-1);
#   checks FAIL — not skip — when HUGIT_GH_TEST_REPO is set but the env is empty.
# RED on current tree (status/ subtree not yet built).

SUITE_ID="wp-e3"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-mirror"
CRATE_NAME="hugit-mirror"
STATUS_DIR="$CRATE_ROOT/src/status"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e3.rs"

export HUGIT_GH_TEST_REPO="humangr-labs/hugit-fleet-syn-1"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-mirror crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-mirror Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-mirror is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-mirror"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "① status/ subtree present (Claims: src/status/)" \
  test -d "$STATUS_DIR"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_e3.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── barrel export: status module exported from lib.rs ────────────────────────
check "① lib.rs barrel exports status module" \
  bash -c "grep -q 'status' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

# ── HUGIT_GH_TEST_REPO must be non-empty (suite always sets it) ───────────────
check "① HUGIT_GH_TEST_REPO env set to non-empty value" \
  bash -c "test -n \"\$HUGIT_GH_TEST_REPO\""

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_e3 -- --list 2>/dev/null || true)"

# ── ① checks appear as GitHub statuses ───────────────────────────────────────
check "① item_1_checks_appear_as_github_statuses declared in acceptance_e3" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_checks_appear_as_github_statuses'"

# ── ② badge staleness bound ───────────────────────────────────────────────────
check "② item_2_badge_staleness_bound declared in acceptance_e3" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_badge_staleness_bound'"

# ── ② badge API-down: last-known + observable staleness, never silent wrong ───
check "② item_2_badge_api_down_last_known declared in acceptance_e3" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2_badge_api_down_last_known'"

# ── ③ 429/5xx retry w/ backoff → eventually true state ───────────────────────
check "③ item_3_api_429_retry_backoff declared in acceptance_e3" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_api_429_retry_backoff'"

# ── ③ no stuck-pending; failures observable ───────────────────────────────────
check "③ item_3_no_stuck_pending_observable declared in acceptance_e3" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_no_stuck_pending_observable'"

# ── structural: status emitter references CheckResult ────────────────────────
check "① CheckResult referenced in status/ src (contract dep)" \
  bash -c "grep -rEq 'CheckResult' '$STATUS_DIR/' 2>/dev/null"

# ── structural: staleness bound is a published constant ──────────────────────
check "② staleness bound constant present in status/ src" \
  bash -c "grep -rEq 'staleness|STALENESS|stale_bound|STALE_BOUND' '$STATUS_DIR/' 2>/dev/null"

# ── structural: no silent-wrong / last-known logic referenced ────────────────
check "② last-known/observable-staleness logic referenced in status/ src" \
  bash -c "grep -rEq 'last_known|LastKnown|observable|stale' '$STATUS_DIR/' 2>/dev/null"

# ── structural: backoff retry referenced ─────────────────────────────────────
check "③ backoff/retry referenced in status/ src" \
  bash -c "grep -rEq 'backoff|retry|Retry|back_off' '$STATUS_DIR/' 2>/dev/null"

# ── structural: no stuck-pending path (no unconditional pending return) ───────
check "③ no stuck-pending sentinel (no bare 'Pending' return without recovery) in status/ src" \
  bash -c "! grep -rEq 'stuck_pending|return.*Pending.*forever' '$STATUS_DIR/' 2>/dev/null"

# ── structural: Claims disjointness — status/ does not bleed into other modules ──
check "① status/ does not bleed into mirror/ graph module" \
  bash -c "! grep -rEq 'mod graph|use.*hugit_mirror::graph' '$STATUS_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "①②③ cargo test -p hugit-mirror --test acceptance_e3 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e3

finish
