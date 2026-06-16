#!/bin/bash
# Build the REAL, chain-verified engine snapshot (`engine-snapshots/hugit.json`)
# from hugit's actual recent forge history — the owner-decided launch dataset for
# engine.githugr.com (real recent PRs; NEVER fabricated). Drives the REAL hugit
# recording verbs (campaign open / intent new / pr open / pr land / check --store)
# so the log is chain-verified by construction — not hand-assembled JSON.
#
# Launch repo is `hugit` (the forge that built itself). corelink-server is
# ultra-sensitive/private and is NEVER a launch dataset.
#
# Usage:  scripts/build-engine-snapshot.sh
# Output: engine-snapshots/hugit.json (+ a throwaway intent store)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Build the CLI with the repo's pinned toolchain (avoids the rustup-proxy quirk).
RUSTC="${RUSTC:-$HOME/.rustup/toolchains/1.96.0-x86_64-apple-darwin/bin/rustc}"
export RUSTC PATH="$(dirname "$RUSTC"):$PATH" CARGO_INCREMENTAL=0
cargo build -q -p hugit-cli --bin hugit
BIN="$ROOT/target/debug/hugit"

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
OUT_DIR="$ROOT/engine-snapshots"; mkdir -p "$OUT_DIR"
OUT="$OUT_DIR/hugit.json"
STORE="$WORK/intents.json"
echo "[]" > "$OUT"

ms() { echo "$(($(date -j -f "%Y-%m-%dT%H:%M:%SZ" "$1" +%s) * 1000))"; }

# Real campaigns from hugit's own history.
"$BIN" campaign open --log "$OUT" --campaign githugr-spine \
  --charter "The git-compatible, LLM-native forge surface — hugit-serve /v1 engine + frozen wire contract powering githugr." \
  --owner humangr >/dev/null
"$BIN" campaign open --log "$OUT" --campaign pendencias-sweep \
  --charter "Post-Round-13 in-control tracked-seam closure (PS-9 / PS-11 / PS-17 + the IntentMetrics conformance twin)." \
  --owner humangr >/dev/null

# Repo authz metadata (the owner_tenant producer seam). hugit is PRIVATE
# (owner-decided 2026-06-16: the product is free, but the forge view is NOT a
# public showcase). `owner_tenant` comes from the env — unset ⇒ empty ⇒
# unassigned (operator-only) so the snapshot NEVER fabricates a tenant id. Set
# HUGIT_OWNER_TENANT=<owner's Clerk org> to let the owner's per-session token
# read+write its own repo. Reads (visibility) and writes (ownership) are decided
# off THIS record by hugit-serve::authz::project_repo_meta.
"$BIN" repo meta set --log "$OUT" --visibility private \
  --owner-tenant "${HUGIT_OWNER_TENANT:-}" --by humangr >/dev/null

# Real PRs:  number | campaign | merged-iso | merge-sha | state | title
PRS=(
  "112|githugr-spine|2026-06-14T03:07:02Z|a6735c03e15bb9aa22bdeebffd57371fb95e9c8c|landed|feat(contracts): Phase 2 wire freeze — 26 read VMs + Accepted write shape"
  "111|githugr-spine|2026-06-14T00:45:51Z|072e21f3c9f649302afafcdfc0888fc10430e7ac|landed|feat: hugit-serve — the /v1 HTTP engine port (Wave 1, githugr read-path)"
  "110|pendencias-sweep|2026-06-13T16:44:36Z|ffacb093942d035aaeb0c5c62cfdc165f8bebd30|landed|feat(checks): PS-11 — hugit check --env-axis declares a custom env dependency"
  "109|pendencias-sweep|2026-06-13T16:21:31Z|f908aaee1d15568a7a3afa85a5f70e71772c797c|landed|feat(intent): PS-9 — truthful per-source-log list/show + --log scope filter"
  "108|pendencias-sweep|2026-06-13T15:15:04Z|2bda149fbcf2c3c06d8dc3e0d23a64aef9eb6aec|landed|fix(checks): PS-17 — bound peak memory on the memo-key snapshot read"
  "107|pendencias-sweep|2026-06-13T15:48:50Z|d051e544df5d0d834beeeb4f3440a0ca48d0d38f|landed|feat(conformance): land IntentMetrics vector (§13.4) — hugit twin"
  "113|githugr-spine|2026-06-14T03:21:00Z|-|open|feat(serve): Phase 2 reads — 6 real-backbone handlers + parity/secret-matrix tests"
  "114|githugr-spine|2026-06-14T03:37:00Z|-|open|feat(serve): R2 read source — SigV4-signed, AWS-vector-proven + Passo-4 snapshot uploader"
)

for row in "${PRS[@]}"; do
  IFS='|' read -r num camp iso sha state title <<< "$row"
  at=$(ms "$iso")
  iid=$("$BIN" intent new --store "$STORE" --log "$OUT" \
        --charter "$title" --campaign "$camp" \
        --acceptance "gates green (fmt/clippy/test/deny)" \
        --id "pr-$num" --agent main \
        | python3 -c "import json,sys;print(json.load(sys.stdin)['intent_id'])")
  "$BIN" pr open --log "$OUT" --pr "$num" --campaign "$camp" \
    --author-kind orchestrator --run-id "wave-$camp" --intent "$iid" \
    --recorded-at "$at" >/dev/null
  "$BIN" pr land --log "$OUT" --pr "$num" --recorded-at "$at" >/dev/null
  [ "$state" = "landed" ] && "$BIN" pr land --log "$OUT" --pr "$num" --settle \
    --recorded-at "$((at + 1000))" >/dev/null
  echo "  PR #$num ($state) intent=$iid"
done

# Real fmt checks: the first executes (MISS), repeats over the same tree are
# memoized HITs — the memoization wedge, demonstrated with REAL data.
for pr in 111 112 113; do
  "$BIN" check --def fmt --log "$OUT" --store --pr "$pr" --principal "orchestrator:hugit" >/dev/null
done

echo "snapshot: $OUT  ($(python3 -c "import json;print(len(json.load(open('$OUT'))))") records)"
