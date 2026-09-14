#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
VERIFIER="$SCRIPT_DIR/verify-evidence-report.py"

python3 - "$VERIFIER" <<'PY'
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile

verifier = Path(sys.argv[1])
statuses = ("conformant", "nonconformant", "incomplete", "expected_refusal", "simulator", "unsupported", "not_applicable")
claims_order = (
    "C-SETUP-REPO", "C-CAPTURE-COMMIT", "C-CAPTURE-RECEIPT-ID", "C-COMMIT-INTENT-BIND",
    "C-INTENT-NEW", "C-PR-OPEN", "C-CHECK-RUN", "C-VERDICT-RECORD", "C-CTX-USAGE",
    "C-CTX-USAGE-PRICE", "C-PR-LAND", "C-LEDGER", "C-WATCH", "C-EXPORT-GIT",
    "C-SCOPE-RUNNER", "C-SCOPE-WS", "C-SCOPE-DISPATCH",
)
claim_commands = {
    "C-SETUP-REPO":"hugit setup --repo <repo>", "C-CAPTURE-COMMIT":"git commit (post-commit hook)",
    "C-CAPTURE-RECEIPT-ID":"git commit (post-commit hook)", "C-COMMIT-INTENT-BIND":"hugit pr open --commit <oid> --intent <id>",
    "C-INTENT-NEW":"hugit intent new", "C-PR-OPEN":"hugit pr open", "C-CHECK-RUN":"hugit check run (miss, hit, changed-key miss)",
    "C-VERDICT-RECORD":"hugit verdict record", "C-CTX-USAGE":"hugit ctx usage", "C-CTX-USAGE-PRICE":"hugit pr land (frozen price-card join)",
    "C-PR-LAND":"hugit pr queue; hugit pr land", "C-LEDGER":"hugit ledger", "C-WATCH":"hugit watch", "C-EXPORT-GIT":"hugit export",
    "C-SCOPE-RUNNER":"legacy hugit pr land --dispatch (intentionally unexecuted)", "C-SCOPE-WS":"hugit ws", "C-SCOPE-DISPATCH":"hugit dispatch",
}
claim_oracles = {
    "C-SETUP-REPO":"installed_hook_bytes_and_modes", "C-CAPTURE-COMMIT":"git_oid_ref_path_and_hook_principal",
    "C-CAPTURE-RECEIPT-ID":"canonical_ref_update_receipt_and_drain_quiescence", "C-COMMIT-INTENT-BIND":"separate_commit_and_intent_arrays",
    "C-INTENT-NEW":"intent_sidecar_and_landed_event", "C-PR-OPEN":"pr_opened_event", "C-CHECK-RUN":"external_sentinel_and_memo_axes",
    "C-VERDICT-RECORD":"caller_supplied_verdict_event", "C-CTX-USAGE":"caller_supplied_usage_event", "C-CTX-USAGE-PRICE":"exact_frozen_card_envelope_cost",
    "C-PR-LAND":"landed_event_and_envelopes", "C-LEDGER":"ledger_projection_matches_raw_events", "C-WATCH":"watch_projection_contains_raw_sequences",
    "C-EXPORT-GIT":"stock_git_fsck_refs_and_files", "C-SCOPE-RUNNER":"discontinued_source_boundary",
    "C-SCOPE-WS":"structured_ws_refusal_no_mutation", "C-SCOPE-DISPATCH":"structured_dispatch_refusal_no_mutation",
}
command_prefixes = {
    "binary-version":("hugit","--version"), "boundary-dispatch":("hugit","dispatch"), "boundary-ws":("hugit","ws"),
    "check-changed":("hugit","check","run"), "check-cold":("hugit","check","run"), "check-warm":("hugit","check","run"),
    "ctx-usage":("hugit","ctx","usage"), "export-log-seed":("hugit","campaign","open"), "export":("hugit","export"),
    "intent-new":("hugit","intent","new"), "ledger":("hugit","ledger"), "pr-land":("hugit","pr","land"),
    "pr-open":("hugit","pr","open"), "pr-queue":("hugit","pr","queue"), "setup":("hugit","setup"),
    "verdict-record":("hugit","verdict","record"), "watch":("hugit","watch"), "git-add-seed":("git","add"),
    "git-add":("git","add"), "git-branch":("git","branch"), "git-commit-seed":("git","commit"), "git-commit":("git","commit"),
    "git-config-email":("git","config"), "git-config-name":("git","config"), "git-head":("git","rev-parse"),
    "git-init":("git","init"), "oracle-changed-paths":("git","diff-tree"), "oracle-export-fsck":("git",), "oracle-export-refs":("git",),
}


def run(command, cwd, env=None, input_bytes=None):
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(command, cwd=cwd, env=merged, input=input_bytes, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def lp(text):
    raw = text.encode()
    return struct.pack(">I", len(raw)) + raw


def reseal(events):
    previous = "0" * 64
    for seq, event in enumerate(events):
        event["seq"] = seq
        event["prev_hash"] = previous
        payload = event["payload"]
        principals = event["principal_chain"]
        preimage = lp(previous) + lp(event["kind"]) + struct.pack(">I", len(principals))
        preimage += b"".join(lp(item) for item in principals) + lp(payload) + struct.pack(">Q", seq)
        event["this_hash"] = hashlib.sha256(preimage).hexdigest()
        previous = event["this_hash"]


def add_event(events, kind, payload, principals=None):
    events.append({
        "seq": 0, "prev_hash": "", "this_hash": "", "kind": kind,
        "principal_chain": principals or ["orchestrator:hugit"],
        "payload": canonical(payload), "recorded_at": 1700000000000 + len(events),
    })


def archive_tree(root, output):
    paths = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs.sort()
        files.sort()
        relative = Path(directory).relative_to(root)
        for name in dirs + files:
            path = Path(directory) / name
            paths.append(((relative / name).as_posix(), path))
    paths.sort(key=lambda item: item[0].encode())
    with tarfile.open(output, "w", format=tarfile.PAX_FORMAT) as archive:
        for name, path in paths:
            info = tarfile.TarInfo(name)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            if path.is_dir():
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                archive.addfile(info)
            else:
                data = path.read_bytes()
                info.type = tarfile.REGTYPE
                info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))


def add_command(bag, ident, stdout, exit_code=0, state=None, argv=None):
    base = bag / "data/commands" / ident
    base.with_suffix(".stdout").write_text(json.dumps(stdout, sort_keys=True) + "\n")
    base.with_suffix(".stderr").write_bytes(b"")
    meta = {
        "schema_version": "1.0", "id": ident, "argv": argv or list(command_prefixes[ident]), "cwd": ".",
        "env": {"GIT_AUTHOR_DATE": "2026-09-13T00:00:00Z", "LC_ALL": "C", "TZ": "UTC"}, "started_at": "2026-09-13T00:00:00Z",
        "duration_ns": 1, "attempt_count": 1, "retry_attempts": [], "exit_code": exit_code,
        "stdout_path": f"data/commands/{ident}.stdout", "stderr_path": f"data/commands/{ident}.stderr",
    }
    if state:
        meta.update(state)
    write_json(base.with_suffix(".json"), meta)


def rebag(bag):
    payload = sorted((path for path in (bag / "data").rglob("*") if path.is_file()), key=lambda path: path.relative_to(bag).as_posix().encode())
    (bag / "manifest-sha256.txt").write_text("".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(bag).as_posix()}\n" for path in payload))
    tags = sorted(("bagit.txt", "bag-info.txt", "README.md", "manifest-sha256.txt"), key=lambda name: name.encode())
    (bag / "tagmanifest-sha256.txt").write_text("".join(f"{hashlib.sha256((bag / name).read_bytes()).hexdigest()}  {name}\n" for name in tags))


def build_runner_fixture(base):
    bag = base / "bag"
    (bag / "data/commands").mkdir(parents=True)
    (bag / "data/assertions").mkdir(parents=True)
    repo = base / "repository"
    repo.mkdir()
    run(["git", "init", "-q", "-b", "main"], repo)
    run(["git", "config", "user.name", "Evidence Fixture"], repo)
    run(["git", "config", "user.email", "fixture@example.invalid"], repo)
    env = {"GIT_AUTHOR_DATE": "2023-11-14T22:13:20Z", "GIT_COMMITTER_DATE": "2023-11-14T22:13:20Z"}
    (repo / "README.fixture").write_text("seed\n")
    run(["git", "add", "README.fixture"], repo, env)
    run(["git", "commit", "-q", "-m", "seed"], repo, env)
    (repo / "journey.txt").write_text("hook-to-land\n")
    run(["git", "add", "journey.txt"], repo, env)
    run(["git", "commit", "-q", "-m", "journey"], repo, env)
    head = run(["git", "rev-parse", "HEAD"], repo).stdout.decode().strip()

    hook_kinds = {"post-commit":"commit", "post-checkout":"checkout", "pre-push":"push-attempt", "post-merge":"merge", "post-rewrite":"rewrite", "reference-transaction":"reference-transaction"}
    for name, kind in hook_kinds.items():
        hook = repo / ".git/hooks" / name
        body = f'#!/bin/sh\n# hugit managed hook\n# hugit-hook-version: 1\nHUGIT_BIN="${{HUGIT_BIN:-hugit}}"\n: "$HUGIT_BIN capture --kind {kind}"\n'
        if name == "post-commit":
            body += 'OID=$(git rev-parse HEAD)\nBRANCH=$(git symbolic-ref --quiet --short HEAD)\n( "$HUGIT_BIN" capture --kind commit --oid "$OID" --branch "$BRANCH" ) &\n'
        hook.write_text(body + "exit 0\n")
        hook.chmod(0o755)
    runtime = repo / ".git/hugit"
    (runtime / "receipts").mkdir(parents=True)
    (runtime / "check-sentinel.txt").write_text("executed\nexecuted\n")
    (repo / ".hugit").mkdir()
    write_json(repo / ".hugit/intents.json", {"sidecars": [{"intent_id": "intent-evidence", "charter": "prove local journey", "acceptance": ["all semantic oracles pass"], "authoritative": False}]})

    tokens = {"input": 1_500_000, "output": 200_000, "cache_read": 4_000_000, "cache_write": 1_000_000, "total": 6_700_000}
    events = []
    add_event(events, "ref.update", {"target": head, "branch": "main", "ref": "refs/heads/main", "files": ["journey.txt"], "receipt_id": "receipt-fixed"}, ["orchestrator:hugit-hook"])
    add_event(events, "intent.landed", {"intent_id": "intent-evidence", "campaign": "evidence", "charter": "prove local journey"})
    add_event(events, "pr.opened", {"pr_id": "PR-EVIDENCE", "campaign": "evidence", "intent_ids": ["intent-evidence"], "commit_ids": [head]})
    add_event(events, "check.recorded", {"memo_key": "a" * 64, "cache_hit": False})
    add_event(events, "check.recorded", {"memo_key": "a" * 64, "cache_hit": True})
    add_event(events, "check.recorded", {"memo_key": "b" * 64, "cache_hit": False})
    add_event(events, "verdict.recorded", {"intent": "intent-evidence", "verdict": "approve", "claims_checked": ["security:approve"]})
    add_event(events, "ctx.usage", {"target_id": "intent-evidence", "target_kind": "intent", "source": "provider_usage", "model": "claude-opus-4-8", "tokens": tokens})
    add_event(events, "pr.queued", {"pr_id": "PR-EVIDENCE"})
    add_event(events, "pr.landed", {"pr_id": "PR-EVIDENCE", "campaign": "evidence"})
    zero_tokens = {key: 0 for key in tokens}
    add_event(events, "intent.envelope", {"intent_id": "intent-evidence", "metrics": {"tokens": zero_tokens, "cost_usd_micros": 0}})
    add_event(events, "pr.envelope", {"intent_id": "PR-EVIDENCE", "snapshot": {"env_manifest": "price_card=pc-2026-07"}, "metrics": {"tokens": tokens, "cost_usd_micros": 20_750_000}})
    reseal(events)
    write_json(runtime / "event-log.json", events)

    add_command(bag, "check-cold", {"cache_hit": False, "local_executions": 1, "memo_key": "a" * 64})
    add_command(bag, "check-warm", {"cache_hit": True, "local_executions": 0, "memo_key": "a" * 64})
    add_command(bag, "check-changed", {"cache_hit": False, "local_executions": 1, "memo_key": "b" * 64})
    add_command(bag, "ledger", {"campaigns": [{"campaign": "evidence", "asked": 1, "done": 1, "proven": 1}], "entries": [{"intent_id": "intent-evidence"}]})
    classes = {"ref.update":"git-activity", "intent.landed":"landing", "verdict.recorded":"verdict"}
    redacted = {"ref.update", "pr.opened", "check.recorded", "verdict.recorded"}
    watch_lines = [{"seq": event["seq"], "class": classes.get(event["kind"], "other"), "text": f"[seq={event['seq']} kind={event['kind']} at={event['recorded_at']}] " + ("[REDACTED]" if event["kind"] in redacted else event["payload"])} for event in events]
    add_command(bag, "watch", {"count": len(events), "lines": watch_lines})
    state = "c" * 64
    state_meta = {"state_sha256_before": state, "state_sha256_after": state, "state_unchanged": True}
    refusal = {"error": {"kind": "unknown_command", "fix": "use CLI-local verbs"}}
    add_command(bag, "boundary-ws", refusal, 2, state_meta)
    add_command(bag, "boundary-dispatch", refusal, 2, state_meta)
    add_command(bag, "intent-new", {}, argv=["hugit","intent","new","--id","intent-evidence","--campaign","evidence","--charter","prove local journey","--acceptance","all semantic oracles pass"])
    retry_base = bag / "data/commands/retries/intent-new-01"
    retry_base.parent.mkdir(parents=True)
    retry_base.with_suffix(".stdout").write_text(json.dumps({"error":{"kind":"log_busy"}}) + "\n")
    retry_base.with_suffix(".stderr").write_bytes(b"")
    intent_meta_path = bag / "data/commands/intent-new.json"
    intent_meta = json.loads(intent_meta_path.read_text())
    intent_meta["attempt_count"] = 2
    intent_meta["retry_attempts"] = [{"attempt":1,"duration_ns":1,"exit_code":2,"stdout_path":"data/commands/retries/intent-new-01.stdout","stderr_path":"data/commands/retries/intent-new-01.stderr"}]
    write_json(intent_meta_path, intent_meta)
    add_command(bag, "pr-open", {}, argv=["hugit","pr","open","--pr","PR-EVIDENCE","--intent","intent-evidence","--commit",head])
    add_command(bag, "verdict-record", {}, argv=["hugit","verdict","record","--intent","intent-evidence","--lens","security","--result","approve"])
    add_command(bag, "ctx-usage", {}, argv=["hugit","ctx","usage","--intent","intent-evidence","--model","claude-opus-4-8","--input","1500000","--output","200000","--cache-read","4000000","--cache-write","1000000"])
    add_command(bag, "pr-queue", {}, argv=["hugit","pr","queue","--pr","PR-EVIDENCE"])
    add_command(bag, "pr-land", {}, argv=["hugit","pr","land","--pr","PR-EVIDENCE"])
    add_command(bag, "export-log-seed", {"campaign":"export-evidence", "charter":"export evidence", "owner":"user:evidence"})
    add_command(bag, "export", {"exported":{"schema_version":"1.0.0", "envelope_json":"/tmp/export/export.json", "redaction_manifest":"/tmp/export/redaction-manifest.json", "git_dir":"/tmp/export/repo.git"}})
    for ident in command_prefixes:
        if not (bag / f"data/commands/{ident}.json").exists():
            add_command(bag, ident, {})
    (bag / "data/commands/binary-version.stdout").write_text("hugit-fixture 1.0\n")
    archive_tree(repo, bag / "data/repository.tar")

    export = base / "export"
    bare = export / "repo.git"
    export.mkdir()
    export_work = base / "export-work"
    export_work.mkdir()
    run(["git", "init", "-q", "-b", "main"], export_work)
    run(["git", "config", "user.name", "Evidence Fixture"], export_work)
    run(["git", "config", "user.email", "fixture@example.invalid"], export_work)
    (export_work / "REFS").write_bytes(b"")
    run(["git", "add", "REFS"], export_work, env)
    run(["git", "commit", "-q", "-m", "hugit export: synthetic snapshot"], export_work, env)
    run(["git", "clone", "-q", "--bare", "--no-hardlinks", str(export_work), str(bare)], base)
    export_head = run(["git", "rev-parse", "HEAD"], export_work).stdout.decode().strip()
    (bare / "packed-refs").unlink(missing_ok=True)
    run(["git", f"--git-dir={bare}", "update-ref", "refs/heads/main", export_head], base)
    write_json(export / "export.json", {"schema_version": "1.0", "events": [{"kind":"campaign.opened", "payload": canonical({"campaign":"export-evidence", "charter":"export evidence", "owner":"user:evidence"})}]})
    write_json(export / "redaction-manifest.json", {"removals": []})
    archive_tree(export, bag / "data/export.tar")

    (bag / "bagit.txt").write_text("BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n")
    (bag / "bag-info.txt").write_text("Bagging-Date: 2026-09-13\n")
    (bag / "README.md").write_text("Runner-format fixture.\n")
    (bag / "data/methodology.md").write_text("Independent compact journey.\n")
    limitations_text = "# Limitations\n\nSingle synthetic journey. Usage and verdict are caller_supplied. Provider authenticity is not established. Hash chain is unkeyed. Export uses a separate clean CLI-created canonical corpus because current export refuses hook receipt payload as sensitive; primary journey is not exported. Network absence is not instrumented. Performance is not measured. Legacy `pr land --dispatch` remains source-reachable but is intentionally unexecuted because runner execution is discontinued. Source-to-binary binding and cryptographic execution causality are not established.\n"
    (bag / "data/limitations.md").write_text(limitations_text)
    write_json(bag / "data/limitations.json", {"schema_version":"1.0", "caller_supplied_verdict_usage":True, "provider_authenticity_established":False, "event_chain_keyed":False, "primary_journey_exported":False, "network_absence_instrumented":False, "performance_measured":False, "runner_execution":"discontinued_unexecuted", "source_to_binary_binding_established":False, "cryptographic_execution_causality_established":False})
    retained_binary = bag / "data/hugit"
    retained_binary.write_text("fixture binary bytes\n")
    retained_binary.chmod(0o755)
    binary_hash = hashlib.sha256(retained_binary.read_bytes()).hexdigest()
    write_json(bag / "data/source.json", {"schema_version": "1.0", "ledger_baseline": "fixture", "source_to_binary_binding": "not_established"})
    write_json(bag / "data/binary.json", {"schema_version": "1.0", "selected_path": "hugit", "retained_path": "data/hugit", "sha256": binary_hash, "version_stdout": "hugit-fixture 1.0", "source_provenance": "unbound-selected-executable"})
    write_json(bag / "data/environment.json", {"schema_version": "1.0", "allowlist": {"LC_ALL": "C"}})

    expected = {claim: "conformant" for claim in claims_order}
    expected["C-SCOPE-RUNNER"] = "not_applicable"
    expected["C-SCOPE-WS"] = expected["C-SCOPE-DISPATCH"] = "expected_refusal"
    oracle = claim_oracles
    claims = {claim: {"ledger_command": claim_commands[claim], "expected_taxonomy": expected[claim], "oracle": oracle[claim]} for claim in claims_order}
    write_json(bag / "data/claims.json", {"schema_version": "1.0", "claims": claims})
    references = []
    for claim in claims_order:
        detail = "Independent package-byte oracle passed."
        if claim == "C-VERDICT-RECORD":
            detail = "caller-supplied verdict; no model authenticity claimed."
        elif claim == "C-CTX-USAGE":
            detail = "caller-supplied usage; provider authenticity not claimed."
        elif claim == "C-CTX-USAGE-PRICE":
            detail = "caller-supplied counters priced exactly; provider authenticity not claimed."
        elif claim == "C-SCOPE-RUNNER":
            detail = "Legacy runner source-reachable; intentionally unexecuted and not inferred from dispatch refusal."
        assertion = {
            "schema_version": "1.0", "claim_id": claim, "status": expected[claim], "oracle": oracle[claim],
            "detail": detail, "evidence": ["data/repository.tar"],
            "evidence_sha256": {"data/repository.tar": hashlib.sha256((bag / "data/repository.tar").read_bytes()).hexdigest()},
        }
        path = bag / "data/assertions" / f"{claim}.json"
        write_json(path, assertion)
        references.append(f"data/assertions/{claim}.json")
    summary = {status: 0 for status in statuses}
    for value in expected.values():
        summary[value] += 1
    write_json(bag / "data/report.json", {
        "schema_version": "1.0", "suite": "hugit-cli-local-journey", "run_id": "runner-fixture",
        "generated_at": "2026-09-13T00:00:00Z", "selected_claims": list(claims_order),
        "assertions": references, "summary": summary, "overall_status": "pass",
    })
    rebag(bag)
    return bag


def rewrite_tar(path, transform, addition=None):
    replacement = path.with_suffix(".new.tar")
    with tarfile.open(path, "r") as source, tarfile.open(replacement, "w", format=tarfile.PAX_FORMAT) as target:
        for member in source.getmembers():
            data = source.extractfile(member).read() if member.isfile() else None
            member, data = transform(member, data)
            target.addfile(member, io.BytesIO(data) if data is not None else None)
        if addition:
            target.addfile(*addition)
    replacement.replace(path)


def mutate_event_tar(path, mutate):
    def transform(member, data):
        if member.name == ".git/hugit/event-log.json":
            events = json.loads(data)
            mutate(events)
            reseal(events)
            data = (json.dumps(events, sort_keys=True, indent=2) + "\n").encode()
            member.size = len(data)
        return member, data
    rewrite_tar(path, transform)


def mutate_fixture(bag, mutation):
    if mutation == "payload_flip":
        (bag / "data/methodology.md").write_text("flipped\n")
        return
    if mutation == "empty_limitations":
        (bag / "data/limitations.md").write_bytes(b"")
        rebag(bag)
        return
    if mutation == "inverted_limitations":
        path = bag / "data/limitations.md"
        path.write_text(path.read_text().replace("Provider authenticity is not established.", "Provider authenticity is established."))
        rebag(bag)
        return
    if mutation == "remove_payload":
        (bag / "data/limitations.md").unlink()
        return
    if mutation == "unmanifested":
        (bag / "data/extra.txt").write_text("extra\n")
        return
    if mutation == "traversal_manifest":
        lines = (bag / "manifest-sha256.txt").read_text().splitlines()
        lines[0] = lines[0].split("  ", 1)[0] + "  data/../escape"
        (bag / "manifest-sha256.txt").write_text("\n".join(lines) + "\n")
        tags = sorted(("bagit.txt", "bag-info.txt", "README.md", "manifest-sha256.txt"))
        (bag / "tagmanifest-sha256.txt").write_text("".join(f"{hashlib.sha256((bag / name).read_bytes()).hexdigest()}  {name}\n" for name in tags))
        return
    if mutation == "duplicate_claim":
        report = json.loads((bag / "data/report.json").read_text())
        report["selected_claims"].append(report["selected_claims"][0])
        write_json(bag / "data/report.json", report)
    elif mutation == "missing_assertion":
        report = json.loads((bag / "data/report.json").read_text())
        reference = report["assertions"].pop()
        (bag / reference).unlink()
        write_json(bag / "data/report.json", report)
    elif mutation == "evidence_hash":
        path = bag / "data/assertions/C-SETUP-REPO.json"
        assertion = json.loads(path.read_text())
        assertion["evidence_sha256"]["data/repository.tar"] = "f" * 64
        write_json(path, assertion)
    elif mutation == "hook_mode":
        def hook_mode(member, data):
            if member.name == ".git/hooks/post-commit": member.mode = 0o644
            return member, data
        rewrite_tar(bag / "data/repository.tar", hook_mode)
    elif mutation == "tar_mtime":
        def mtime(member, data):
            if member.name == ".git/HEAD": member.mtime = 1
            return member, data
        rewrite_tar(bag / "data/repository.tar", mtime)
    elif mutation == "command_env":
        path = bag / "data/commands/check-cold.json"; value = json.loads(path.read_text()); value["env"]["API_TOKEN"] = "secret"; write_json(path, value)
    elif mutation == "command_basename":
        path = bag / "data/commands/check-cold.json"; value = json.loads(path.read_text()); value["stdout_path"] = "data/commands/check-warm.stdout"; write_json(path, value)
    elif mutation == "command_argv":
        path = bag / "data/commands/setup.json"; value = json.loads(path.read_text()); value["argv"] = ["/usr/bin/false"]; write_json(path, value)
    elif mutation == "command_attempt_count":
        path = bag / "data/commands/setup.json"; value = json.loads(path.read_text()); value["attempt_count"] = 0; write_json(path, value)
    elif mutation == "command_retry_kind":
        path = bag / "data/commands/retries/intent-new-01.stdout"; write_json(path, {"error":{"kind":"io"}})
    elif mutation == "claim_contract":
        path = bag / "data/claims.json"; value = json.loads(path.read_text()); value["claims"]["C-SETUP-REPO"]["oracle"] = "invented"; write_json(path, value)
    elif mutation == "repo_corruption":
        changed = [False]
        def corrupt(member, data):
            if not changed[0] and member.isfile() and member.name.startswith(".git/objects/") and len(member.name.split("/")[-1]) == 38:
                data = data[:-1] + bytes([data[-1] ^ 1]); changed[0] = True
            return member, data
        rewrite_tar(bag / "data/repository.tar", corrupt)
    elif mutation == "unsafe_tar":
        info = tarfile.TarInfo("../escape"); info.uid = info.gid = 0; info.uname = info.gname = ""; info.mtime = 0; info.mode = 0o644; info.size = 1
        rewrite_tar(bag / "data/repository.tar", lambda member, data: (member, data), (info, io.BytesIO(b"x")))
    elif mutation == "semantic_capture":
        def wrong(events):
            payload = json.loads(events[0]["payload"]); payload["branch"] = "wrong"; events[0]["payload"] = canonical(payload)
        mutate_event_tar(bag / "data/repository.tar", wrong)
    elif mutation == "semantic_setup":
        def setup(member, data):
            if member.name == ".git/hooks/post-commit": data = data.replace(b"hugit", b"other"); member.size = len(data)
            return member, data
        rewrite_tar(bag / "data/repository.tar", setup)
    elif mutation == "semantic_noop_hook":
        def noop(member, data):
            if member.name == ".git/hooks/post-commit":
                data = b'#!/bin/sh\n# hugit-hook-version: 1\n# HUGIT_BIN="${HUGIT_BIN:-hugit}"\n# OID=$(git rev-parse HEAD)\n# BRANCH=$(git symbolic-ref --quiet --short HEAD)\n# ( "$HUGIT_BIN" capture --kind commit ) &\nexit 0\n'; member.size = len(data)
            return member, data
        rewrite_tar(bag / "data/repository.tar", noop)
    elif mutation == "semantic_early_exit_hook":
        def early_exit(member, data):
            if member.name == ".git/hooks/post-commit":
                data = data.replace(b'HUGIT_BIN="${HUGIT_BIN:-hugit}"\n', b'HUGIT_BIN="${HUGIT_BIN:-hugit}"\nexit 0\n', 1); member.size = len(data)
            return member, data
        rewrite_tar(bag / "data/repository.tar", early_exit)
    elif mutation == "semantic_receipt":
        def receipt(member, data):
            if member.name == ".git/hugit/event-log.json":
                events = json.loads(data); payload = json.loads(events[0]["payload"]); payload["receipt_id"] = ""; events[0]["payload"] = canonical(payload); reseal(events); data = (json.dumps(events, sort_keys=True, indent=2) + "\n").encode(); member.size = len(data)
            return member, data
        rewrite_tar(bag / "data/repository.tar", receipt)
    elif mutation == "semantic_intent":
        def intent(member, data):
            if member.name == ".hugit/intents.json": data = b'{"intents":[]}\n'; member.size = len(data)
            return member, data
        rewrite_tar(bag / "data/repository.tar", intent)
    elif mutation == "semantic_pr_binding":
        def wrong(events):
            for event in events:
                if event["kind"] == "pr.opened":
                    payload = json.loads(event["payload"]); payload["intent_ids"] = ["wrong"]; event["payload"] = canonical(payload)
        mutate_event_tar(bag / "data/repository.tar", wrong)
    elif mutation == "semantic_check":
        def sentinel(member, data):
            if member.name == ".git/hugit/check-sentinel.txt": data = b"executed\n"; member.size = len(data)
            return member, data
        rewrite_tar(bag / "data/repository.tar", sentinel)
    elif mutation == "semantic_verdict":
        def wrong(events):
            for event in events:
                if event["kind"] == "verdict.recorded":
                    payload = json.loads(event["payload"]); payload["verdict"] = "reject"; event["payload"] = canonical(payload)
        mutate_event_tar(bag / "data/repository.tar", wrong)
    elif mutation == "semantic_usage":
        def wrong(events):
            for event in events:
                if event["kind"] == "ctx.usage":
                    payload = json.loads(event["payload"]); payload["tokens"]["input"] = 1; event["payload"] = canonical(payload)
        mutate_event_tar(bag / "data/repository.tar", wrong)
    elif mutation == "semantic_usage_argv":
        path = bag / "data/commands/ctx-usage.json"; value = json.loads(path.read_text()); value["argv"][value["argv"].index("--input") + 1] = "1"; write_json(path, value)
    elif mutation == "semantic_price":
        def wrong(events):
            for event in events:
                if event["kind"] == "pr.envelope":
                    payload = json.loads(event["payload"]); payload["metrics"]["cost_usd_micros"] = 1; event["payload"] = canonical(payload)
        mutate_event_tar(bag / "data/repository.tar", wrong)
    elif mutation == "semantic_land":
        def wrong(events):
            events[:] = [event for event in events if event["kind"] != "pr.landed"]
        mutate_event_tar(bag / "data/repository.tar", wrong)
        watch = json.loads((bag / "data/commands/watch.stdout").read_text()); watch["count"] -= 1; watch["lines"] = watch["lines"][:-1]; write_json(bag / "data/commands/watch.stdout", watch)
    elif mutation == "semantic_ledger":
        write_json(bag / "data/commands/ledger.stdout", {"campaigns": [{"campaign": "evidence", "asked": 0, "done": 1, "proven": 1}], "entries": []})
    elif mutation == "semantic_watch":
        watch = json.loads((bag / "data/commands/watch.stdout").read_text()); watch["count"] += 1; write_json(bag / "data/commands/watch.stdout", watch)
    elif mutation == "semantic_scope":
        meta = bag / "data/commands/boundary-ws.json"; value = json.loads(meta.read_text()); value["exit_code"] = 0; write_json(meta, value)
    elif mutation == "semantic_runner":
        path = bag / "data/assertions/C-SCOPE-RUNNER.json"; value = json.loads(path.read_text()); value["detail"] = "Runner passed."; write_json(path, value)
    elif mutation == "unknown_claim":
        report = json.loads((bag / "data/report.json").read_text()); claims = json.loads((bag / "data/claims.json").read_text())
        claim = "C-UNKNOWN"; report["selected_claims"].append(claim); report["assertions"].append(f"data/assertions/{claim}.json"); report["summary"]["conformant"] += 1
        claims["claims"][claim] = {"ledger_command": "unknown", "expected_taxonomy": "conformant", "oracle": "unknown"}
        assertion = {"schema_version": "1.0", "claim_id": claim, "status": "conformant", "oracle": "unknown", "detail": "forged", "evidence": ["data/repository.tar"], "evidence_sha256": {"data/repository.tar": hashlib.sha256((bag / "data/repository.tar").read_bytes()).hexdigest()}}
        write_json(bag / f"data/assertions/{claim}.json", assertion); write_json(bag / "data/report.json", report); write_json(bag / "data/claims.json", claims)
    elif mutation == "export_corruption":
        changed = [False]
        def corrupt(member, data):
            if not changed[0] and member.isfile() and member.name.startswith("repo.git/objects/") and len(member.name.split("/")[-1]) == 38:
                data = data[:-1] + bytes([data[-1] ^ 1]); changed[0] = True
            return member, data
        rewrite_tar(bag / "data/export.tar", corrupt)
    elif mutation == "export_substitution":
        def substitute(member, data):
            if member.name == "export.json":
                data = json.dumps({"events":[{"kind":"campaign.opened","payload":canonical({"campaign":"other","charter":"other","owner":"attacker"})}]}).encode(); member.size = len(data)
            return member, data
        rewrite_tar(bag / "data/export.tar", substitute)
    elif mutation == "export_git_substitution":
        with tempfile.TemporaryDirectory(prefix="hugit-export-substitute-") as raw:
            root = Path(raw)
            with tarfile.open(bag / "data/export.tar", "r") as archive:
                archive.extractall(root)
            bare = root / "repo.git"
            blob = run(["git", f"--git-dir={bare}", "hash-object", "-w", "--stdin"], root, input_bytes=b"attacker\n").stdout.decode().strip()
            tree = run(["git", f"--git-dir={bare}", "mktree"], root, input_bytes=f"100644 blob {blob}\tATTACKER\n".encode()).stdout.decode().strip()
            env = {"GIT_AUTHOR_NAME":"attacker", "GIT_AUTHOR_EMAIL":"attacker@example.invalid", "GIT_COMMITTER_NAME":"attacker", "GIT_COMMITTER_EMAIL":"attacker@example.invalid"}
            commit = run(["git", f"--git-dir={bare}", "commit-tree", tree, "-m", "attacker export"], root, env).stdout.decode().strip()
            run(["git", f"--git-dir={bare}", "update-ref", "refs/heads/main", commit], root)
            archive_tree(root, bag / "data/export.tar")
    else:
        raise AssertionError(f"unknown fixture mutation: {mutation}")
    rebag(bag)


def invoke(bag, executable=verifier):
    return subprocess.run([sys.executable, str(executable), str(bag)], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def check_case(base_bag, root, name, mutation, expected_check):
    bag = root / name
    shutil.copytree(base_bag, bag)
    mutate_fixture(bag, mutation)
    result = invoke(bag)
    if result.returncode == 0:
        raise AssertionError(f"{name}: verifier unexpectedly passed")
    output = json.loads(result.stdout)
    failed = {item["name"] for item in output["checks"] if item["status"] == "fail"}
    if expected_check not in failed:
        raise AssertionError(f"{name}: expected {expected_check}, got {sorted(failed)}; stderr={result.stderr}")
    print(f"PASS {name}: exit={result.returncode} named_check={expected_check}")


with tempfile.TemporaryDirectory(prefix="hugit-evidence-suite-") as raw:
    root = Path(raw)
    base_bag = build_runner_fixture(root / "base")
    result = invoke(base_bag)
    if result.returncode != 0:
        raise AssertionError(f"runner-format positive failed: {result.stderr}\n{result.stdout}")
    assert json.loads(result.stdout)["overall_status"] == "valid"
    positive = json.loads(result.stdout)
    assert positive["taxonomy_counts"]["conformant"] == 14
    assert positive["taxonomy_counts"]["expected_refusal"] == 2
    assert positive["taxonomy_counts"]["not_applicable"] == 1
    print("PASS runner_format_positive: exit=0 overall_status=valid claims=17 GIT_AUTHOR_DATE=allowed")
    outer = root / "runner-format.tar"
    archive_tree(base_bag, outer)
    outer_result = invoke(outer)
    if outer_result.returncode != 0 or json.loads(outer_result.stdout).get("overall_status") != "valid":
        raise AssertionError(f"outer archive positive failed: {outer_result.stderr}\n{outer_result.stdout}")
    print("PASS outer_archive_positive: exit=0 overall_status=valid")

    cases = [
        ("payload_byte_flip", "payload_flip", "payload_manifest"),
        ("empty_limitations", "empty_limitations", "semantic_boundary"),
        ("inverted_limitations", "inverted_limitations", "semantic_boundary"),
        ("remove_payload", "remove_payload", "payload_manifest"),
        ("unmanifested_payload", "unmanifested", "payload_manifest"),
        ("traversal_manifest_path", "traversal_manifest", "payload_manifest"),
        ("duplicate_claim", "duplicate_claim", "claim_coverage"),
        ("missing_assertion", "missing_assertion", "claim_coverage"),
        ("assertion_evidence_hash", "evidence_hash", "evidence"),
        ("hook_mode_change", "hook_mode", "repository_archive"),
        ("tar_mtime_change", "tar_mtime", "repository_archive"),
        ("command_env_secret", "command_env", "schemas"),
        ("command_basename_mismatch", "command_basename", "schemas"),
        ("command_argv_forgery", "command_argv", "schemas"),
        ("command_attempt_count", "command_attempt_count", "schemas"),
        ("command_retry_kind", "command_retry_kind", "schemas"),
        ("claim_contract_relabel", "claim_contract", "schemas"),
        ("git_object_corruption", "repo_corruption", "git_fsck"),
        ("unsafe_tar_traversal", "unsafe_tar", "repository_archive"),
        ("forged_conformant_capture", "semantic_capture", "semantic_oracles"),
        ("semantic_setup", "semantic_setup", "semantic_oracles"),
        ("semantic_noop_hook", "semantic_noop_hook", "semantic_oracles"),
        ("semantic_early_exit_hook", "semantic_early_exit_hook", "semantic_oracles"),
        ("semantic_receipt", "semantic_receipt", "semantic_oracles"),
        ("semantic_intent_pr", "semantic_intent", "semantic_oracles"),
        ("semantic_pr_binding", "semantic_pr_binding", "semantic_oracles"),
        ("semantic_check", "semantic_check", "semantic_oracles"),
        ("semantic_verdict_usage", "semantic_verdict", "semantic_oracles"),
        ("semantic_usage", "semantic_usage", "semantic_oracles"),
        ("semantic_usage_argv", "semantic_usage_argv", "semantic_oracles"),
        ("semantic_pricing", "semantic_price", "semantic_oracles"),
        ("semantic_land", "semantic_land", "semantic_oracles"),
        ("semantic_ledger", "semantic_ledger", "semantic_oracles"),
        ("semantic_watch", "semantic_watch", "semantic_oracles"),
        ("semantic_scope_refusal", "semantic_scope", "semantic_oracles"),
        ("semantic_runner_boundary", "semantic_runner", "semantic_oracles"),
        ("unknown_selected_claim", "unknown_claim", "semantic_oracles"),
        ("export_object_corruption", "export_corruption", "export_git_fsck"),
        ("export_valid_substitution", "export_substitution", "export_archive"),
        ("export_valid_git_substitution", "export_git_substitution", "export_git_fsck"),
    ]
    for case in cases:
        check_case(base_bag, root, *case)

    probe_bag = root / "probe"
    shutil.copytree(base_bag, probe_bag)
    mutate_fixture(probe_bag, "payload_flip")
    weakened = root / "weakened-verifier.py"
    source = verifier.read_text()
    needle = 'require(sha256_file(path) == digest, f"payload checksum mismatch: {name}")'
    replacement = 'require(True, f"payload checksum mismatch: {name}")'
    if source.count(needle) != 1:
        raise AssertionError("mutation probe could not locate payload checksum guard")
    weakened.write_text(source.replace(needle, replacement))
    wrong = invoke(probe_bag, weakened)
    if wrong.returncode != 0:
        raise AssertionError(f"weakened verifier did not expose mutation: {wrong.stderr}\n{wrong.stdout}")
    restored = invoke(probe_bag)
    if restored.returncode == 0:
        raise AssertionError("original verifier failed to reject probe fixture")
    print("PASS mutation_probe: weakened_guard=>wrong_green(exit=0), restored_guard=>red(exit=1)")
    print(f"PASS suite: {2 + len(cases)} controls + mutation probe")
PY
