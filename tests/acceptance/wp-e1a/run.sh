#!/usr/bin/env bash
# WP-E1a acceptance suite — verified mirror: outbound sync + hash verify + ordering/queue (items ① ③ ⑩).
# Acceptance harness for WP-E1a.
# Owned items:
#   ①  landing on GitHub <60s hash-verified
#   ③  72h soak 100% verified
#   ⑩🔧 outage queue has a stated capacity bound; overflow → backpressure + incident, never drop
# Oracle: crates/hugit-mirror/tests/acceptance_e1a.rs — one #[test] item_<n>_<slug>
#   per owned acceptance item.
# Claims: crates/hugit-mirror/src/outbound/, src/verify/, src/queue/ — disjoint from other modules.
# Live-GitHub items (①) require HUGIT_GH_TEST_REPO (suite exports humangr-labs/hugit-fleet-syn-1);
#   checks FAIL — not skip — when HUGIT_GH_TEST_REPO is set but the env lacks the installation.
# Hash-verify/queue-capacity items are local fixture proofs over mirror logic.
# RED on current tree (outbound/ verify/ queue/ subtrees not yet built).

SUITE_ID="wp-e1a"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-mirror"
CRATE_NAME="hugit-mirror"
OUTBOUND_DIR="$CRATE_ROOT/src/outbound"
VERIFY_DIR="$CRATE_ROOT/src/verify"
QUEUE_DIR="$CRATE_ROOT/src/queue"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_e1a.rs"

export HUGIT_GH_TEST_REPO="humangr-labs/hugit-fleet-syn-1"

# ── crate presence ────────────────────────────────────────────────────────────
check "hugit-mirror crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-mirror Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-mirror is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | grep -qF "hugit-mirror"'

# ── claimed src paths ─────────────────────────────────────────────────────────
check "① outbound/ subtree present (Claims: src/outbound/)" \
  test -d "$OUTBOUND_DIR"

check "① verify/ subtree present (Claims: src/verify/)" \
  test -d "$VERIFY_DIR"

check "⑩ queue/ subtree present (Claims: src/queue/)" \
  test -d "$QUEUE_DIR"

# ── oracle file ───────────────────────────────────────────────────────────────
check "acceptance_e1a.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── barrel export: modules exported from lib.rs ───────────────────────────────
check "① lib.rs barrel exports outbound module" \
  bash -c "grep -q 'outbound' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "① lib.rs barrel exports verify module" \
  bash -c "grep -q 'verify' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

check "⑩ lib.rs barrel exports queue module" \
  bash -c "grep -q 'queue' '$CRATE_ROOT/src/lib.rs' 2>/dev/null"

# ── HUGIT_GH_TEST_REPO must be non-empty (suite always sets it) ───────────────
check "① HUGIT_GH_TEST_REPO env set to non-empty value" \
  bash -c "test -n \"\$HUGIT_GH_TEST_REPO\""

# ── TESTLIST: one cargo --list invocation, reused by grep checks ──────────────
export TESTLIST
TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_e1a -- --list 2>/dev/null || true)"

# ── ① landing on GitHub <60s hash-verified ────────────────────────────────────
check "① item_1_landing_hash_verified_within_sla declared in acceptance_e1a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_1_landing_hash_verified_within_sla'"

# ── ③ 72h soak 100% verified ──────────────────────────────────────────────────
check "③ item_3_soak_72h_all_verified declared in acceptance_e1a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3_soak_72h_all_verified'"

# ── ⑩ queue capacity bound stated; overflow → backpressure, never drop ────────
check "⑩ item_10_queue_capacity_bound_stated declared in acceptance_e1a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_10_queue_capacity_bound_stated'"

check "⑩ item_10_queue_overflow_backpressure_not_drop declared in acceptance_e1a" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_10_queue_overflow_backpressure_not_drop'"

# ── structural: App auth (GitHub App, not PAT) used in outbound/ ─────────────
check "① outbound/ references App installation token (not PAT)" \
  bash -c "grep -rEq 'installation_token|InstallationToken|app_token|AppToken|AppAuth|app_auth' '$OUTBOUND_DIR/' 2>/dev/null"

# ── structural: per-push hash verification in verify/ ─────────────────────────
check "① verify/ contains per-push content-hash check logic" \
  bash -c "grep -rEq 'content_hash|ContentHash|hash_verify|HashVerify|byte_identity|object_hash' '$VERIFY_DIR/' 2>/dev/null"

# ── structural: fail-CLOSED on mismatch in verify/ ───────────────────────────
check "① verify/ references divergence signal on mismatch (fail-CLOSED)" \
  bash -c "grep -rEq 'divergence|Divergence|mismatch|Mismatch|fail_closed|FailClosed' '$VERIFY_DIR/' 2>/dev/null"

# ── structural: queue capacity bound is a stated constant ────────────────────
check "⑩ queue/ contains stated capacity-bound constant" \
  bash -c "grep -rEq 'CAPACITY|capacity_bound|CapacityBound|MAX_QUEUE|max_capacity' '$QUEUE_DIR/' 2>/dev/null"

# ── structural: queue backpressure logic referenced ───────────────────────────
check "⑩ queue/ references backpressure on overflow (never drop)" \
  bash -c "grep -rEq 'backpressure|BackPressure|back_pressure|overflow|Overflow' '$QUEUE_DIR/' 2>/dev/null"

# ── structural: queue ordering preserved (no reorder) ────────────────────────
check "⑩ queue/ references ordered/FIFO semantics" \
  bash -c "grep -rEq 'ordered|FIFO|fifo|ordering|Ordering' '$QUEUE_DIR/' 2>/dev/null"

# ── structural: one-way only (outbound/ never reads GitHub state as truth) ────
check "① outbound/ does not import a reverse-sync path" \
  bash -c "! grep -rEq 'reverse_sync|ReverseSync|sync_from_github|from_mirror' '$OUTBOUND_DIR/' 2>/dev/null"

# ── full acceptance suite green ───────────────────────────────────────────────
check "①③⑩ cargo test -p hugit-mirror --test acceptance_e1a green" \
  cargo test -p "$CRATE_NAME" --test acceptance_e1a

finish
