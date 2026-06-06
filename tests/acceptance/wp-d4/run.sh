#!/usr/bin/env bash
# WP-D4 acceptance suite — intents native + projection.
# Contract: docs/plan/wp-contracts/WP-D4.md
# Owned items: ① commits embed intent_id, reproducible from log
#              ② two altitudes consistent (50-intent fixture)
#              ③ sidecar corpus importable
#              ④(+) mixed fixture (intents + raw pushes interleaved on one ref):
#                    altitudes stay provably consistent, externals as external-change
# Claims: crates/hugit-refstore/src/intent/{model,projection,import}/
#         crates/hugit-refstore/tests/intents_projection/
# Oracle: crates/hugit-refstore/tests/acceptance_d4.rs
#   one #[test] item_<n>_<slug> per owned item.
# RED on current tree (crate paths not yet built).

SUITE_ID="wp-d4"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE_ROOT="crates/hugit-refstore"
CRATE_NAME="hugit-refstore"
ACCEPTANCE_FILE="$CRATE_ROOT/tests/acceptance_d4.rs"

# ── crate presence ───────────────────────────────────────────────────────────
check "hugit-refstore crate directory present" \
  test -d "$CRATE_ROOT"

check "hugit-refstore Cargo.toml present" \
  test -f "$CRATE_ROOT/Cargo.toml"

check "hugit-refstore is a workspace member (cargo metadata)" bash -c \
  'cargo metadata --no-deps --format-version 1 2>/dev/null | \
   grep -qF "hugit-refstore"'

# ── acceptance oracle file ───────────────────────────────────────────────────
check "acceptance_d4.rs committed" \
  test -f "$ACCEPTANCE_FILE"

# ── claimed src paths ────────────────────────────────────────────────────────
check "intent/model/ subtree present" \
  test -d "$CRATE_ROOT/src/intent/model"

check "intent/projection/ subtree present" \
  test -d "$CRATE_ROOT/src/intent/projection"

check "intent/import/ subtree present" \
  test -d "$CRATE_ROOT/src/intent/import"

check "intents_projection/ test fixture dir present" \
  test -d "$CRATE_ROOT/tests/intents_projection"

# ── structural negatives: D1a paths must be untouched ────────────────────────
# [no hugit-proto/ writes (D2/D3 — must be untouched)] — claim-disjointness (no writes outside claimed paths) is
# enforced authoritatively by the orchestrator at integration via
# `git diff --name-only main..HEAD` (scope-clean gate); an in-suite
# mtime check is unsound (rebase touches timestamps; siblings may be unbuilt).

# ── ① commits embed intent_id, reproducible from log ────────────────────────
check "① item_1_commits_embed_intent_id declared in acceptance_d4" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d4 -- --list 2>/dev/null | \
   grep -q "item_1_commits_embed_intent_id"'

# ── ② two altitudes consistent (50-intent fixture) ──────────────────────────
check "② item_2_two_altitudes_consistent declared in acceptance_d4" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d4 -- --list 2>/dev/null | \
   grep -q "item_2_two_altitudes_consistent"'

# ── ③ sidecar corpus importable ─────────────────────────────────────────────
check "③ item_3_sidecar_corpus_importable declared in acceptance_d4" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d4 -- --list 2>/dev/null | \
   grep -q "item_3_sidecar_corpus_importable"'

# ── ④ mixed fixture: altitudes consistent + externals as external-change ─────
check "④ item_4_mixed_fixture_externals_stay_external declared in acceptance_d4" bash -c \
  'cargo test -p '"$CRATE_NAME"' --test acceptance_d4 -- --list 2>/dev/null | \
   grep -q "item_4_mixed_fixture_externals_stay_external"'

# ── full acceptance suite green ──────────────────────────────────────────────
check "cargo test -p hugit-refstore --test acceptance_d4 green" \
  cargo test -p "$CRATE_NAME" --test acceptance_d4

finish
