#!/usr/bin/env bash
# Benchmark every shipped local CLI ledger family against a real Git checkout.
# Reports are intentionally retained: pass/fail claims need inspectable evidence.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${HUGIT_BIN:-$ROOT/target/release/hugit}"
REPORT="${HUGIT_BENCHMARK_REPORT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/hugit-feature-ledger.XXXXXX")}"
WORK="$REPORT/work"
NDJSON="$REPORT/cases.ndjson"
SUMMARY="$REPORT/summary.json"
MANIFEST="$REPORT/manifest.json"
mkdir -p "$REPORT" "$WORK"
: > "$NDJSON"

failures=0
cases=0
required=()

if [ ! -x "$BIN" ]; then
  (cd "$ROOT" && cargo build -p hugit-cli --release --locked) || {
    printf 'FAIL: release hugit build\n' >&2
    exit 1
  }
fi

# Preserve existing go-live evidence. The prerequisite uses the same canonical
# Git-common-dir runtime state as this matrix.
PREREQ_DIR="$REPORT/go-live-prerequisite"
mkdir -p "$PREREQ_DIR"
set +e
HUGIT_BIN="$BIN" "$ROOT/scripts/validate-go-live.sh" "$PREREQ_DIR" >"$REPORT/go-live.stdout" 2>"$REPORT/go-live.stderr"
prerequisite_rc=$?
set -e
if [ "$prerequisite_rc" -eq 0 ]; then
  prerequisite_status=pass
else
  prerequisite_status=fail
  failures=$((failures + 1))
fi

export HOME="$WORK/home"
export XDG_CONFIG_HOME="$WORK/xdg"
export XDG_CACHE_HOME="$WORK/cache"
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL="$HOME/.gitconfig"
export GIT_TERMINAL_PROMPT=0
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME"

REPO="$WORK/hello-world"
HUGIT_BENCHMARK_REPO="${HUGIT_BENCHMARK_REPO:-https://github.com/octocat/Hello-World.git}"
git clone --depth=1 "$HUGIT_BENCHMARK_REPO" "$REPO" >"$REPORT/clone.stdout" 2>"$REPORT/clone.stderr" || {
  printf 'FAIL: source clone (%s)\n' "$HUGIT_BENCHMARK_REPO" >&2
  exit 1
}
git -C "$REPO" config user.name benchmark
git -C "$REPO" config user.email benchmark@example.invalid
SOURCE_COMMIT="$(git -C "$REPO" rev-parse HEAD)"
ZERO_OID=0000000000000000000000000000000000000000
OID="$SOURCE_COMMIT"
LOG="$REPO/.git/hugit/event-log.json"
export HUGIT_LOG="$LOG"
export HUGIT_BIN="$BIN"

record_case() {
  local name="$1" expected="$2" rc="$3" out="$4" err="$5" command="$6" attempts="$7" status
  if [ "$expected" = "live-success" ] || [ "$expected" = "silent-success" ]; then
    if [ "$rc" -eq 0 ] && { [ "$expected" = "silent-success" ] || python3 - "$out" <<'PY'
import json, sys
try:
    json.load(open(sys.argv[1]))
except (OSError, json.JSONDecodeError):
    raise SystemExit(1)
PY
    }; then
      status=pass
    else
      status=fail
    fi
  else
    if [ "$rc" -ne 0 ] && python3 - "$out" <<'PY'
import json, sys
try:
    value = json.load(open(sys.argv[1]))
except (OSError, json.JSONDecodeError):
    raise SystemExit(1)
error = value.get("error") if isinstance(value, dict) else None
raise SystemExit(0 if isinstance(error, dict) and isinstance(error.get("kind"), str) and isinstance(error.get("fix"), str) else 1)
PY
    then
      status=pass
    else
      status=fail
    fi
  fi
python3 - "$NDJSON" "$name" "$expected" "$rc" "$status" "$command" "$attempts" "$out" "$err" <<'PY'
import json, pathlib, sys
dest, name, expected, rc, status, command, attempts, out, err = sys.argv[1:]
def read(path):
    return pathlib.Path(path).read_text(errors="replace")
row = {
    "case": name,
    "expected_class": expected,
    "exit": int(rc),
    "stdout": read(out),
    "stderr": read(err),
    "command": command,
    "attempts": int(attempts),
    "status": status,
}
with open(dest, "a", encoding="utf-8") as f:
    f.write(json.dumps(row, sort_keys=True) + "\n")
PY
  cases=$((cases + 1))
  required+=("$name")
  [ "$status" = pass ] || failures=$((failures + 1))
}

run_case() {
  local name="$1" expected="$2"; shift 2
  local out err rc attempts=0
  out="$REPORT/${cases}_$(printf '%s' "$name" | tr '/ ' '__').stdout"
  err="$REPORT/${cases}_$(printf '%s' "$name" | tr '/ ' '__').stderr"
  local command="$BIN" arg
  for arg in "$@"; do command="$command $(printf '%q' "$arg")"; done
  while [ "$attempts" -lt 20 ]; do
    attempts=$((attempts + 1))
    set +e
    (cd "$REPO" && "$BIN" "$@") >"$out" 2>"$err"
    rc=$?
    set -e
    if ! grep -q 'log_busy' "$out" || [ "$rc" -eq 0 ]; then break; fi
    sleep 0.5
  done
  record_case "$name" "$expected" "$rc" "$out" "$err" "$command" "$attempts"
}

set -e

# Hook lifecycle and all capture kinds.
run_case setup live-success setup --repo "$REPO"
# Keep one legacy hook so attach adoption has one deterministic hash-bound token.
for hook in post-checkout pre-push post-merge post-rewrite reference-transaction; do
  rm -f "$REPO/.git/hooks/$hook"
done
run_case attach-preview live-success attach --repo "$REPO" --preview
preview_token=""
while IFS= read -r token; do
  if [ -n "$token" ]; then preview_token="$token"; break; fi
done < <(python3 - "$REPORT/1_attach-preview.stdout" <<'PY'
import json, sys
for change in json.load(open(sys.argv[1])).get("changes", []):
    token = change.get("preview_token")
    if token:
        print(token)
PY
)
[ -n "$preview_token" ] || { printf '%s\n' 'FAIL: attach preview returned no adoption token' >&2; exit 1; }
run_case attach-adopt live-success attach --repo "$REPO" --adopt-managed-dispatcher "$preview_token"
run_case health live-success health --dir "$REPO"
for kind in commit checkout push-attempt merge rewrite reference-transaction; do
  run_case "capture-$kind" silent-success capture --kind "$kind" --top-level "$REPO" --log "$LOG" --oid "$OID" --branch main --from "$ZERO_OID"
done
sleep 1  # allow detached capture workers to release the canonical log lock
# Create committed evidence in cloned public repo after hook setup.
printf '%s\n' 'fn benchmark_symbol() {}' > "$REPO/benchmark.rs"
git -C "$REPO" add benchmark.rs
git -C "$REPO" commit -qm 'benchmark: symbol fixture'
sleep 1
SHA="$(git -C "$REPO" rev-parse HEAD)"
run_case capture-committed silent-success capture --kind commit --top-level "$REPO" --log "$LOG" --oid "$SHA" --branch main
sleep 1
CLEAN_LOG="$WORK/clean-event-log.json"
run_case export-log-seed live-success campaign open --campaign export --charter export --owner user:export --log "$CLEAN_LOG"
run_case capture-reference silent-success capture --kind reference-transaction --top-level "$REPO" --log "$LOG" --reference-tuples "$ZERO_OID $SHA refs/heads/master" --transaction-phase committed
run_case capture-undo-base silent-success capture --kind commit --top-level "$REPO" --log "$LOG" --oid "$ZERO_OID" --branch main
run_case capture-undo-tip silent-success capture --kind commit --top-level "$REPO" --log "$LOG" --oid "$OID" --branch main
UNDO_SEQ="$(python3 - "$LOG" <<'PY'
import json, sys
print(len(json.load(open(sys.argv[1]))) - 1)
PY
)"
run_case detach live-success detach --dir "$REPO"

# Campaign, intent, issue, PR, queue, checks, verdict, landing.
run_case campaign-open live-success campaign open --campaign bench --charter benchmark --owner user:benchmark --log "$LOG"
run_case intent-new live-success intent new --id intent-bench --charter benchmark --acceptance pass --campaign bench --store "$REPO/.hugit/intents.json" --log "$LOG"
run_case campaign-show live-success campaign show --campaign bench --log "$LOG"
run_case campaign-list live-success campaign list --log "$LOG"
run_case intent-show live-success intent show --intent intent-bench --store "$REPO/.hugit/intents.json"
run_case intent-list live-success intent list --store "$REPO/.hugit/intents.json" --log "$LOG"
run_case issue-transition live-success issue transition --n 1 --to open --log "$LOG"
run_case pr-open live-success pr open --pr PR-BENCH --campaign bench --author-kind human --principal user:benchmark --intent intent-bench --log "$LOG"
run_case pr-show live-success pr show --pr PR-BENCH --log "$LOG"
run_case pr-list live-success pr list --log "$LOG"
run_case pr-queue live-success pr queue --pr PR-BENCH --log "$LOG"
run_case queue-show live-success queue show --log "$LOG"
run_case check-run live-success check run --def benchmark --cmd true --store --log "$LOG"
run_case check-show live-success check show --log "$LOG"
run_case check-key live-success check key --tree 0000000000000000000000000000000000000000000000000000000000000000 --def 1111111111111111111111111111111111111111111111111111111111111111 --toolchain 2222222222222222222222222222222222222222222222222222222222222222
run_case verdict-record live-success verdict record --intent intent-bench --store --log "$LOG" --lens security --result approve
run_case verdict-approve live-success verdict approve --intent intent-bench --log "$LOG"
run_case land-queue live-success land queue --log "$LOG"
run_case pr-land live-success pr land --pr PR-BENCH --log "$LOG"
run_case pr-land-dispatch fail-closed pr land --pr PR-BENCH --dispatch --log "$LOG"

# Evidence/projection families.
run_case why fail-closed why --log "$LOG" --path absent.txt
run_case why-walk live-success why --log "$LOG" --path absent.txt --walk
cat > "$WORK/graph.json" <<'JSON'
{"ecosystem":"cargo","root_manifests":["Cargo.toml"],"packages":[{"name":"app","path":".","direct_deps":["lib"]},{"name":"lib","path":"lib","direct_deps":[]}]}
JSON
run_case impact live-success impact --graph "$WORK/graph.json" --path benchmark.rs
run_case tournament live-success tournament --candidates 2 --intent intent-bench
run_case export live-success export --log "$CLEAN_LOG" --out "$WORK/export"
run_case undo live-success undo --seq "$UNDO_SEQ" --log "$LOG"
cat > "$WORK/context.json" <<'JSON'
{"commit_messages":["benchmark"],"commit_parent_counts":[1],"changed_files":["benchmark.rs"],"file_contents":{},"metadata":{}}
JSON
run_case policy-test live-success policy test --context "$WORK/context.json"
run_case policy-edit live-success policy edit --gate dco --disable --principal user:benchmark --log "$LOG"
run_case note live-success note --note benchmark --workspace bench-workspace --intent intent-bench --log "$LOG"
run_case diag fail-closed diag --def-digest missing --log "$LOG"
run_case ledger live-success ledger --log "$LOG"
run_case fleet live-success fleet --log "$LOG"
run_case watch live-success watch --log "$LOG"
run_case symbol-local live-success symbol --file "$REPO/benchmark.rs"
run_case symbol-committed live-success symbol --ref HEAD --path benchmark.rs --git-dir "$REPO"
run_case ctx-resume live-success ctx resume --workspace bench-workspace --intent intent-bench --log "$LOG" --now-ms 0
run_case review live-success review --question 'what checks ran?' --intent intent-bench --log "$LOG"
run_case meta live-success meta set --visibility private --owner-tenant benchmark --by user:benchmark --log "$LOG"

# Dock surface. Explicit coin gives deterministic local evidence.
GITDIR="$(git -C "$REPO" rev-parse --absolute-git-dir)"
BRANCH="$(git -C "$REPO" branch --show-current)"
run_case dock-coin silent-success dock coin --top-level "$REPO" --gitdir "$GITDIR" --branch "$BRANCH" --log "$LOG"
DOCK_ID="$(python3 - "$LOG" <<'PY'
import json, sys
try:
    rows = json.load(open(sys.argv[1]))
except Exception:
    rows = []
ids = []
for row in rows:
    if row.get("kind") not in ("dock.coined", "dock.record"):
        continue
    payload = row.get("payload", {})
    if isinstance(payload, str):
        try:
            payload = json.loads(payload)
        except json.JSONDecodeError:
            payload = {}
    if payload.get("dock_id"):
        ids.append(payload["dock_id"])
print(ids[-1] if ids else "")
PY
)"
if [ -n "$DOCK_ID" ]; then
  run_case dock-ls live-success dock ls --log "$LOG"
  run_case dock-show live-success dock show "$DOCK_ID" --log "$LOG"
  run_case dock-insight live-success dock insight --log "$LOG"
  run_case dock-close live-success dock close --id "$DOCK_ID" --log "$LOG"
  run_case dock-reconcile live-success dock reconcile --log "$LOG"
else
  printf '%s\n' 'dock coin produced no dock id' > "$REPORT/dock.stderr"
  failures=$((failures + 1))
fi

# Namespace and execution boundaries: rejection is the expected proof.
run_case ws-discontinued rejected ws
run_case dispatch-discontinued rejected dispatch

GIT_VERSION="$(git --version)"
BIN_VERSION="$("$BIN" -V 2>&1)"
PLATFORM="$(uname -a)"
python3 - "$NDJSON" "$SUMMARY" "$MANIFEST" "$cases" "$failures" "$HUGIT_BENCHMARK_REPO" "$SOURCE_COMMIT" "$BIN" "$BIN_VERSION" "$GIT_VERSION" "$PLATFORM" "$prerequisite_status" "${required[@]}" <<'PY'
import json, pathlib, sys
ndjson, summary, manifest, count, failures, source, source_commit, binary, binary_version, git_version, platform, prerequisite, *required = sys.argv[1:]
rows = [json.loads(line) for line in pathlib.Path(ndjson).read_text().splitlines() if line]
result = {
    "benchmark": "hugit-feature-ledger",
    "source_repo": source,
    "source_commit": source_commit,
    "binary": binary,
    "binary_version": binary_version,
    "git_version": git_version,
    "platform": platform,
    "cases": int(count),
    "failures": int(failures),
    "status": "pass" if int(failures) == 0 and len(rows) == int(count) else "fail",
    "ndjson": ndjson,
    "required_cases": required,
    "go_live_prerequisite": prerequisite,
}
pathlib.Path(summary).write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
pathlib.Path(manifest).write_text(json.dumps({"schema": "hugit-benchmark-v1", "files": [ndjson, summary], "case_count": len(rows)}, indent=2) + "\n")
PY

if [ "$failures" -eq 0 ]; then
  printf 'REPORT: %s\n' "$REPORT"
  exit 0
fi
printf 'REPORT: %s\n' "$REPORT" >&2
exit 1
