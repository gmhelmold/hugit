#!/usr/bin/env bash
# write-smoke.sh — joint live-write smoke for the /v1 engine (close-the-product cutover).
#
# Proves the deployed engine accepts a REAL write end-to-end: 200 + Accepted, the
# idempotency ledger replays byte-identically, a key reused with a different body 409s,
# the write persists (its seq shows up in the event stream), and the auth gate rejects
# a bad token. Uses `comment` as the probe verb — append-only, does NOT mutate landing
# queue state, so it is safe to run against a live repo.
#
# Secrets NEVER on the command line or in this file: the bearer token comes from the
# HUGIT_SMOKE_BEARER env var (out-of-band). Nothing here echoes the token.
#
# Usage:
#   HUGIT_SMOKE_BEARER='<engine-or-dev-token>' \
#     scripts/write-smoke.sh https://engine.githugr.com hugit [PR_NUMBER]
#
# Args:
#   $1  BASE     base URL, no trailing slash (e.g. https://engine.githugr.com)
#   $2  REPO     repo slug (e.g. hugit)
#   $3  PR       PR number to comment on (default: 1)
#
# Exit: 0 = all checks PASS; 1 = a check FAILED; 2 = bad invocation.
set -u

BASE="${1:-}"
REPO="${2:-}"
PR="${3:-1}"

if [[ -z "$BASE" || -z "$REPO" ]]; then
  echo "usage: HUGIT_SMOKE_BEARER=<token> $0 <BASE_URL> <REPO> [PR_NUMBER]" >&2
  exit 2
fi
if [[ -z "${HUGIT_SMOKE_BEARER:-}" ]]; then
  echo "error: set HUGIT_SMOKE_BEARER (the bearer token) in the environment, not on argv" >&2
  exit 2
fi

AUTH="Authorization: Bearer ${HUGIT_SMOKE_BEARER}"
COMMENTS_URL="${BASE}/v1/repos/${REPO}/prs/${PR}/comments"
EVENTS_URL="${BASE}/v1/repos/${REPO}/events"

PASS=0
FAIL=0
ok()   { echo "  ✅ PASS — $1"; PASS=$((PASS+1)); }
bad()  { echo "  ❌ FAIL — $1"; FAIL=$((FAIL+1)); }

# curl helper: prints "HTTP_STATUS<newline>BODY". Never prints the token.
# $1 method, $2 url, $3 idempotency-key ("" to omit), $4 body ("" to omit)
req() {
  local method="$1" url="$2" key="$3" data="$4"
  local args=(-sS -X "$method" -H "$AUTH" -H "Content-Type: application/json"
              -w $'\n%{http_code}' -o -)
  [[ -n "$key" ]]  && args+=(-H "Idempotency-Key: ${key}")
  [[ -n "$data" ]] && args+=(--data "$data")
  curl "${args[@]}" "$url" 2>/dev/null
}
status_of() { printf '%s' "$1" | tail -n1; }
body_of()   { printf '%s' "$1" | sed '$d'; }
seq_of()    { printf '%s' "$1" | grep -o '"seq"[[:space:]]*:[[:space:]]*[0-9]*' | head -n1 | grep -o '[0-9]*$'; }

STAMP="$(date -u +%Y%m%dT%H%M%SZ 2>/dev/null || echo run)"
KEY="smoke-${STAMP}-$$"
BODY1="{\"body\":\"write-smoke ${STAMP} — cutover joint smoke (append-only probe)\"}"
BODY2="{\"body\":\"write-smoke ${STAMP} — DIFFERENT body, same key (409 probe)\"}"

echo "== /v1 live-write smoke =="
echo "   base=${BASE}  repo=${REPO}  pr=${PR}  key=${KEY}"
echo

# 0) baseline event-stream tip (so we can prove the new seq appears AFTER it)
R="$(req GET "${EVENTS_URL}?since=0" "" "")"
if [[ "$(status_of "$R")" == "200" ]]; then
  BASE_MAX="$(printf '%s' "$(body_of "$R")" | grep -o '"seq"[[:space:]]*:[[:space:]]*[0-9]*' | grep -o '[0-9]*$' | sort -n | tail -n1)"
  BASE_MAX="${BASE_MAX:-0}"
  ok "events replay readable (baseline tip seq=${BASE_MAX})"
else
  bad "events replay GET returned $(status_of "$R") (expected 200)"; BASE_MAX=0
fi

# 1) auth gate — a bogus token must be rejected (401), not silently accepted
R="$(curl -sS -X POST -H "Authorization: Bearer not-a-real-token" \
        -H "Content-Type: application/json" -H "Idempotency-Key: ${KEY}-authneg" \
        -w $'\n%{http_code}' -o - --data "$BODY1" "$COMMENTS_URL" 2>/dev/null)"
case "$(status_of "$R")" in
  401) ok "auth gate rejects a bad bearer (401)" ;;
  *)   bad "bad bearer returned $(status_of "$R") (expected 401)" ;;
esac

# 2) the WRITE — 200 + Accepted{seq}
R="$(req POST "$COMMENTS_URL" "$KEY" "$BODY1")"
ST="$(status_of "$R")"; SEQ1="$(seq_of "$R")"
if [[ "$ST" == "200" && -n "$SEQ1" ]]; then
  ok "comment write accepted (200, seq=${SEQ1})"
else
  bad "comment write returned ${ST}, seq='${SEQ1}' (expected 200 + seq); body: $(body_of "$R")"
fi

# 3) idempotent replay — SAME key + SAME body → byte-identical 200 (same seq)
R="$(req POST "$COMMENTS_URL" "$KEY" "$BODY1")"
SEQ2="$(seq_of "$R")"
if [[ "$(status_of "$R")" == "200" && -n "$SEQ1" && "$SEQ2" == "$SEQ1" ]]; then
  ok "idempotent replay returns the same outcome (seq=${SEQ2})"
else
  bad "replay returned $(status_of "$R"), seq='${SEQ2}' (expected 200 + seq=${SEQ1})"
fi

# 4) conflict — SAME key + DIFFERENT body → 409
R="$(req POST "$COMMENTS_URL" "$KEY" "$BODY2")"
case "$(status_of "$R")" in
  409) ok "idempotency-key reuse with a different body 409s" ;;
  *)   bad "key+different-body returned $(status_of "$R") (expected 409)" ;;
esac

# 5) persistence read-back — the event stream must have GROWN past the baseline.
#    The write returns the engine's INTERNAL (0-based) seq; the event stream emits
#    the WIRE seq (`internal + 1`, frozen client contract). Rather than couple to
#    that offset, we assert the stream tip advanced beyond the baseline AND a new
#    "comment" frame is present — proof the write persisted to the durable log.
if [[ -n "$SEQ1" ]]; then
  R="$(req GET "${EVENTS_URL}?since=${BASE_MAX}" "" "")"
  NEW_BODY="$(body_of "$R")"
  NEW_MAX="$(printf '%s' "$NEW_BODY" | grep -o '"seq"[[:space:]]*:[[:space:]]*[0-9]*' | grep -o '[0-9]*$' | sort -n | tail -n1)"
  NEW_MAX="${NEW_MAX:-0}"
  if [[ "$NEW_MAX" -gt "$BASE_MAX" ]] && printf '%s' "$NEW_BODY" | grep -q '"kind"[[:space:]]*:[[:space:]]*"comment"'; then
    ok "write persisted — stream advanced ${BASE_MAX}→${NEW_MAX} with a new comment frame"
  else
    bad "stream did not advance past ${BASE_MAX} (tip=${NEW_MAX}) or no comment frame (write did not persist?)"
  fi
fi

echo
echo "== result: ${PASS} passed, ${FAIL} failed =="
[[ "$FAIL" -eq 0 ]]
