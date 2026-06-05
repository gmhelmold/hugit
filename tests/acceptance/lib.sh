#!/usr/bin/env bash
# Shared acceptance-harness helpers. Orchestrator-owned (Day 0).
# Suites are standalone scripts — deliberately OUTSIDE the cargo gate lane,
# so a red suite never blocks main's fmt/clippy/test/audit gates.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

_PASS=0
_FAIL=0
_SUITE="${SUITE_ID:?SUITE_ID must be set before sourcing lib.sh}"

pass() { _PASS=$((_PASS + 1)); printf 'ACCEPT-PASS[%s]: %s\n' "$_SUITE" "$1"; }

fail() { _FAIL=$((_FAIL + 1)); printf 'ACCEPT-FAIL[%s]: %s\n' "$_SUITE" "$1"; }

# check <description> <command...>  — run command silently, record pass/fail
check() {
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then pass "$desc"; else fail "$desc"; fi
}

finish() {
  printf -- '----------------------------------------\n'
  printf 'SUITE %s: %d passed, %d failed → %s\n' \
    "$_SUITE" "$_PASS" "$_FAIL" "$([ "$_FAIL" -eq 0 ] && echo GREEN || echo RED)"
  [ "$_FAIL" -eq 0 ]
}
