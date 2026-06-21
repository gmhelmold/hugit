#!/usr/bin/env bash
# WP-B1 acceptance suite — hugit-app Worker skeleton (all 5 owned items).
# Acceptance harness for WP-B1. Green = forged-webhook 401+audit,
# PR event persisted ack<1s, check-run on real PR, least-privilege manifest,
# uninstall revokes+halts (audited). RED on absent crate.
#
# Naming convention (pre-decided by the lead, binding for the agent):
#   acceptance test  → item_<n>_<slug>   (e.g. item_1_forged_webhook_rejected)
#   acceptance file  → crates/hugit-app/tests/acceptance_wp_b1.rs

SUITE_ID="wp-b1"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATE=crates/hugit-app
ACCEPTANCE_FILE="$CRATE/tests/acceptance_wp_b1.rs"

# ── (a) the claimed crate exists ─────────────────────────────────────────────
check "① hugit-app crate present (Cargo.toml + src/lib.rs)" \
  bash -c "test -f $CRATE/Cargo.toml && test -f $CRATE/src/lib.rs"

# ── (b) cargo test --list declares item_<n> for every owned item ─────────────
TESTLIST="$(cargo test -p hugit-app --test acceptance_wp_b1 -- --list 2>/dev/null || true)"

check "① item_1 declared (forged webhook→401+audit)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_1'"
check "② item_2 declared (PR event persisted, ack<1s)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_2'"
check "③ item_3 declared (check-run on real PR)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_3'"
check "④ item_4 declared (least-privilege manifest snapshot)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_4'"
check "⑤ item_5 declared (uninstall revokes access + halts processing, audited)" \
  bash -c "echo '$TESTLIST' | grep -q 'item_5'"

# ── (c) cargo test -p hugit-app --test acceptance_wp_b1 is green ─────────────
check "cargo test -p hugit-app --test acceptance_wp_b1 green" \
  cargo test -p hugit-app --test acceptance_wp_b1

# ── (d) structural / negative asserts from the contract ──────────────────────

# ④ a manifest fixture file must be committed
check "④ manifest fixture committed (manifest.json or app-manifest.json)" \
  bash -c "find $CRATE -name 'manifest.json' -o -name 'app-manifest.json' | grep -q ."

# ① forged-webhook 401 path: signature verification code present (not just tests)
check "① X-Hub-Signature-256 verification present in src" \
  bash -c "grep -rq 'X-Hub-Signature-256\|x_hub_signature_256\|hmac.*sha256\|sha256.*hmac' $CRATE/src/"

# ① audit trail: EventRecord of kind webhook.rejected wired in src
check "① webhook.rejected EventRecord wired in src" \
  bash -c "grep -rq 'webhook\.rejected\|webhook_rejected' $CRATE/src/"

# ⑤ installation.revoked EventRecord wired in src
check "⑤ installation.revoked EventRecord wired in src" \
  bash -c "grep -rq 'installation\.revoked\|installation_revoked' $CRATE/src/"

# Claims boundary guards RETIRED (lead, wave D1.2 integration): the
# "sidecar//ui/ absent" asserts were B1-wave isolation fences; B6/B7 now
# legitimately own those paths.

finish
