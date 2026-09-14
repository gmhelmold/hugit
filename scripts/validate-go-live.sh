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
cd "$WORK/repo" || exit 1
git init -q -b main >/dev/null 2>&1; assert "git init works"
[ -f .git/hooks/post-commit ];       assert "git init copied the hook from template"
echo x > f.txt
git add f.txt >/dev/null 2>&1
git commit -qm first >/dev/null 2>&1
# hooks fire async — poll for the lazy-booted log + ref.update
LANDED=0
for _ in $(seq 1 60); do
  if [ -f .git/hugit/event-log.json ] && python3 -c "
import json, sys
r = json.load(open('.git/hugit/event-log.json'))
sys.exit(0 if r and any(x['kind'] == 'ref.update' for x in r) else 1)
" 2>/dev/null; then LANDED=1; break; fi
  sleep 0.2
done
[ "$LANDED" = "1" ];                assert "first commit lazy-boots log + captures ref.update"
sleep 3  # let detached capture workers release the canonical log lock

# ── 4. campaign + intent + check memoization ────────────────────────────────
step "4. campaign / intent / check (memoized)"
CAMP=""
CAMP_RC=1
for _ in $(seq 1 20); do
  set +e
  CAMP="$("$BIN" campaign open --campaign c1 --charter "first" --owner user:test 2>&1)"
  CAMP_RC=$?
  set -e
  if ! printf '%s' "$CAMP" | grep -q 'log_busy'; then break; fi
  sleep 0.5
done
[ "$CAMP_RC" -eq 0 ]; assert "campaign open"
printf '%s' "$CAMP" | grep -q '"opened":true'; assert "campaign opened:true"
IID="$("$BIN" intent new --charter "task" --acceptance "ok" --campaign c1 2>&1 | python3 -c "import sys,json; print(json.load(sys.stdin)['intent_id'])")"
[ -n "$IID" ];                     assert "intent new gives id"
# A custom, environment-independent check (exit 0 always) — a GOING-GREEN check.
# Using the builtin `fmt` would depend on rustfmt being installed and would
# record a synthetic RED in an uninstalled sandbox; a custom `true` is a stable,
# toolchain-independent green that still exercises the memo keys + AC.
R1="$("$BIN" check run --def smoke --cmd "true" --store 2>&1)";  assert "check run 1 (miss)"
printf '%s' "$R1" | grep -q '"cache_hit":false'; assert "check1 is a MISS"
printf '%s' "$R1" | grep -q '"exit":0';          assert "check1 ran green (exit 0)"
R2="$("$BIN" check run --def smoke --cmd "true" 2>&1)";          assert "check run 2 (hit)"
printf '%s' "$R2" | grep -q '"cache_hit":true';  assert "check2 is a HIT (memoized)"
printf '%s' "$R2" | grep -q '"local_executions":0'; assert "zero local exec on hit"
printf '%s' "$R2" | grep -q '"saved_ms"';        assert "saved_ms reported on hit"
EXPORT_LOG="$WORK/export-event-log.json"
"$BIN" campaign open --campaign export --charter export --owner user:export --log "$EXPORT_LOG" >/dev/null 2>&1
assert "export fixture log"

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
cd "$WORK" || exit 1
git -C repo worktree add -q -b feat/rate ../wt-rate >/dev/null 2>&1; assert "worktree add"
DOCKED=""
for _ in $(seq 1 50); do
  DOCKED="$("$BIN" dock ls --log repo/.git/hugit/event-log.json 2>&1)"
  if printf '%s' "$DOCKED" | grep -q '"state":"open"'; then break; fi
  sleep 0.2
done
printf '%s' "$DOCKED" | grep -q '"state":"open"'; assert "dock ls"
printf '%s' "$DOCKED" | grep -q '"state":"open"'; assert "a dock is open"
printf '%s' "$DOCKED" | grep -q 'feat/rate';       assert "the worktree's dock is listed"

# ── 7. export (exit guarantee) ──────────────────────────────────────────────
step "7. export"
"$BIN" export --log "$EXPORT_LOG" --out "$WORK/out" >/dev/null 2>&1; assert "export"
[ -f "$WORK/out/export.json" ];                    assert "export.json written"
[ -d "$WORK/out/repo.git" ];                       assert "git bundle written"
[ -f "$WORK/out/redaction-manifest.json" ];        assert "redaction manifest written"
python3 -c "import json; assert isinstance(json.load(open('$WORK/out/redaction-manifest.json'))['removals'], list)" 2>/dev/null
assert "redaction manifest has a removals list"

# ── 8. D14 + seal lifecycle ─────────────────────────────────────────────────
step "8. D14 author kind + campaign seal"
cd "$WORK/repo" || exit 1
# author-kind human requires --principal (D14) — refused WITHOUT it.
NOPR="$(HUGIT_LOG=".git/hugit/event-log.json" "$BIN" pr open --pr PR-NP --campaign c1 --author-kind human --commit "$SHA" 2>&1)" || true
printf '%s' "$NOPR" | grep -q '"error"';   assert "pr open human w/o principal yields a structured error (D14)"
# seal the campaign: needs the in-flight PR settled first? c1 has PR-1 landed; seal it.
SEAL="$("$BIN" campaign close --campaign c1 --log .git/hugit/event-log.json 2>&1)"; assert "campaign close (seal)"
printf '%s' "$SEAL" | grep -q '"closed":true';     assert "campaign closed:true"
SEAL2="$("$BIN" campaign close --campaign c1 --log .git/hugit/event-log.json 2>&1)"; assert "campaign close idempotent"
printf '%s' "$SEAL2" | grep -q '"already_closed":true'; assert "second close → already_closed"
# a sealed campaign REFUSES new PR (and new intent, via the default log).
BLOQ="$("$BIN" pr open --pr PR-L --campaign c1 --author-kind human --principal user:test --commit "$SHA" --log .git/hugit/event-log.json 2>&1)" || true
printf '%s' "$BLOQ" | grep -q '"campaign_sealed"'; assert "sealed campaign REFUSES a new PR (kind campaign_sealed)"
BLOQI="$("$BIN" intent new --charter late --campaign c1 --id intent-sealfail --store "$WORK/intents.json" --log .git/hugit/event-log.json 2>&1)" || true
printf '%s' "$BLOQI" | grep -q '"campaign_sealed"'; assert "sealed campaign REFUSES intent new (kind campaign_sealed)"

# ── 9. dock land byte-identity + reconcile ──────────────────────────────────
step "9. dock land (verified) + ghost reconcile"
DOCKID="$("$BIN" dock ls --log .git/hugit/event-log.json 2>&1 | python3 -c "import sys,json; d=json.load(sys.stdin); print([x['dock_id'] for x in d if x['branch']=='feat/rate'][0])")"
[ -n "$DOCKID" ];                                  assert "resolved the feat/rate dock id"
# commit in the worktree, capture it, then dock land must VERIFY.
cd "$WORK/wt-rate" || exit 1
echo z >> f.txt
git add f.txt >/dev/null 2>&1
git commit -qm more >/dev/null 2>&1
sleep 2  # async capture; allow the detached worker to append before dock land
LANDD=""
LAND_RC=1
for _ in $(seq 1 20); do
  set +e
  LANDD="$("$BIN" dock land --id "$DOCKID" --log "$WORK/repo/.git/hugit/event-log.json" 2>&1)"
  LAND_RC=$?
  set -e
  if ! printf '%s' "$LANDD" | grep -q 'log_busy'; then break; fi
  sleep 0.5
done
[ "$LAND_RC" -eq 0 ];                           assert "dock land"
printf '%s' "$LANDD" | grep -q '"byte_identity":"verified"'; assert "byte_identity verified"
printf '%s' "$LANDD" | grep -q '"landed":true';     assert "dock landed:true"
# remove the worktree → dock reconcile closes the ghost.
cd "$WORK" || exit 1
git -C repo worktree remove --force wt-rate >/dev/null 2>&1; assert "worktree remove"
"$BIN" dock reconcile --log repo/.git/hugit/event-log.json >/dev/null 2>&1; assert "dock reconcile"
# the dock was already closed by land; reconciling again stays idempotent.
"$BIN" dock reconcile --log repo/.git/hugit/event-log.json >/dev/null 2>&1; assert "dock reconcile idempotent"
LS2="$("$BIN" dock ls --log repo/.git/hugit/event-log.json 2>&1)"; assert "dock ls after remove"
printf '%s' "$LS2" | grep -q '"state":"ghost"';    assert "the removed worktree's dock shows ghost"

# ── 10. cost honesty (ctx usage verbatim) + dock insight ───────────────────
step "10. cost honesty + dock insight residual"
cd "$WORK/repo" || exit 1
CU="$("$BIN" ctx usage --intent "$IID" --model claude-drop --input 120 --output 40 --cache-read 0 --cache-write 0 --log .git/hugit/event-log.json 2>&1)"; assert "ctx usage records"
printf '%s' "$CU" | grep -q '"target_kind":"intent"'; assert "usage targets the intent"
printf '%s' "$CU" | grep -q '"total":160';        assert "usage totals input+output (120+40)"
INS="$("$BIN" dock insight --log .git/hugit/event-log.json 2>&1)"; assert "dock insight"
printf '%s' "$INS" | grep -q '"matched_usd_micros":0';   assert "insight honest-zero cost (no gateway sample)"
printf '%s' "$INS" | grep -q '"unlabeled_commit_count"'; assert "insight residual bucket present"

# ── 11. policy / undo / symbol / why / verdict ──────────────────────────────
step "11. policy, undo, symbol, why, verdict"
PCA="$("$BIN" policy test --context /dev/null 2>&1)" || true
printf '%s' "$PCA" | grep -q '"error"';  assert "policy test with a missing context file yields a structured error"
echo '{"commit_messages":["feat: x"],"commit_parent_counts":[1],"changed_files":["a.rs"],"file_contents":{},"metadata":{}}' > "$WORK/ctx.json"
PG="$("$BIN" policy test --context "$WORK/ctx.json" 2>&1)"; assert "policy test with context"
printf '%s' "$PG" | grep -q '"dco"';              assert "dco gate evaluated"
mkdir -p "$WORK/srcsym"
printf 'fn main() {}
struct Point { x: i32 }
' > "$WORK/srcsym/point.rs"
SYM="$("$BIN" symbol --file "$WORK/srcsym/point.rs" 2>&1)"; assert "symbol outline"
printf '%s' "$SYM" | grep -q '"lang":"rust"';     assert "symbol language rust"
printf '%s' "$SYM" | grep -q '"fn"';              assert "symbols include a function"
WHY="$("$BIN" why --log .git/hugit/event-log.json --path src/nope.rs 2>&1)" || true
printf '%s' "$WHY" | grep -q '"unresolved"';      assert "why on an unattributed path → unresolved (never fabricates)"
UNDO_SEQ="$(python3 -c "import json; r=json.load(open('.git/hugit/event-log.json')); print(next(x['seq'] for x in r if x['kind'] != 'ref.update'))")"
set +e
UD="$("$BIN" undo --seq "$UNDO_SEQ" --log .git/hugit/event-log.json 2>&1)"
set -e
printf '%s' "$UD" | grep -q '"nothing_to_compensate"'; assert "undo on a non-ref event → nothing_to_compensate (honest)"
# verdict approve — single-lens human, persists a verdict.recorded.
VERD="$("$BIN" verdict approve --intent "$IID" --log .git/hugit/event-log.json 2>&1)"
printf '%s' "$VERD" | grep -q '"verdict_recorded":true'; assert "verdict approve persists verdict_recorded:true"

echo
VERSION_STR="$("$BIN" -V 2>&1)"
echo "ALL PASS (${FAILED} failures) — go-live check succeeded ($VERSION_STR)."
exit 0
