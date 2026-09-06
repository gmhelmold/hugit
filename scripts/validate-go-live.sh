#!/usr/bin/env bash
# validate-go-live.sh — the go-live product check (item 3).
#
# A scripted, deterministic validation of the hugit CLI as a real user would
# use it. Run by a sub-agent inside a pre-prepared worktree. Isolated: uses a
# temp HOME + XDG (never touches the machine's real gitconfig), a fresh git
# repo, and the RELEASE binary.
#
# EXIT: 0 = every step PASS. Non-zero on the FIRST failure (fail-fast), with a
# `FAIL:` line naming the step. Each step echoes `PASS: <what proved>`.
#
# Usage: HUGIT_BIN=/abs/path/to/hugit ./validate-go-live.sh [workdir]
#   workdir defaults to a fresh mktemp dir under $TMPDIR.

set -u
BIN="${HUGIT_BIN:?HUGIT_BIN must be a path to the hugit binary}"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

WORK="${1:-$(mktemp -d "${TMPDIR:-/tmp}/hugit-golive.XXXXXX")}"
mkdir -p "$WORK/gitconfig-home" "$WORK/xdg" "$WORK/repo"
export HOME="$WORK/gitconfig-home"
export XDG_CONFIG_HOME="$WORK/xdg"
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL="$HOME/.gitconfig"
export HUGIT_BIN="$BIN"

declare -i FAILED=0
step() { echo; echo "## $1"; }
# assert <label> — PASS if the PREVIOUS command exited 0, else FAIL + exit 1.
assert() {
  local label="$1" rc=$?
  if [ $rc -eq 0 ]; then
    echo "   PASS: $label"
  else
    echo "   FAIL: $label (rc=$rc)"
    exit 1
  fi
}
jq() { python3 -c "import sys,json; v=json.load(sys.stdin); print(json.dumps(eval(sys.argv[1]),ensure_ascii=False))" "$1"; }

# ── 1. binary sanity ──────────────────────────────────────────────────────────
step "1. binary: -V and --help"
V="$("$BIN" -V 2>&1)";            assert "version prints ($V)"
"$BIN" --help 2>&1 | grep -q "why"; assert "help lists verbs"
"$BIN" --help 2>&1 | grep -q "dock"; assert "help lists dock"

# ── 2. setup: template + global init.templateDir ─────────────────────────────
step "2. hugit setup"
SETUP="$("$BIN" setup 2>&1)";     assert "setup exits 0"
printf '%s' "$SETUP" | grep -q "template_dir"; assert "setup returns template_dir"
TPL="$(printf '%s' "$SETUP" | python3 -c "import sys,json; print(json.load(sys.stdin)['template_dir'])")"
[ -f "$TPL/hooks/post-commit" ];    assert "template post-commit hook written"
[ -f "$TPL/hooks/post-checkout" ];  assert "template post-checkout hook written"
[ -f "$TPL/OWNED-BY-HUGIT" ];       assert "ownership marker written"
CFG="$(git config --global init.templateDir 2>/dev/null)"
[ "$CFG" = "$TPL" ];                assert "git global init.templateDir set"

# ── 3. fresh git init auto-ships hooks + lazy log on first commit ────────────
step "3. git init (template) + lazy boot"
cd "$WORK/repo"
git init -q -b main >/dev/null 2>&1; assert "git init works"
[ -f .git/hooks/post-commit ];       assert "git init copied the hook from template"
echo x > f.txt
git add f.txt >/dev/null 2>&1
git commit -qm first >/dev/null 2>&1
# hooks fire async — poll for the lazy-booted log + ref.update
LANDED=0
for _ in $(seq 1 60); do
  if [ -f .hugit/log.json ] && python3 -c "
import json, sys
r = json.load(open('.hugit/log.json'))
sys.exit(0 if r and any(x['kind'] == 'ref.update' for x in r) else 1)
" 2>/dev/null; then LANDED=1; break; fi
  sleep 0.2
done
[ "$LANDED" = "1" ];                assert "first commit lazy-boots log + captures ref.update"

# ── 4. campaign + intent + check memoization ────────────────────────────────
step "4. campaign / intent / check (memoized)"
CAMP="$("$BIN" campaign open --campaign c1 --charter "first" --owner user:test 2>&1)"; assert "campaign open"
printf '%s' "$CAMP" | grep -q '"opened":true'; assert "campaign opened:true"
IID="$("$BIN" intent new --charter "task" --acceptance "ok" --campaign c1 2>&1 | python3 -c "import sys,json; print(json.load(sys.stdin)['intent_id'])")"
[ -n "$IID" ];                     assert "intent new gives id"
R1="$("$BIN" check run --def fmt --store 2>&1)";  assert "check run 1 (miss)"
printf '%s' "$R1" | grep -q '"cache_hit":false'; assert "check1 is a MISS"
R2="$("$BIN" check run --def fmt 2>&1)";          assert "check run 2 (hit)"
printf '%s' "$R2" | grep -q '"cache_hit":true';  assert "check2 is a HIT (memoized)"
printf '%s' "$R2" | grep -q '"local_executions":0'; assert "zero local exec on hit"

# ── 5. pr open / queue / land (union engine) ────────────────────────────────
step "5. PR flow + union landing"
SHA="$(git rev-parse HEAD)"
"$BIN" pr open --pr PR-1 --campaign c1 --author-kind human --principal user:test --commit "$SHA" >/dev/null 2>&1
assert "pr open with captured commit"
"$BIN" pr queue --pr PR-1 >/dev/null 2>&1;  assert "pr queue"
LAND="$("$BIN" land queue 2>&1)";                 assert "land queue"
printf '%s' "$LAND" | grep -q '"landed":\[';     assert "landed set present"
printf '%s' "$LAND" | grep -q '"PR-1"';          assert "PR-1 landed"
printf '%s' "$LAND" | grep -q '"verdict":"green"'; assert "union verdict green"
"$BIN" pr show --pr PR-1 >/dev/null 2>&1;         assert "pr show"

# ── 6. dock: worktree coin + ls ─────────────────────────────────────────────
step "6. dock (worktree cost unit)"
cd "$WORK"
git -C repo worktree add -q -b feat/rate ../wt-rate >/dev/null 2>&1; assert "worktree add"
sleep 0.5
DOCKED="$("$BIN" dock ls --log repo/.hugit/log.json 2>&1)"; assert "dock ls"
printf '%s' "$DOCKED" | grep -q '"state":"open"'; assert "a dock is open"
printf '%s' "$DOCKED" | grep -q 'feat/rate';       assert "the worktree's dock is listed"

# ── 7. export (exit guarantee) ──────────────────────────────────────────────
step "7. export"
EXPORT="$("$BIN" export --log repo/.hugit/log.json --out "$WORK/out" 2>&1)"; assert "export"
[ -f "$WORK/out/export.json" ];                    assert "export.json written"
[ -d "$WORK/out/repo.git" ];                       assert "git bundle written"

echo
echo "ALL PASS (${FAILED} failures) — hugit 0.1.0 go-live check succeeded."
exit 0