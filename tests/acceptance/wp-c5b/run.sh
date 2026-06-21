#!/usr/bin/env bash
# WP-C5b acceptance suite — secrets broker + escape red-team harness.
# Acceptance harness for WP-C5b.
# Owned items:
#   ② zero secret material in job (red-team env/proc/disk)
#   ③ broker calls audited w/ principal chain
#   ④ broker down → fail CLOSED
#   ⑤(+) active escape red-team: traversal/symlink/out-of-fence writes/
#         fork-bomb/disk-fill contained; cannot reach another lease or
#         starve the box
#   ⑥(R2) positive path: job completes credential-needing operation VIA
#          the broker successfully; raw credential provably absent during
#          and after
# Claims: crates/hugit-fence/broker/
# Oracle: crates/hugit-fence/tests/acceptance_c5b.rs
#   one #[test] item_<n>_<slug> per owned item.
# Box env: HUGIT_RUNNER_HOST=203.0.113.10 (exported before cargo invocations;
#   box-dependent tests FAIL — not skip — when env set but box unreachable).
#   Tests may skip only when HUGIT_RUNNER_HOST is entirely unset.
# Red-team containers namespaced hugit-c5b-*.
# RED on current tree (broker/ subtree not yet built).

SUITE_ID="wp-c5b"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

export HUGIT_RUNNER_HOST=203.0.113.10

CRATE_ROOT="crates/hugit-fence"
CRATE_NAME="hugit-fence"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_c5b.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-fence crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-fence Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-fence is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-fence"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_c5b.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "broker/ subtree present" \
  test -d "$CRATE_ROOT/src/broker"

# ── structural negatives: C5a paths must be untouched by C5b ─────────────────
# [materialize/ subtree belongs to C5a — no new files from C5b] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# [enforce/ subtree belongs to C5a — no new files from C5b] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# ── item declaration check via --list ────────────────────────────────────────
export TESTLIST="$(cargo test -p "$CRATE_NAME" --test acceptance_c5b -- --list 2>/dev/null || true)"

check "② item_2_zero_secret_in_job_env_proc_disk declared in acceptance_c5b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_2'"

check "③ item_3_broker_calls_audited_principal_chain declared in acceptance_c5b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_3'"

check "④ item_4_broker_down_fail_closed declared in acceptance_c5b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_4'"

check "⑤ item_5_escape_redteam_all_attacks_contained declared in acceptance_c5b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_5'"

check "⑥ item_6_positive_path_via_broker_credential_absent declared in acceptance_c5b" \
  bash -c "printf '%s\n' \"\$TESTLIST\" | grep -q 'item_6'"

# ── structural/contract asserts on broker/ src ───────────────────────────────

# ② raw credential must never appear in runner env
check "② no raw-credential-on-runner path in broker/ src" bash -c \
  "! grep -rEq 'raw_secret|plaintext_cred|credential_env' '$CRATE_ROOT/src/broker/' 2>/dev/null"

# ③ principal chain audit: RunnerLease reference present
check "③ principal chain / RunnerLease referenced in broker/ src" bash -c \
  "grep -rEq 'RunnerLease|principal_chain|audit_record|AuditRecord' '$CRATE_ROOT/src/broker/'"

# ④ fail-closed: no fallback to credential-on-runner when broker down
check "④ fail-closed — no credential fallback on broker failure in broker/ src" bash -c \
  "! grep -rEq 'fallback_credential|credential_fallback|bypass_broker' '$CRATE_ROOT/src/broker/' 2>/dev/null"

check "④ fail-closed error variant declared in broker/ src" bash -c \
  "grep -rEq 'BrokerDown|broker_down|FailClosed|fail_closed' '$CRATE_ROOT/src/broker/'"

# ⑤ escape red-team: all five attack vectors must be covered
check "⑤ traversal attack fixture referenced in broker/ src or tests" bash -c \
  "grep -rEq 'traversal|path_traversal|DotDot|dotdot' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

check "⑤ symlink escape fixture referenced in broker/ src or tests" bash -c \
  "grep -rEq 'symlink|symlink_escape|SymlinkEscape' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

check "⑤ out-of-fence write fixture referenced in broker/ src or tests" bash -c \
  "grep -rEq 'out_of_fence|OutOfFence|fence_write' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

check "⑤ fork-bomb containment referenced in broker/ src or tests" bash -c \
  "grep -rEq 'fork.?bomb|ForkBomb|fork_bomb|pids|PID_LIMIT' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

check "⑤ disk-fill containment referenced in broker/ src or tests" bash -c \
  "grep -rEq 'disk.?fill|DiskFill|disk_fill|disk_limit|DISK_LIMIT' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

check "⑤ hugit-c5b-* container namespace used in red-team fixtures" bash -c \
  "grep -rEq 'hugit-c5b' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

# ⑤ held commands: attack jobs must overlap in time (sleep-held, not instant-exit)
check "⑤ held commands (sleep) used in escape red-team — attacks overlap in time" bash -c \
  "grep -rEq 'sleep|held|hold_open' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

# ⑥ positive path: raw credential absent scan present
check "⑥ raw-credential-absent scan referenced in broker/ src or tests" bash -c \
  "grep -rEq 'credential_absent|absent_scan|scan_clean|credential_scan' \
   '$CRATE_ROOT/src/broker/' '$CRATE_ROOT/tests/' 2>/dev/null"

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-fence --test acceptance_c5b green" \
  cargo test -p "$CRATE_NAME" --test acceptance_c5b

finish
