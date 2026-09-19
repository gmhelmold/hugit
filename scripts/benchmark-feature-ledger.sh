#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${HUGIT_BIN:-$ROOT/target/release/hugit}"
if [[ ! -x "$BIN" ]]; then
  if [[ -n "${HUGIT_BIN:-}" ]]; then
    printf 'HUGIT_BIN is not executable: %s\n' "$BIN" >&2
    exit 2
  fi
  cargo build --manifest-path "$ROOT/Cargo.toml" -p hugit-cli --release --locked
fi

if [[ -n "${HUGIT_EVIDENCE_DIR:-}" ]]; then
  REPORT="$HUGIT_EVIDENCE_DIR"
else
  # Retain a unique parent; the producer exclusively creates its fresh leaf.
  # Allocate only after binary validation/build, so those errors leave no output.
  REPORT_PARENT="$(mktemp -d "${TMPDIR:-/tmp}/hugit-evidence-cli-local.XXXXXX")"
  REPORT="$REPORT_PARENT/report"
  printf 'Evidence destination: %s\n' "$REPORT" >&2
fi

exec python3 "$ROOT/scripts/evidence_report.py" \
  --hugit-bin "$BIN" \
  --output "$REPORT" \
  "$@"
