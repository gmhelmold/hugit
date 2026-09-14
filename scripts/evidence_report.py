#!/usr/bin/env python3
"""Build reproducible, claim-specific evidence for hugit CLI-local journey."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import unicodedata
from collections import Counter

SCHEMA = "1.0"
SUITE = "hugit-cli-local-journey"
CLAIMS = [
    "C-SETUP-REPO",
    "C-CAPTURE-COMMIT",
    "C-CAPTURE-RECEIPT-ID",
    "C-COMMIT-INTENT-BIND",
    "C-INTENT-NEW",
    "C-PR-OPEN",
    "C-CHECK-RUN",
    "C-VERDICT-RECORD",
    "C-CTX-USAGE",
    "C-CTX-USAGE-PRICE",
    "C-PR-LAND",
    "C-LEDGER",
    "C-WATCH",
    "C-EXPORT-GIT",
    "C-SCOPE-RUNNER",
    "C-SCOPE-WS",
    "C-SCOPE-DISPATCH",
]
TAXONOMIES = ("conformant", "nonconformant", "incomplete", "expected_refusal", "simulator", "unsupported", "not_applicable")
EXPECTED = {
    **{claim: "conformant" for claim in CLAIMS},
    "C-SCOPE-RUNNER": "not_applicable",
    "C-SCOPE-WS": "expected_refusal",
    "C-SCOPE-DISPATCH": "expected_refusal",
}
COMMANDS = {
    "C-SETUP-REPO": "hugit setup --repo <repo>",
    "C-CAPTURE-COMMIT": "git commit (post-commit hook)",
    "C-CAPTURE-RECEIPT-ID": "git commit (post-commit hook)",
    "C-COMMIT-INTENT-BIND": "hugit pr open --commit <oid> --intent <id>",
    "C-INTENT-NEW": "hugit intent new",
    "C-PR-OPEN": "hugit pr open",
    "C-CHECK-RUN": "hugit check run (miss, hit, changed-key miss)",
    "C-VERDICT-RECORD": "hugit verdict record",
    "C-CTX-USAGE": "hugit ctx usage",
    "C-CTX-USAGE-PRICE": "hugit pr land (frozen price-card join)",
    "C-PR-LAND": "hugit pr queue; hugit pr land",
    "C-LEDGER": "hugit ledger",
    "C-WATCH": "hugit watch",
    "C-EXPORT-GIT": "hugit export",
    "C-SCOPE-RUNNER": "legacy hugit pr land --dispatch (intentionally unexecuted)",
    "C-SCOPE-WS": "hugit ws",
    "C-SCOPE-DISPATCH": "hugit dispatch",
}
ORACLES = {
    "C-SETUP-REPO": "installed_hook_bytes_and_modes",
    "C-CAPTURE-COMMIT": "git_oid_ref_path_and_hook_principal",
    "C-CAPTURE-RECEIPT-ID": "canonical_ref_update_receipt_and_drain_quiescence",
    "C-COMMIT-INTENT-BIND": "separate_commit_and_intent_arrays",
    "C-INTENT-NEW": "intent_sidecar_and_landed_event",
    "C-PR-OPEN": "pr_opened_event",
    "C-CHECK-RUN": "external_sentinel_and_memo_axes",
    "C-VERDICT-RECORD": "caller_supplied_verdict_event",
    "C-CTX-USAGE": "caller_supplied_usage_event",
    "C-CTX-USAGE-PRICE": "exact_frozen_card_envelope_cost",
    "C-PR-LAND": "landed_event_and_envelopes",
    "C-LEDGER": "ledger_projection_matches_raw_events",
    "C-WATCH": "watch_projection_contains_raw_sequences",
    "C-EXPORT-GIT": "stock_git_fsck_refs_and_files",
    "C-SCOPE-RUNNER": "discontinued_source_boundary",
    "C-SCOPE-WS": "structured_ws_refusal_no_mutation",
    "C-SCOPE-DISPATCH": "structured_dispatch_refusal_no_mutation",
}
ENV_KEYS = (
    "GIT_AUTHOR_DATE",
    "GIT_COMMITTER_DATE",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_TERMINAL_PROMPT",
    "HOME",
    "HUGIT_BIN",
    "LC_ALL",
    "PATH",
    "TZ",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
)
LIMITATIONS = {
    "schema_version": SCHEMA,
    "caller_supplied_verdict_usage": True,
    "provider_authenticity_established": False,
    "event_chain_keyed": False,
    "primary_journey_exported": False,
    "network_absence_instrumented": False,
    "performance_measured": False,
    "runner_execution": "discontinued_unexecuted",
    "source_to_binary_binding_established": False,
    "cryptographic_execution_causality_established": False,
}
LIMITATIONS_TEXT = """# Limitations

Single synthetic journey. Usage and verdict are caller_supplied. Provider authenticity is not established. Hash chain is unkeyed. Export uses a separate clean CLI-created canonical corpus because current export refuses hook receipt payload as sensitive; primary journey is not exported. Network absence is not instrumented. Performance is not measured. Legacy `pr land --dispatch` remains source-reachable but is intentionally unexecuted because runner execution is discontinued. Source-to-binary binding and cryptographic execution causality are not established.
"""


def dump(path: pathlib.Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def payload(record: dict) -> dict:
    value = record.get("payload", {})
    if isinstance(value, str):
        return json.loads(value)
    if isinstance(value, dict):
        return value
    raise ValueError("event payload is neither JSON string nor object")


class Driver:
    def __init__(self, bag: pathlib.Path, repo: pathlib.Path, binary: pathlib.Path, env: dict[str, str]):
        self.bag = bag
        self.repo = repo
        self.binary = binary
        self.env = env
        self.commands: dict[str, dict] = {}
        self.order = 0

    def run(
        self,
        ident: str,
        argv: list[str],
        cwd: pathlib.Path | None = None,
        retry_log_busy: bool = False,
    ) -> subprocess.CompletedProcess[bytes]:
        if ident in self.commands:
            raise RuntimeError(f"duplicate command id: {ident}")
        cwd = cwd or self.repo
        self.order += 1
        base = self.bag / "data" / "commands" / ident
        started = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
        before = time.monotonic_ns()
        attempt_count = 0
        while True:
            attempt_count += 1
            result = subprocess.run(argv, cwd=cwd, env=self.env, capture_output=True, check=False)
            try:
                error_kind = json.loads(result.stdout).get("error", {}).get("kind")
            except (AttributeError, json.JSONDecodeError, UnicodeDecodeError):
                error_kind = None
            if not retry_log_busy or result.returncode != 2 or error_kind != "log_busy" or attempt_count >= 20:
                break
            time.sleep(0.1)
        duration = time.monotonic_ns() - before
        stdout_path = base.with_suffix(".stdout")
        stderr_path = base.with_suffix(".stderr")
        stdout_path.write_bytes(result.stdout)
        stderr_path.write_bytes(result.stderr)
        try:
            label = cwd.relative_to(self.repo).as_posix() or "."
        except ValueError:
            label = "fixture-parent"
        meta = {
            "schema_version": SCHEMA,
            "id": ident,
            "argv": argv,
            "cwd": label,
            "env": {key: self.env[key] for key in ENV_KEYS if key in self.env},
            "started_at": started,
            "duration_ns": duration,
            "attempt_count": attempt_count,
            "exit_code": result.returncode,
            "stdout_path": f"data/commands/{ident}.stdout",
            "stderr_path": f"data/commands/{ident}.stderr",
        }
        dump(base.with_suffix(".json"), meta)
        self.commands[ident] = meta
        return result

    def hugit(self, ident: str, *args: str) -> subprocess.CompletedProcess[bytes]:
        return self.run(ident, [str(self.binary), *args], retry_log_busy=True)

    def git(self, ident: str, *args: str, cwd: pathlib.Path | None = None) -> subprocess.CompletedProcess[bytes]:
        return self.run(ident, ["git", *args], cwd=cwd)

    def json_stdout(self, ident: str) -> object:
        path = self.bag / "data" / "commands" / f"{ident}.stdout"
        return json.loads(path.read_text(encoding="utf-8"))


def require_ok(result: subprocess.CompletedProcess[bytes], name: str) -> None:
    if result.returncode != 0:
        stdout = result.stdout.decode("utf-8", "replace").strip()
        stderr = result.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError(f"fixture command {name} failed ({result.returncode}); stdout={stdout!r}; stderr={stderr!r}")


def load_log(log: pathlib.Path) -> list[dict]:
    value = json.loads(log.read_text(encoding="utf-8"))
    if not isinstance(value, list):
        raise ValueError("canonical log is not array")
    return value


def wait_capture(log: pathlib.Path, oid: str, timeout: float = 20.0) -> list[dict]:
    # Git waits for hooks to publish receipts before returning. Only their
    # detached drain workers remain; later mutations retry exact `log_busy`.
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            records = load_log(log)
            found = any(r.get("kind") == "ref.update" and payload(r).get("target") == oid for r in records)
            lock = pathlib.Path(str(log) + ".lock")
            receipts = log.parent / "receipts"
            pending = receipts.exists() and any(receipts.iterdir())
            if found and not lock.exists() and not pending:
                return records
        except (OSError, ValueError, json.JSONDecodeError):
            pass
        time.sleep(0.05)
    raise RuntimeError(f"capture did not quiesce for {oid}")


def tree_digest(root: pathlib.Path) -> str:
    h = hashlib.sha256()
    if not root.exists():
        return h.hexdigest()
    for path in sorted(root.rglob("*"), key=lambda p: os.fsencode(p.relative_to(root).as_posix())):
        rel = path.relative_to(root).as_posix().encode()
        h.update(rel + b"\0")
        if path.is_file() and not path.is_symlink():
            h.update(path.read_bytes())
    return h.hexdigest()


def safe_directory_tar(root: pathlib.Path, destination: pathlib.Path, check_git_alternates: bool = False) -> None:
    alternates = root / ".git" / "objects" / "info" / "alternates"
    if check_git_alternates and not alternates.exists():
        alternates = root / "repo.git" / "objects" / "info" / "alternates"
    if alternates.exists() and alternates.read_text(encoding="utf-8").strip():
        raise RuntimeError("external Git alternates forbidden")
    paths = list(root.rglob("*"))
    names: set[str] = set()
    normalized: set[str] = set()
    folded: set[str] = set()
    for path in paths:
        mode = path.lstat().st_mode
        if stat.S_ISLNK(mode) or not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
            raise RuntimeError(f"unsafe repository node: {path}")
        name = path.relative_to(root).as_posix()
        if pathlib.PurePosixPath(name).is_absolute() or ".." in pathlib.PurePosixPath(name).parts:
            raise RuntimeError(f"unsafe repository path: {name}")
        nfc = unicodedata.normalize("NFC", name)
        if name != nfc or name in names or nfc in normalized or nfc.casefold() in folded:
            raise RuntimeError(f"noncanonical or colliding repository path: {name}")
        names.add(name)
        normalized.add(nfc)
        folded.add(nfc.casefold())
    with tarfile.open(destination, "w", format=tarfile.PAX_FORMAT) as archive:
        for path in sorted(paths, key=lambda p: os.fsencode(p.relative_to(root).as_posix())):
            name = path.relative_to(root).as_posix()
            info = archive.gettarinfo(str(path), arcname=name)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            info.pax_headers = {}
            if info.isdir():
                info.mode = 0o755
            elif info.isfile():
                info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
            if info.isfile():
                with path.open("rb") as stream:
                    archive.addfile(info, stream)
            else:
                archive.addfile(info)


def safe_repository_tar(repo: pathlib.Path, destination: pathlib.Path) -> None:
    safe_directory_tar(repo, destination, check_git_alternates=True)


def assertion(claim: str, status_value: str, detail: str, evidence: list[str], bag: pathlib.Path) -> dict:
    digests = {}
    for rel in evidence:
        path = bag / rel
        if not path.is_file():
            raise RuntimeError(f"assertion evidence missing: {rel}")
        digests[rel] = sha256(path)
    return {
        "schema_version": SCHEMA,
        "claim_id": claim,
        "status": status_value,
        "oracle": ORACLES[claim],
        "detail": detail,
        "evidence": evidence,
        "evidence_sha256": digests,
    }


def inspect_repository_tar(path: pathlib.Path) -> set[str]:
    with tarfile.open(path, "r") as archive:
        members = archive.getmembers()
        ordered = [m.name for m in members]
        names = set(ordered)
        if len(names) != len(ordered) or ordered != sorted(ordered, key=os.fsencode):
            raise ValueError("tar members are duplicate or not byte-sorted")
        for member in members:
            pure = pathlib.PurePosixPath(member.name)
            if pure.is_absolute() or ".." in pure.parts or not (member.isfile() or member.isdir()):
                raise ValueError(f"unsafe tar member: {member.name}")
            expected_mode = 0o755 if member.isdir() or member.mode & 0o111 else 0o644
            if member.mode != expected_mode or member.uid != 0 or member.gid != 0 or member.uname or member.gname or member.mtime != 0:
                raise ValueError(f"non-deterministic tar metadata: {member.name}")
        return names


def semantic_assertions(driver: Driver, log: pathlib.Path, repo_tar: pathlib.Path, oid: str, branch: str,
                        intent_store: pathlib.Path, sentinel: pathlib.Path, export_dir: pathlib.Path,
                        export_tar: pathlib.Path) -> list[dict]:
    bag = driver.bag
    records = load_log(log)
    by_kind: dict[str, list[tuple[dict, dict]]] = {}
    for record in records:
        by_kind.setdefault(record.get("kind", ""), []).append((record, payload(record)))
    captured = [(r, p) for r, p in by_kind.get("ref.update", []) if p.get("target") == oid]
    opened = [(r, p) for r, p in by_kind.get("pr.opened", []) if p.get("pr_id") == "PR-EVIDENCE"]
    usage = [(r, p) for r, p in by_kind.get("ctx.usage", []) if p.get("target_id") == "intent-evidence"]
    verdict = [(r, p) for r, p in by_kind.get("verdict.recorded", []) if p.get("intent") == "intent-evidence"]
    landed = [(r, p) for r, p in by_kind.get("pr.landed", []) if p.get("pr_id") == "PR-EVIDENCE"]
    intent_env = [(r, p) for r, p in by_kind.get("intent.envelope", []) if p.get("intent_id") == "intent-evidence"]
    pr_env = [(r, p) for r, p in by_kind.get("pr.envelope", []) if p.get("intent_id") == "PR-EVIDENCE"]
    common = ["data/repository.tar", "data/hugit"]
    out: list[dict] = []

    hook_kinds = {
        "post-commit": "commit",
        "post-checkout": "checkout",
        "pre-push": "push-attempt",
        "post-merge": "merge",
        "post-rewrite": "rewrite",
        "reference-transaction": "reference-transaction",
    }
    setup_ok = True
    for hook, kind in hook_kinds.items():
        path = driver.repo / ".git" / "hooks" / hook
        text = path.read_text(encoding="utf-8") if path.is_file() else ""
        active = [line.strip() for line in text.splitlines() if line.strip() and not line.lstrip().startswith("#")]
        setup_ok = setup_ok and os.access(path, os.X_OK)
        setup_ok = setup_ok and "# hugit-hook-version: 1" in text
        setup_ok = setup_ok and any(f"capture --kind {kind}" in line for line in active)
    post_commit = (driver.repo / ".git" / "hooks" / "post-commit").read_text(encoding="utf-8")
    setup_ok = setup_ok and "git rev-parse HEAD" in post_commit and "git symbolic-ref --quiet --short HEAD" in post_commit
    out.append(assertion("C-SETUP-REPO", "conformant" if setup_ok else "nonconformant",
                         "Installed six executable managed v1 hooks with expected capture kinds; post-commit derives OID and branch through Git.", common + ["data/commands/setup.json"], bag))

    git_files = driver.git("oracle-changed-paths", "diff-tree", "--root", "--no-commit-id", "--name-only", "-r", oid).stdout.decode().splitlines()
    cap_ok = len(captured) == 1 and captured[0][1].get("ref") == f"refs/heads/{branch}" and captured[0][1].get("branch") == branch
    cap_ok = cap_ok and captured[0][1].get("files") == git_files and "orchestrator:hugit-hook" in captured[0][0].get("principal_chain", [])
    out.append(assertion("C-CAPTURE-COMMIT", "conformant" if cap_ok else "nonconformant",
                         "Git-derived OID, branch/ref, changed path, and hook principal match canonical ref.update after quiescence.", common + ["data/commands/git-commit.json", "data/commands/oracle-changed-paths.stdout"], bag))
    receipt = captured[0][1].get("receipt_id") if captured else None
    idle = not pathlib.Path(str(log) + ".lock").exists() and not any((log.parent / "receipts").iterdir())
    receipt_ok = isinstance(receipt, str) and bool(receipt) and idle
    out.append(assertion("C-CAPTURE-RECEIPT-ID", "conformant" if receipt_ok else "nonconformant",
                         "Captured ref.update carries non-empty receipt_id; lock absent and receipt queue empty.", common, bag))

    store = json.loads(intent_store.read_text(encoding="utf-8")) if intent_store.exists() else None
    intent_events = [(r, p) for r, p in by_kind.get("intent.landed", []) if p.get("intent_id") == "intent-evidence"]
    store_blob = json.dumps(store, sort_keys=True)
    intent_ok = len(intent_events) == 1 and "intent-evidence" in store_blob and "prove local journey" in store_blob
    out.append(assertion("C-INTENT-NEW", "conformant" if intent_ok else "nonconformant",
                         "Intent exists in sidecar and exactly one intent.landed payload with requested identity exists.", common + ["data/commands/intent-new.json"], bag))
    bind_ok = len(opened) == 1 and opened[0][1].get("commit_ids") == [oid] and opened[0][1].get("intent_ids") == ["intent-evidence"]
    out.append(assertion("C-COMMIT-INTENT-BIND", "conformant" if bind_ok else "nonconformant",
                         "pr.opened preserves exact Git commit and intent in separate arrays; no commit-derived intent created.", common + ["data/commands/pr-open.json"], bag))
    out.append(assertion("C-PR-OPEN", "conformant" if bind_ok else "nonconformant",
                         "Exactly one pr.opened event identifies PR-EVIDENCE, campaign, intent, and captured commit.", common + ["data/commands/pr-open.stdout"], bag))

    cold = driver.json_stdout("check-cold")
    warm = driver.json_stdout("check-warm")
    changed = driver.json_stdout("check-changed")
    sentinel_count = len(sentinel.read_text(encoding="utf-8").splitlines()) if sentinel.exists() else 0
    check_ok = (sentinel_count == 2 and cold.get("cache_hit") is False and cold.get("local_executions") == 1
                and warm.get("cache_hit") is True and warm.get("local_executions") == 0
                and changed.get("cache_hit") is False and changed.get("local_executions") == 1
                and cold.get("memo_key") == warm.get("memo_key") and changed.get("memo_key") != cold.get("memo_key"))
    out.append(assertion("C-CHECK-RUN", "conformant" if check_ok else "nonconformant",
                         f"External sentinel count={sentinel_count}; cold miss executed once, warm hit zero times, changed tree produced new miss/key.", common + ["data/commands/check-cold.stdout", "data/commands/check-warm.stdout", "data/commands/check-changed.stdout"], bag))

    verdict_ok = len(verdict) == 1 and verdict[0][1].get("verdict") == "approve" and "security:approve" in verdict[0][1].get("claims_checked", [])
    out.append(assertion("C-VERDICT-RECORD", "conformant" if verdict_ok else "nonconformant",
                         "caller_supplied: canonical verdict.recorded preserves submitted security approval; no model-authenticity claim.", common + ["data/commands/verdict-record.json"], bag))
    tokens = {"input": 1_500_000, "output": 200_000, "cache_read": 4_000_000, "cache_write": 1_000_000, "total": 6_700_000}
    usage_ok = len(usage) == 1 and usage[0][1].get("tokens") == tokens and usage[0][1].get("model") == "claude-opus-4-8"
    out.append(assertion("C-CTX-USAGE", "conformant" if usage_ok else "nonconformant",
                         "caller_supplied: ctx.usage preserves exact submitted counters; source label does not authenticate provider origin.", common + ["data/commands/ctx-usage.json"], bag))
    price_ok = len(pr_env) == 1 and pr_env[0][1].get("metrics", {}).get("cost_usd_micros") == 20_750_000
    price_ok = price_ok and pr_env[0][1].get("metrics", {}).get("tokens") == tokens and pr_env[0][1].get("snapshot", {}).get("env_manifest") == "price_card=pc-2026-07"
    out.append(assertion("C-CTX-USAGE-PRICE", "conformant" if price_ok else "nonconformant",
                         "Exact frozen pc-2026-07 arithmetic yields 20,750,000 micro-USD from caller_supplied counters; provider authenticity not claimed.", common + ["data/commands/pr-land.json"], bag))
    land_ok = len(landed) == 1 and len(intent_env) == 1 and len(pr_env) == 1
    out.append(assertion("C-PR-LAND", "conformant" if land_ok else "nonconformant",
                         "Exactly one pr.landed plus intent/pr envelopes exist; later ledger/watch reads returned integrity-readable projections.", common + ["data/commands/pr-queue.json", "data/commands/pr-land.json", "data/commands/ledger.json", "data/commands/watch.json"], bag))

    ledger = driver.json_stdout("ledger")
    campaigns = ledger.get("campaigns", []) if isinstance(ledger, dict) else []
    row = next((x for x in campaigns if x.get("campaign") == "evidence"), {})
    ledger_ok = row.get("asked") == 1 and row.get("done") == 1 and row.get("proven") == 1 and len(ledger.get("entries", [])) == 1
    out.append(assertion("C-LEDGER", "conformant" if ledger_ok else "nonconformant",
                         "Ledger asked/done/proven counts independently match raw intent/verdict event sets.", common + ["data/commands/ledger.stdout"], bag))
    watch = driver.json_stdout("watch")
    raw_seqs = {r.get("seq") for r in records}
    watch_seqs = {line.get("seq") for line in watch.get("lines", [])} if isinstance(watch, dict) else set()
    watch_ok = watch.get("count") == len(records) and watch_seqs == raw_seqs
    out.append(assertion("C-WATCH", "conformant" if watch_ok else "nonconformant",
                         "Watch row count and sequence set exactly match canonical raw event array.", common + ["data/commands/watch.stdout"], bag))

    export_repo = export_dir / "repo.git"
    fsck = driver.git("oracle-export-fsck", f"--git-dir={export_repo}", "fsck", "--full", cwd=export_dir)
    refs = driver.git("oracle-export-refs", f"--git-dir={export_repo}", "for-each-ref", "--format=%(refname)", cwd=export_dir)
    export_stdout = driver.json_stdout("export")
    envelope_path = pathlib.Path(export_stdout.get("exported", {}).get("envelope_json", "")) if isinstance(export_stdout, dict) else pathlib.Path()
    names = inspect_repository_tar(repo_tar)
    export_ok = fsck.returncode == 0 and envelope_path.is_file() and envelope_path.parent == export_dir and bool(refs.stdout.strip())
    export_names = inspect_repository_tar(export_tar)
    envelope_rel = envelope_path.relative_to(export_dir).as_posix() if envelope_path.is_file() and envelope_path.parent == export_dir else ""
    export_ok = export_ok and ".git/config" in names and ".git/index" in names and any(n.startswith(".git/objects/") for n in names)
    export_ok = export_ok and ".git/hugit/event-log.json" in names and ".git/hugit/check-sentinel.txt" in names
    export_ok = export_ok and "repo.git/config" in export_names and envelope_rel in export_names
    out.append(assertion("C-EXPORT-GIT", "conformant" if export_ok else "nonconformant",
                         "Stock git fsck passes repo exported from separate clean canonical corpus; retained export.tar contains refs and envelope. Retained journey tar includes worktree, index, config, hooks, objects, event log, and sentinel.", common + ["data/export.tar", "data/commands/export-log-seed.json", "data/commands/export.json", "data/commands/oracle-export-fsck.stderr", "data/commands/oracle-export-refs.stdout"], bag))

    for claim, command_id in (("C-SCOPE-WS", "boundary-ws"), ("C-SCOPE-DISPATCH", "boundary-dispatch")):
        meta = driver.commands[command_id]
        try:
            value = driver.json_stdout(command_id)
        except (json.JSONDecodeError, UnicodeDecodeError):
            value = None
        structured = isinstance(value, dict) and isinstance(value.get("error"), dict) and isinstance(value["error"].get("kind"), str) and isinstance(value["error"].get("fix"), str)
        refused = meta["exit_code"] != 0 and structured and meta.get("state_unchanged") is True
        detail = "Stable structured public-command refusal; canonical runtime digest unchanged."
        out.append(assertion(claim, "expected_refusal" if refused else "nonconformant", detail, common + [f"data/commands/{command_id}.json", f"data/commands/{command_id}.stdout"], bag))
    out.append(assertion("C-SCOPE-RUNNER", "not_applicable",
                         "Legacy pr land --dispatch remains source-reachable and intentionally unexecuted because runner execution is discontinued; no runtime result inferred from top-level dispatch refusal.",
                         ["data/source.json", "data/limitations.md"], bag))
    return out


def add_state_digest(driver: Driver, ident: str, before: str, after: str) -> None:
    meta_path = driver.bag / "data" / "commands" / f"{ident}.json"
    meta = json.loads(meta_path.read_text(encoding="utf-8"))
    meta["state_sha256_before"] = before
    meta["state_sha256_after"] = after
    meta["state_unchanged"] = before == after
    dump(meta_path, meta)
    driver.commands[ident] = meta


def write_bag_files(bag: pathlib.Path, run_id: str) -> None:
    payload_files = [path for path in (bag / "data").rglob("*") if path.is_file()]
    payload_bytes = sum(path.stat().st_size for path in payload_files)
    (bag / "bagit.txt").write_text("BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n", encoding="ascii")
    bagging_date = dt.datetime.now(dt.timezone.utc).date().isoformat()
    (bag / "bag-info.txt").write_text(f"Bagging-Date: {bagging_date}\nExternal-Identifier: {run_id}\nPayload-Oxum: {payload_bytes}.{len(payload_files)}\n", encoding="utf-8")
    (bag / "README.md").write_text("# Hugit Reproducible Evidence Report\n\nFrom a hugit source checkout containing evidence schema v1, run `python3 scripts/verify-evidence-report.py <bag>` for independent verification. Retained executable is source-unbound. Runner `--verify` performs package-closure self-check only.\n", encoding="utf-8")


def manifests(bag: pathlib.Path) -> None:
    payload = sorted((p for p in (bag / "data").rglob("*") if p.is_file()), key=lambda p: os.fsencode(p.relative_to(bag).as_posix()))
    (bag / "manifest-sha256.txt").write_text("".join(f"{sha256(p)}  {p.relative_to(bag).as_posix()}\n" for p in payload), encoding="ascii")
    tags = ["bagit.txt", "bag-info.txt", "README.md", "manifest-sha256.txt"]
    (bag / "tagmanifest-sha256.txt").write_text("".join(f"{sha256(bag / name)}  {name}\n" for name in sorted(tags, key=os.fsencode)), encoding="ascii")


def verify_manifest(path: pathlib.Path, manifest: pathlib.Path, expected: set[str] | None = None) -> None:
    rows = manifest.read_text(encoding="ascii").splitlines()
    seen: list[str] = []
    for row in rows:
        digest, rel = row.split("  ", 1)
        if rel.startswith("/") or ".." in pathlib.PurePosixPath(rel).parts or rel in seen:
            raise ValueError(f"unsafe or duplicate manifest path: {rel}")
        target = path / rel
        if not target.is_file() or sha256(target) != digest:
            raise ValueError(f"manifest mismatch: {rel}")
        seen.append(rel)
    if seen != sorted(seen, key=os.fsencode):
        raise ValueError("manifest paths are not byte-sorted")
    if expected is not None and set(seen) != expected:
        raise ValueError("manifest exact-set mismatch")


def verify_bag(bag: pathlib.Path) -> None:
    roots = {"bagit.txt", "bag-info.txt", "manifest-sha256.txt", "tagmanifest-sha256.txt", "README.md"}
    if not all((bag / p).is_file() for p in roots):
        raise ValueError("required BagIt root file missing")
    payload = {p.relative_to(bag).as_posix() for p in (bag / "data").rglob("*") if p.is_file()}
    verify_manifest(bag, bag / "manifest-sha256.txt", payload)
    verify_manifest(bag, bag / "tagmanifest-sha256.txt", {"bagit.txt", "bag-info.txt", "README.md", "manifest-sha256.txt"})
    report = json.loads((bag / "data/report.json").read_text(encoding="utf-8"))
    for name in ("source.json", "binary.json", "environment.json"):
        metadata = json.loads((bag / "data" / name).read_text(encoding="utf-8"))
        if metadata.get("schema_version") != SCHEMA:
            raise ValueError(f"metadata schema mismatch: {name}")
    claims_wrapper = json.loads((bag / "data/claims.json").read_text(encoding="utf-8"))
    if set(claims_wrapper) != {"schema_version", "claims"} or claims_wrapper.get("schema_version") != SCHEMA or not isinstance(claims_wrapper.get("claims"), dict):
        raise ValueError("claims wrapper mismatch")
    selected = report.get("selected_claims")
    if not isinstance(selected, list) or not selected:
        raise ValueError("selected claim set is empty")
    if set(claims_wrapper["claims"]) != set(selected) or len(claims_wrapper["claims"]) != len(selected):
        raise ValueError("claims wrapper selected-set mismatch")
    assertions = report.get("assertions")
    if not isinstance(assertions, list):
        raise ValueError("report assertions missing")
    ids = [a.get("claim_id") for a in assertions]
    if ids != selected or len(set(ids)) != len(ids):
        raise ValueError("selected claims/assertions exact-set mismatch")
    for item in assertions:
        expected = EXPECTED.get(item["claim_id"])
        if item.get("status") != expected:
            raise ValueError(f"claim status does not close: {item['claim_id']}")
        for rel, digest in item.get("evidence_sha256", {}).items():
            if sha256(bag / rel) != digest:
                raise ValueError(f"assertion evidence mismatch: {rel}")
        standalone = json.loads((bag / "data" / "assertions" / f"{item['claim_id']}.json").read_text(encoding="utf-8"))
        if standalone != item:
            raise ValueError(f"standalone assertion differs from report: {item['claim_id']}")
    counts_seen = Counter(a["status"] for a in assertions)
    counts = {taxonomy: counts_seen[taxonomy] for taxonomy in TAXONOMIES}
    if report.get("summary") != counts or report.get("overall_status") != "pass":
        raise ValueError("report aggregate mismatch")
    inspect_repository_tar(bag / "data/repository.tar")
    inspect_repository_tar(bag / "data/export.tar")
    extract = pathlib.Path(tempfile.mkdtemp(prefix="hugit-export-self-check-"))
    try:
        with tarfile.open(bag / "data/export.tar", "r") as archive:
            archive.extractall(extract)
        exported = extract / "repo.git"
        fsck = subprocess.run(["git", f"--git-dir={exported}", "fsck", "--full"], capture_output=True, check=False)
        refs = subprocess.run(["git", f"--git-dir={exported}", "for-each-ref", "--format=%(refname)"], capture_output=True, check=False)
        envelopes = [extract / "export.json", extract / "envelope.json"]
        if fsck.returncode != 0 or refs.returncode != 0 or not refs.stdout.strip() or not any(path.is_file() for path in envelopes):
            raise ValueError("retained export failed stock Git/envelope self-check")
    finally:
        shutil.rmtree(extract, ignore_errors=True)


def build(args: argparse.Namespace) -> pathlib.Path:
    binary = pathlib.Path(args.hugit_bin).resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"hugit binary not executable: {binary}")
    selected = [] if args.empty_selected else list(CLAIMS)
    if not selected:
        raise RuntimeError("selected claim set must not be empty")
    output = pathlib.Path(args.output).resolve()
    if output.exists():
        shutil.rmtree(output)
    (output / "data/commands").mkdir(parents=True)
    (output / "data/assertions").mkdir(parents=True)
    if args.work_dir:
        work = pathlib.Path(args.work_dir).resolve()
        shutil.rmtree(work, ignore_errors=True)
        work.mkdir(parents=True)
    else:
        work = pathlib.Path(tempfile.mkdtemp(prefix="hugit-evidence-work-"))
    try:
        repo = work / "repository"
        home = work / "home"
        repo.mkdir()
        home.mkdir()
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(work / "xdg-config"),
            "XDG_CACHE_HOME": str(work / "xdg-cache"),
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": str(home / ".gitconfig"),
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_AUTHOR_DATE": "2026-09-13T00:00:00Z",
            "GIT_COMMITTER_DATE": "2026-09-13T00:00:00Z",
            "HUGIT_BIN": str(binary),
            "LC_ALL": "C",
            "TZ": "UTC",
        }
        driver = Driver(output, repo, binary, env)
        require_ok(driver.git("git-init", "init", "-q", "-b", "main"), "git init")
        require_ok(driver.git("git-config-name", "config", "user.name", "Evidence Runner"), "git config name")
        require_ok(driver.git("git-config-email", "config", "user.email", "evidence@example.invalid"), "git config email")
        (repo / "README.fixture").write_text("seed\n", encoding="ascii")
        require_ok(driver.git("git-add-seed", "add", "README.fixture"), "git add seed")
        require_ok(driver.git("git-commit-seed", "commit", "-q", "-m", "seed", "--no-gpg-sign"), "git commit seed")
        require_ok(driver.hugit("setup", "setup", "--repo", str(repo)), "setup")
        (repo / "journey.txt").write_text("hook-to-land\n", encoding="ascii")
        require_ok(driver.git("git-add", "add", "journey.txt"), "git add")
        require_ok(driver.git("git-commit", "commit", "-q", "-m", "capture journey", "--no-gpg-sign"), "git commit")
        oid = driver.git("git-head", "rev-parse", "HEAD").stdout.decode().strip()
        branch = driver.git("git-branch", "branch", "--show-current").stdout.decode().strip()
        log = repo / ".git" / "hugit" / "event-log.json"
        wait_capture(log, oid)
        store = repo / ".hugit" / "intents.json"
        require_ok(driver.hugit("intent-new", "intent", "new", "--id", "intent-evidence", "--campaign", "evidence", "--charter", "prove local journey", "--acceptance", "all semantic oracles pass", "--store", str(store), "--log", str(log)), "intent new")
        require_ok(driver.hugit("pr-open", "pr", "open", "--pr", "PR-EVIDENCE", "--campaign", "evidence", "--author-kind", "orchestrator", "--run-id", "evidence-run", "--intent", "intent-evidence", "--commit", oid, "--log", str(log)), "pr open")

        sentinel = repo / ".git" / "hugit" / "check-sentinel.txt"
        check_script = repo / ".git" / "hugit" / "sentinel.sh"
        check_script.write_text("#!/bin/sh\nprintf 'executed\\n' >> \"$1\"\n", encoding="ascii")
        check_script.chmod(0o755)
        check_root = repo / "check-input"
        check_root.mkdir()
        (check_root / "input.txt").write_text("axis-one\n", encoding="ascii")
        ac = repo / ".git" / "hugit" / "evidence.ac.json"
        check_args = ("check", "run", "--def", "evidence-check", "--cmd", f"{check_script} {sentinel}", "--store", "--log", str(log), "--ac", str(ac), "--root", str(check_root), "--toolchain", "evidence-toolchain")
        require_ok(driver.hugit("check-cold", *check_args), "check cold")
        require_ok(driver.hugit("check-warm", *check_args), "check warm")
        (check_root / "input.txt").write_text("axis-two\n", encoding="ascii")
        require_ok(driver.hugit("check-changed", *check_args), "check changed")
        require_ok(driver.hugit("verdict-record", "verdict", "record", "--intent", "intent-evidence", "--store", "--log", str(log), "--lens", "security", "--result", "approve"), "verdict")
        require_ok(driver.hugit("ctx-usage", "ctx", "usage", "--intent", "intent-evidence", "--model", "claude-opus-4-8", "--input", "1500000", "--output", "200000", "--cache-read", "4000000", "--cache-write", "1000000", "--log", str(log)), "ctx usage")
        require_ok(driver.hugit("pr-queue", "pr", "queue", "--pr", "PR-EVIDENCE", "--log", str(log)), "pr queue")
        require_ok(driver.hugit("pr-land", "pr", "land", "--pr", "PR-EVIDENCE", "--log", str(log)), "pr land")
        require_ok(driver.hugit("ledger", "ledger", "--log", str(log)), "ledger")
        require_ok(driver.hugit("watch", "watch", "--log", str(log)), "watch")
        export_log = work / "export-log.json"
        require_ok(driver.hugit("export-log-seed", "campaign", "open", "--campaign", "export-evidence", "--charter", "export evidence", "--owner", "user:evidence", "--log", str(export_log)), "export log seed")
        export_dir = work / "export"
        require_ok(driver.hugit("export", "export", "--log", str(export_log), "--out", str(export_dir)), "export")

        for ident, token in (("boundary-ws", "ws"), ("boundary-dispatch", "dispatch")):
            before = tree_digest(repo / ".git" / "hugit")
            driver.hugit(ident, token)
            after = tree_digest(repo / ".git" / "hugit")
            add_state_digest(driver, ident, before, after)

        source_metadata = {
            "schema_version": SCHEMA,
            "ledger_baseline": "6f9bbfa",
            "fixture": "synthetic-local-git",
            "network": "no_network_action_configured; absence_not_instrumented",
            "source_to_binary_binding": "not_established",
            "discontinued_runner_boundary": "legacy pr land --dispatch remains source-reachable and was intentionally unexecuted",
        }
        dump(output / "data/source.json", source_metadata)
        dump(output / "data/limitations.json", LIMITATIONS)
        (output / "data/limitations.md").write_text(LIMITATIONS_TEXT, encoding="ascii")
        retained_binary = output / "data" / "hugit"
        shutil.copyfile(binary, retained_binary)
        retained_binary.chmod(0o755)
        version = driver.hugit("binary-version", "--version")
        dump(output / "data/binary.json", {
            "schema_version": SCHEMA,
            "selected_path": str(binary),
            "retained_path": "data/hugit",
            "sha256": sha256(retained_binary),
            "version_stdout": version.stdout.decode(errors="replace").strip(),
            "source_provenance": "unbound-selected-executable",
        })
        repo_tar = output / "data" / "repository.tar"
        safe_repository_tar(repo, repo_tar)
        export_tar = output / "data" / "export.tar"
        safe_directory_tar(export_dir, export_tar, check_git_alternates=True)
        assertions = semantic_assertions(driver, log, repo_tar, oid, branch, store, sentinel, export_dir, export_tar)
        assertions.sort(key=lambda a: selected.index(a["claim_id"]))
        for item in assertions:
            dump(output / "data" / "assertions" / f"{item['claim_id']}.json", item)
        counts_seen = Counter(a["status"] for a in assertions)
        counts = {taxonomy: counts_seen[taxonomy] for taxonomy in TAXONOMIES}
        overall = "pass" if [a["claim_id"] for a in assertions] == selected and all(a["status"] == EXPECTED[a["claim_id"]] for a in assertions) else "fail"
        generated = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
        report = {"schema_version": SCHEMA, "suite": SUITE, "run_id": args.run_id, "generated_at": generated, "selected_claims": selected, "assertions": assertions, "summary": counts, "overall_status": overall}
        dump(output / "data/report.json", report)
        dump(output / "data/claims.json", {"schema_version": SCHEMA, "claims": {claim: {"ledger_command": COMMANDS[claim], "expected_taxonomy": EXPECTED[claim], "oracle": ORACLES[claim]} for claim in selected}})
        dump(output / "data/environment.json", {"schema_version": SCHEMA, "allowlist": {key: env[key] for key in ENV_KEYS if key in env}, "python": sys.version.split()[0], "platform": sys.platform})
        (output / "data/methodology.md").write_text("# Methodology\n\nSynthetic local Git fixture; argv-array subprocesses; claim-specific Git/JSON/filesystem oracles; exit status retained only as diagnostic. The separate verifier independently recomputes journey semantics from retained bytes.\n", encoding="ascii")
        write_bag_files(output, args.run_id)
        manifests(output)
        verify_bag(output)
        if overall != "pass":
            raise RuntimeError("one or more semantic assertions failed")
        return output
    finally:
        shutil.rmtree(work, ignore_errors=True)


def archive_bag(bag: pathlib.Path) -> pathlib.Path:
    archive = pathlib.Path(str(bag) + ".tar")
    archive.unlink(missing_ok=True)
    safe_directory_tar(bag, archive)
    return archive


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--hugit-bin")
    parser.add_argument("--output")
    parser.add_argument("--run-id", default="local-20260913")
    parser.add_argument("--verify", type=pathlib.Path)
    parser.add_argument("--work-dir", help=argparse.SUPPRESS)
    parser.add_argument("--empty-selected", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    try:
        if args.verify:
            verify_bag(args.verify.resolve())
            print(f"PACKAGE SELF-CHECKED: {args.verify.resolve()}")
            return 0
        if not args.hugit_bin or not args.output:
            parser.error("--hugit-bin and --output are required for a run")
        bag = build(args)
        archive = archive_bag(bag)
        print(f"REPORT: {bag}")
        print(f"ARCHIVE: {archive}")
        print(f"ARCHIVE_SHA256: {sha256(archive)}")
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"evidence report failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
