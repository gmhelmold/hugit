#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${HUGIT_BIN:-$ROOT/target/release/hugit}"
if [[ -n "${HUGIT_EVIDENCE_DIR:-}" ]]; then
  REPORT="$HUGIT_EVIDENCE_DIR"
else
  REPORT="$(mktemp -d "${TMPDIR:-/tmp}/hugit-evidence-cli-local.XXXXXX")"
fi

if [[ ! -x "$BIN" ]]; then
  if [[ -n "${HUGIT_BIN:-}" ]]; then
    printf 'HUGIT_BIN is not executable: %s\n' "$BIN" >&2
    exit 2
  fi
  cargo build --manifest-path "$ROOT/Cargo.toml" -p hugit-cli --release --locked
fi

exec python3 "$ROOT/scripts/evidence_report.py" \
  --hugit-bin "$BIN" \
  --output "$REPORT" \
  "$@"
