#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/hugit-evidence-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
FAKE="$TMP/fake-hugit.py"

cat >"$FAKE" <<'PY'
#!/usr/bin/env python3
import hashlib, json, os, pathlib, struct, subprocess, sys

args = sys.argv[1:]
def arg(name, default=None):
    return args[args.index(name)+1] if name in args else default
def emit(value, code=0):
    print(json.dumps(value, sort_keys=True)); raise SystemExit(code)
def logpath(): return pathlib.Path(arg("--log"))
def records():
    p=logpath(); return json.loads(p.read_text()) if p.exists() else []
def append(kind, value, principal=None):
    rows=records(); seq=len(rows); prev=rows[-1]["this_hash"] if rows else "0"*64; principals=principal or []; encoded=json.dumps(value,sort_keys=True,separators=(",",":")); lp=lambda s:struct.pack(">I",len(s.encode()))+s.encode(); pre=lp(prev)+lp(kind)+struct.pack(">I",len(principals))+b"".join(lp(x) for x in principals)+lp(encoded)+struct.pack(">Q",seq); rows.append({"seq":seq,"prev_hash":prev,"kind":kind,"principal_chain":principals,"payload":encoded,"this_hash":hashlib.sha256(pre).hexdigest(),"recorded_at":0}); logpath().parent.mkdir(parents=True,exist_ok=True); logpath().write_text(json.dumps(rows)); return rows[-1]
if args == ["--version"]: print("hugit-fake 1.0"); raise SystemExit(0)
if args and args[0] in ("ws","dispatch"):
    emit({"error":{"kind":"unknown_command","message":"reserved and discontinued","fix":"use CLI-local verbs"}},2)
if args[:1] == ["setup"]:
    repo=pathlib.Path(arg("--repo")); hooks=repo/".git/hooks"
    kinds={"post-commit":"commit","post-checkout":"checkout","pre-push":"push-attempt","post-merge":"merge","post-rewrite":"rewrite","reference-transaction":"reference-transaction"}
    for name in ("post-commit","post-checkout","pre-push","post-merge","post-rewrite","reference-transaction"):
        body=f'#!/bin/sh\n# hugit fake managed hook\n# hugit-hook-version: 1\nHUGIT_BIN="${{HUGIT_BIN:-hugit}}"\n: "$HUGIT_BIN capture --kind {kinds[name]}"\n'
        if name=="post-commit": body += 'OID=$(git rev-parse HEAD)\nBRANCH=$(git symbolic-ref --quiet --short HEAD)\n( "$HUGIT_BIN" capture --kind commit --top-level "$(git rev-parse --show-toplevel)" --log "$(git rev-parse --git-dir)/hugit/event-log.json" --oid "$OID" --branch "$BRANCH" )\n'
        p=hooks/name; p.write_text(body); p.chmod(0o755)
    emit({"hooks_installed":list(("post-commit","post-checkout","pre-push","post-merge","post-rewrite","reference-transaction"))})
if args[:1] == ["capture"]:
    repo=pathlib.Path(arg("--top-level")); oid=arg("--oid"); branch=arg("--branch"); changed=subprocess.check_output(["git","diff-tree","--root","--no-commit-id","--name-only","-r",oid],cwd=repo,text=True).splitlines()
    runtime=logpath().parent; (runtime/"receipts").mkdir(parents=True,exist_ok=True)
    append("ref.update",{"target":oid,"branch":branch,"ref":"refs/heads/"+branch,"files":changed,"receipt_id":"receipt-fixed"},["orchestrator:hugit-hook"]); raise SystemExit(0)
if args[:2] == ["intent","new"]:
    p=pathlib.Path(arg("--store")); p.parent.mkdir(parents=True,exist_ok=True); p.write_text(json.dumps({"sidecars":[{"intent_id":arg("--id"),"charter":arg("--charter"),"acceptance":[arg("--acceptance")],"authoritative":False}]})); append("intent.landed",{"intent_id":arg("--id"),"campaign":arg("--campaign"),"charter":arg("--charter")}); emit({"intent_id":arg("--id")})
if args[:2] == ["pr","open"]:
    value={"pr_id":arg("--pr"),"campaign":arg("--campaign"),"intent_ids":[arg("--intent")],"commit_ids":[arg("--commit")]}; append("pr.opened",value); emit(value)
if args[:2] == ["check","run"]:
    ac=pathlib.Path(arg("--ac")); root=pathlib.Path(arg("--root")); tree=hashlib.sha256(b"".join(p.read_bytes() for p in sorted(root.glob("*.txt")))).hexdigest(); key=hashlib.sha256((arg("--def")+arg("--cmd")+tree+arg("--toolchain")).encode()).hexdigest(); cache=json.loads(ac.read_text()) if ac.exists() else {}; hit=key in cache
    if not hit: subprocess.run(arg("--cmd"),shell=True,check=True); cache[key]=1; ac.write_text(json.dumps(cache))
    append("check.recorded",{"memo_key":key,"cache_hit":hit}); emit({"cache_hit":hit,"local_executions":0 if hit else 1,"memo_key":key})
if args[:2] == ["verdict","record"]:
    append("verdict.recorded",{"intent":arg("--intent"),"verdict":"approve","claims_checked":[arg("--lens")+":"+arg("--result")]}); emit({"verdict_recorded":True})
if args[:2] == ["ctx","usage"]:
    t={k:int(arg("--"+k.replace("_","-"))) for k in ("input","output","cache_read","cache_write")}; t["total"]=sum(t.values()); append("ctx.usage",{"target_id":arg("--intent"),"target_kind":"intent","source":"provider_usage","model":arg("--model"),"tokens":t}); emit({"kind":"ctx.usage","tokens":t})
if args[:2] == ["pr","queue"]: append("pr.queued",{"pr_id":arg("--pr")}); emit({"queued":True})
if args[:2] == ["pr","land"]:
    rows=records(); opened=json.loads(next(r["payload"] for r in rows if r["kind"]=="pr.opened")); usage=json.loads(next(r["payload"] for r in rows if r["kind"]=="ctx.usage")); t=usage["tokens"]; cost=t["input"]*5+t["output"]*25+t["cache_read"]*0.5+t["cache_write"]*6.25
    zero={k:0 for k in t}; append("pr.landed",{"pr_id":arg("--pr"),"campaign":"evidence"}); append("intent.envelope",{"intent_id":opened["intent_ids"][0],"metrics":{"tokens":zero,"cost_usd_micros":0}}); append("pr.envelope",{"intent_id":arg("--pr"),"snapshot":{"env_manifest":"price_card=pc-2026-07"},"metrics":{"tokens":t,"cost_usd_micros":int(cost)}}); emit({"landed":True})
if args[:2] == ["campaign","open"]: append("campaign.opened",{"campaign":arg("--campaign"),"charter":arg("--charter"),"owner":arg("--owner")}); emit({"opened":True,"campaign":arg("--campaign"),"charter":arg("--charter"),"owner":arg("--owner")})
if args[:1] == ["ledger"]:
    rows=records(); emit({"campaigns":[{"campaign":"evidence","asked":1,"done":1,"proven":1,"rejected":0}],"entries":[{"intent_id":"intent-evidence"}]})
if args[:1] == ["watch"]:
    rows=records(); classes={"ref.update":"git-activity","intent.landed":"landing","verdict.recorded":"verdict"}; redacted={"ref.update","pr.opened","check.recorded","verdict.recorded"}; emit({"count":len(rows),"lines":[{"seq":r["seq"],"class":classes.get(r["kind"],"other"),"text":f"[seq={r['seq']} kind={r['kind']} at={r['recorded_at']}] "+("[REDACTED]" if r["kind"] in redacted else r["payload"])} for r in rows]})
if args[:1] == ["export"]:
    out=pathlib.Path(arg("--out")); work=out/"repo.work"; work.mkdir(parents=True); subprocess.run(["git","init","-q",str(work)],check=True); subprocess.run(["git","-C",str(work),"config","user.name","fixture"],check=True); subprocess.run(["git","-C",str(work),"config","user.email","fixture@example.invalid"],check=True); (work/"REFS").write_text(""); subprocess.run(["git","-C",str(work),"add","REFS"],check=True); subprocess.run(["git","-C",str(work),"commit","-q","-m","hugit export: synthetic snapshot"],check=True); subprocess.run(["git","clone","-q","--bare","--no-hardlinks",str(work),str(out/"repo.git")],check=True); subprocess.run(["git","--git-dir",str(out/"repo.git"),"branch","-m","master","main"],check=True); event={"kind":"campaign.opened","payload":json.dumps({"campaign":"export-evidence","charter":"export evidence","owner":"user:evidence"},sort_keys=True,separators=(",",":"))}; (out/"export.json").write_text(json.dumps({"events":[event]})); (out/"redaction-manifest.json").write_text(json.dumps({"removals":[]})); emit({"exported":{"schema_version":"1.0.0","git_dir":str(out/"repo.git"),"envelope_json":str(out/"export.json"),"redaction_manifest":str(out/"redaction-manifest.json")}})
emit({"error":{"kind":"unknown","fix":"fix fixture"}},2)
PY
chmod +x "$FAKE"

BAG1="$TMP/bag1"
python3 "$ROOT/scripts/evidence_report.py" --hugit-bin "$FAKE" --output "$BAG1" --run-id fixture --work-dir "$TMP/stable-work"
python3 "$ROOT/scripts/evidence_report.py" --verify "$BAG1"
python3 "$ROOT/scripts/verify-evidence-report.py" "$BAG1.tar"
python3 - "$BAG1" <<'PY'
import json, pathlib, sys, tarfile
b=pathlib.Path(sys.argv[1])
for name in ("source.json","binary.json","environment.json"):
    assert json.loads((b/"data"/name).read_text())["schema_version"] == "1.0"
claims=json.loads((b/"data/claims.json").read_text())
assert set(claims) == {"schema_version","claims"} and claims["schema_version"] == "1.0"
assert all(set(v) == {"ledger_command","expected_taxonomy","oracle"} for v in claims["claims"].values())
report=json.loads((b/"data/report.json").read_text())
assert set(report["summary"]) == {"conformant","nonconformant","incomplete","expected_refusal","simulator","unsupported","not_applicable"}
assert report["summary"]["not_applicable"] == 1
assert (b/"data/export.tar").is_file()
with tarfile.open(b/"data/repository.tar","r") as archive:
    names=[m.name for m in archive.getmembers()]
assert "repository" not in names and not any(n.startswith("repository/") for n in names)
assert ".git/hugit/event-log.json" in names and ".git/hugit/check-sentinel.txt" in names
with tarfile.open(b/"data/export.tar","r") as archive:
    export_names={m.name for m in archive.getmembers()}
assert "repo.git/config" in export_names and ("export.json" in export_names or "envelope.json" in export_names)
PY
test -f "$BAG1.tar"
python3 - "$ROOT/scripts/evidence_report.py" "$BAG1" "$TMP" <<'PY'
import importlib.util, pathlib, sys
spec=importlib.util.spec_from_file_location("evidence_report",sys.argv[1]); module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
bag=pathlib.Path(sys.argv[2]); root=pathlib.Path(sys.argv[3])
first=module.archive_bag(bag).read_bytes(); second=module.archive_bag(bag).read_bytes()
if first != second: raise SystemExit("deterministic evidence archive failed")
PY

if python3 "$ROOT/scripts/evidence_report.py" --hugit-bin "$FAKE" --output "$TMP/empty" --empty-selected 2>"$TMP/empty.stderr"; then
  printf 'empty selected set unexpectedly passed\n' >&2; exit 1
fi
grep -q 'selected claim set must not be empty' "$TMP/empty.stderr"

FAKE_ZERO="$TMP/fake-zero"
printf '#!/bin/sh\nexit 0\n' >"$FAKE_ZERO"
chmod +x "$FAKE_ZERO"
if python3 "$ROOT/scripts/evidence_report.py" --hugit-bin "$FAKE_ZERO" --output "$TMP/zero" 2>"$TMP/zero.stderr"; then
  printf 'exit-zero no-state fixture unexpectedly passed\n' >&2; exit 1
fi

python3 - "$ROOT/scripts/evidence_report.py" "$BAG1/data/repository.tar" "$TMP" <<'PY'
import importlib.util, pathlib, sys, tarfile
spec=importlib.util.spec_from_file_location("evidence_report",sys.argv[1]); module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
root=pathlib.Path(sys.argv[3]); extracted=root/"extracted"
with tarfile.open(sys.argv[2],"r") as archive: archive.extractall(extracted)
module.safe_repository_tar(extracted,root/"repacked-a.tar")
module.safe_repository_tar(extracted,root/"repacked-b.tar")
if (root/"repacked-a.tar").read_bytes() != (root/"repacked-b.tar").read_bytes(): raise SystemExit("deterministic packaging failed")
PY

# Mutation probe: break one semantic oracle result, refresh BagIt manifests,
# prove verifier rejects semantic closure, restore, then prove green again.
cp "$BAG1/data/report.json" "$TMP/report.json"
python3 - "$BAG1/data/report.json" <<'PY'
import json, pathlib, sys
p=pathlib.Path(sys.argv[1]); v=json.loads(p.read_text()); v["assertions"][0]["status"]="nonconformant"; p.write_text(json.dumps(v,indent=2,sort_keys=True)+"\n")
PY
python3 - "$BAG1" <<'PY'
import hashlib, pathlib, sys
b=pathlib.Path(sys.argv[1]); files=sorted((p for p in (b/"data").rglob("*") if p.is_file()),key=lambda p:p.relative_to(b).as_posix().encode()); (b/"manifest-sha256.txt").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(b).as_posix()}\n" for p in files)); tags=sorted(["bagit.txt","bag-info.txt","README.md","manifest-sha256.txt"],key=lambda x:x.encode()); (b/"tagmanifest-sha256.txt").write_text("".join(f"{hashlib.sha256((b/n).read_bytes()).hexdigest()}  {n}\n" for n in tags))
PY
if python3 "$ROOT/scripts/evidence_report.py" --verify "$BAG1" 2>"$TMP/mutation.stderr"; then
  printf 'package-closure mutation unexpectedly passed\n' >&2; exit 1
fi
grep -q 'claim status does not close' "$TMP/mutation.stderr"
cp "$TMP/report.json" "$BAG1/data/report.json"
python3 - "$BAG1" <<'PY'
import hashlib, pathlib, sys
b=pathlib.Path(sys.argv[1]); files=sorted((p for p in (b/"data").rglob("*") if p.is_file()),key=lambda p:p.relative_to(b).as_posix().encode()); (b/"manifest-sha256.txt").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(b).as_posix()}\n" for p in files)); tags=sorted(["bagit.txt","bag-info.txt","README.md","manifest-sha256.txt"],key=lambda x:x.encode()); (b/"tagmanifest-sha256.txt").write_text("".join(f"{hashlib.sha256((b/n).read_bytes()).hexdigest()}  {n}\n" for n in tags))
PY
python3 "$ROOT/scripts/evidence_report.py" --verify "$BAG1"
printf 'PASS: evidence report tests; package-closure mutation red then restored green\n'
