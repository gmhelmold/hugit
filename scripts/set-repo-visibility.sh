#!/usr/bin/env bash
# set-repo-visibility.sh — set a repo's public/private flag on the live engine.
#
# One-shot operator op: POST /v1/repos/<repo>/repo/meta {"visibility":"..."}.
# The engine's `write_repo_meta` verb APPENDS a canonical repo.meta record; the
# read gate (authorize_read) + the git-wire public-clone gate reflect it
# immediately. APPEND-ONLY → fully REVERSIBLE (run again with the other value).
#
# Secrets NEVER on the command line or in this file: the bearer comes from the
# HUGIT_SMOKE_BEARER env var (out-of-band). Nothing here echoes the token.
# If HUGIT_SMOKE_BEARER is the engine DEV token, the engine must be booted with
# HUGIT_ALLOW_DEV_OPERATOR=1 (operator write) — it currently is. A real
# Clerk-minted owning-tenant token also authorizes it (no break-glass needed).
#
# Usage (run in YOUR terminal, token in the env — not on argv, not in chat):
#   HUGIT_SMOKE_BEARER='<engine-operator-or-owning-tenant-token>' \
#     scripts/set-repo-visibility.sh https://engine.githugr.com hugit public
#
#   $1 BASE   base URL, no trailing slash
#   $2 REPO   repo slug (e.g. hugit)
#   $3 VIS    "public" | "private"
set -euo pipefail

BASE="${1:?usage: HUGIT_SMOKE_BEARER=<token> $0 <BASE_URL> <REPO> <public|private>}"
REPO="${2:?repo slug required}"
VIS="${3:?visibility required: public|private}"

if [ -z "${HUGIT_SMOKE_BEARER:-}" ]; then
  echo "error: set HUGIT_SMOKE_BEARER (the bearer token) in the environment, not on argv" >&2
  exit 2
fi
if [ "$VIS" != "public" ] && [ "$VIS" != "private" ]; then
  echo "error: visibility must be exactly 'public' or 'private'" >&2
  exit 2
fi

URL="${BASE}/v1/repos/${REPO}/repo/meta"

# Every engine write requires an Idempotency-Key header ("toda escrita exige um
# Idempotency-Key") — without it the write 400s IDEMPOTENCY_REQUIRED. A fresh
# random hex per invocation (a retry of THE SAME logical op should reuse the key;
# here each run is a distinct explicit flip, so a new key is correct).
KEY="$(openssl rand -hex 16 2>/dev/null || od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"

# git User-Agent clears Cloudflare bot-protection (a non-git/browser UA gets 403 1010).
# -o /dev/null + -w prints ONLY the HTTP status — never the token, never the body.
CODE="$(curl -sS -X POST \
  -H "Authorization: Bearer ${HUGIT_SMOKE_BEARER}" \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: ${KEY}" \
  -A "git/2.43" \
  -d "{\"visibility\":\"${VIS}\"}" \
  -o /dev/null -w '%{http_code}' \
  "$URL" 2>/dev/null)"

echo "POST ${URL} visibility=${VIS} -> HTTP ${CODE}"
case "$CODE" in
  200|201|204) echo "OK — ${REPO} is now ${VIS}. Re-run with the other value to revert." ;;
  401) echo "DENIED (401) — token not accepted (bad token, or dev-token with HUGIT_ALLOW_DEV_OPERATOR=0)." >&2; exit 1 ;;
  403) echo "403 — bot-protection or non-owner. (UA is git/*; check the token's write authz.)" >&2; exit 1 ;;
  404) echo "404 — repo not found / not served." >&2; exit 1 ;;
  *)   echo "unexpected status ${CODE}" >&2; exit 1 ;;
esac
