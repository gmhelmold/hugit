#!/usr/bin/env python3
"""Verify a Hugit Evidence Report v1 without executing its benchmark."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import unicodedata


HEX256 = re.compile(r"[0-9a-f]{64}")
STATUSES = (
    "conformant",
    "nonconformant",
    "incomplete",
    "expected_refusal",
    "simulator",
    "unsupported",
    "not_applicable",
)
PASS_STATUSES = {"conformant", "expected_refusal", "simulator", "unsupported", "not_applicable"}
ROOT_FILES = {"bagit.txt", "bag-info.txt", "manifest-sha256.txt", "tagmanifest-sha256.txt", "README.md"}
TAGGED_FILES = {"bagit.txt", "bag-info.txt", "README.md", "manifest-sha256.txt"}
REQUIRED_PAYLOAD = {
    "data/report.json",
    "data/methodology.md",
    "data/limitations.md",
    "data/limitations.json",
    "data/source.json",
    "data/binary.json",
    "data/environment.json",
    "data/claims.json",
    "data/repository.tar",
    "data/export.tar",
    "data/hugit",
}
ENV_ALLOWLIST = {
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
}
SECRET_KEY = re.compile(
    r"(?:^|_)(?:PASSWORD|PASSWD|SECRET|CREDENTIAL)(?:_|$)"
    r"|(?:^|_)(?:API|HUGIT|AUTH)_TOKEN(?:_|$)"
    r"|(?:^|_)PRIVATE_KEY(?:_|$)",
    re.I,
)
CLAIM_ORDER = (
    "C-SETUP-REPO", "C-CAPTURE-COMMIT", "C-CAPTURE-RECEIPT-ID",
    "C-COMMIT-INTENT-BIND", "C-INTENT-NEW", "C-PR-OPEN", "C-CHECK-RUN",
    "C-VERDICT-RECORD", "C-CTX-USAGE", "C-CTX-USAGE-PRICE", "C-PR-LAND",
    "C-LEDGER", "C-WATCH", "C-EXPORT-GIT", "C-SCOPE-RUNNER", "C-SCOPE-WS",
    "C-SCOPE-DISPATCH",
)
CLAIM_CONTRACT = {
    "C-SETUP-REPO": ("hugit setup --repo <repo>", "conformant", "installed_hook_bytes_and_modes"),
    "C-CAPTURE-COMMIT": ("git commit (post-commit hook)", "conformant", "git_oid_ref_path_and_hook_principal"),
    "C-CAPTURE-RECEIPT-ID": ("git commit (post-commit hook)", "conformant", "canonical_ref_update_receipt_and_drain_quiescence"),
    "C-COMMIT-INTENT-BIND": ("hugit pr open --commit <oid> --intent <id>", "conformant", "separate_commit_and_intent_arrays"),
    "C-INTENT-NEW": ("hugit intent new", "conformant", "intent_sidecar_and_landed_event"),
    "C-PR-OPEN": ("hugit pr open", "conformant", "pr_opened_event"),
    "C-CHECK-RUN": ("hugit check run (miss, hit, changed-key miss)", "conformant", "external_sentinel_and_memo_axes"),
    "C-VERDICT-RECORD": ("hugit verdict record", "conformant", "caller_supplied_verdict_event"),
    "C-CTX-USAGE": ("hugit ctx usage", "conformant", "caller_supplied_usage_event"),
    "C-CTX-USAGE-PRICE": ("hugit pr land (frozen price-card join)", "conformant", "exact_frozen_card_envelope_cost"),
    "C-PR-LAND": ("hugit pr queue; hugit pr land", "conformant", "landed_event_and_envelopes"),
    "C-LEDGER": ("hugit ledger", "conformant", "ledger_projection_matches_raw_events"),
    "C-WATCH": ("hugit watch", "conformant", "watch_projection_contains_raw_sequences"),
    "C-EXPORT-GIT": ("hugit export", "conformant", "stock_git_fsck_refs_and_files"),
    "C-SCOPE-RUNNER": ("legacy hugit pr land --dispatch (intentionally unexecuted)", "not_applicable", "discontinued_source_boundary"),
    "C-SCOPE-WS": ("hugit ws", "expected_refusal", "structured_ws_refusal_no_mutation"),
    "C-SCOPE-DISPATCH": ("hugit dispatch", "expected_refusal", "structured_dispatch_refusal_no_mutation"),
}
COMMAND_PREFIXES = {
    "binary-version": ("hugit", "--version"),
    "boundary-dispatch": ("hugit", "dispatch"),
    "boundary-ws": ("hugit", "ws"),
    "check-changed": ("hugit", "check", "run"),
    "check-cold": ("hugit", "check", "run"),
    "check-warm": ("hugit", "check", "run"),
    "ctx-usage": ("hugit", "ctx", "usage"),
    "export-log-seed": ("hugit", "campaign", "open"),
    "export": ("hugit", "export"),
    "intent-new": ("hugit", "intent", "new"),
    "ledger": ("hugit", "ledger"),
    "pr-land": ("hugit", "pr", "land"),
    "pr-open": ("hugit", "pr", "open"),
    "pr-queue": ("hugit", "pr", "queue"),
    "setup": ("hugit", "setup"),
    "verdict-record": ("hugit", "verdict", "record"),
    "watch": ("hugit", "watch"),
    "git-add-seed": ("git", "add"),
    "git-add": ("git", "add"),
    "git-branch": ("git", "branch"),
    "git-commit-seed": ("git", "commit"),
    "git-commit": ("git", "commit"),
    "git-config-email": ("git", "config"),
    "git-config-name": ("git", "config"),
    "git-head": ("git", "rev-parse"),
    "git-init": ("git", "init"),
    "oracle-changed-paths": ("git", "diff-tree"),
    "oracle-export-fsck": ("git",),
    "oracle-export-refs": ("git",),
}
MAX_ARCHIVE_BYTES = 256 * 1024 * 1024
MAX_ARCHIVE_MEMBERS = 100_000
MAX_MEMBER_BYTES = 256 * 1024 * 1024
MAX_EXTRACTED_BYTES = 1024 * 1024 * 1024
EXPECTED_LIMITATIONS = {
    "schema_version": "1.0",
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
EXPECTED_LIMITATIONS_TEXT = """# Limitations

Single synthetic journey. Usage and verdict are caller_supplied. Provider authenticity is not established. Hash chain is unkeyed. Export uses a separate clean CLI-created canonical corpus because current export refuses hook receipt payload as sensitive; primary journey is not exported. Network absence is not instrumented. Performance is not measured. Legacy `pr land --dispatch` remains source-reachable but is intentionally unexecuted because runner execution is discontinued. Source-to-binary binding and cryptographic execution causality are not established.
"""


class Invalid(Exception):
    pass


class DuplicateKey(Invalid):
    pass


def duplicate_safe_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKey(f"duplicate JSON key: {key!r}")
        result[key] = value
    return result


def load_json(path: Path):
    try:
        with path.open("r", encoding="utf-8") as handle:
            return json.load(handle, object_pairs_hook=duplicate_safe_object)
    except (OSError, UnicodeError, json.JSONDecodeError, DuplicateKey) as error:
        raise Invalid(f"invalid JSON {path.name}: {error}") from error


def require(condition, message):
    if not condition:
        raise Invalid(message)


def require_type(value, expected, label):
    if expected is int:
        require(type(value) is int, f"{label} must be integer")
    else:
        require(isinstance(value, expected), f"{label} has wrong type")


def safe_path(text, prefix=None):
    require(isinstance(text, str) and text, "path must be non-empty string")
    require("\\" not in text, f"backslash forbidden in path: {text!r}")
    require("\x00" not in text, f"NUL forbidden in path: {text!r}")
    require(unicodedata.normalize("NFC", text) == text, f"path is not NFC-normalized: {text!r}")
    path = PurePosixPath(text)
    require(not path.is_absolute(), f"absolute path forbidden: {text!r}")
    require(text == path.as_posix(), f"noncanonical path: {text!r}")
    require(path.parts, f"empty path forbidden: {text!r}")
    require(all(part not in ("", ".", "..") for part in path.parts), f"unsafe path: {text!r}")
    if prefix is not None:
        require(path.parts and path.parts[0] == prefix, f"path must be under {prefix}/: {text!r}")
    return path


def sha256_file(path: Path):
    digest = hashlib.sha256()
    try:
        with path.open("rb") as handle:
            for block in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as error:
        raise Invalid(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def parse_manifest(path: Path, required_prefix=None):
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise Invalid(f"cannot read {path.name}: {error}") from error
    require(text.endswith("\n"), f"{path.name} must end with newline")
    entries = []
    seen = set()
    for number, line in enumerate(text.splitlines(), 1):
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, f"{path.name}:{number}: malformed manifest line")
        digest, name = match.groups()
        safe_path(name, required_prefix)
        require(name not in seen, f"{path.name}: duplicate path {name!r}")
        seen.add(name)
        entries.append((name, digest))
    require(entries, f"{path.name} is empty")
    names = [name for name, _ in entries]
    require(names == sorted(names, key=lambda item: item.encode("utf-8")), f"{path.name} is not byte-sorted")
    return entries


def regular_files(root: Path, relative_root: str):
    base = root / relative_root
    require(base.is_dir() and not base.is_symlink(), f"missing directory: {relative_root}")
    found = set()
    for directory, dirnames, filenames in os.walk(base, followlinks=False):
        for name in dirnames:
            require(not (Path(directory) / name).is_symlink(), f"symlink forbidden in package: {Path(directory) / name}")
        for name in filenames:
            path = Path(directory) / name
            require(path.is_file() and not path.is_symlink(), f"non-regular package file: {path}")
            found.add(path.relative_to(root).as_posix())
    return found


def validate_bag_inventory(root: Path):
    require(root.is_dir() and not root.is_symlink(), "REPORT_DIR must be directory")
    entries = set()
    try:
        entries = {entry.name for entry in root.iterdir()}
    except OSError as error:
        raise Invalid(f"cannot list report directory: {error}") from error
    require(entries == ROOT_FILES | {"data"}, f"root inventory mismatch: {sorted(entries)}")
    for name in ROOT_FILES:
        require((root / name).is_file() and not (root / name).is_symlink(), f"missing regular root file: {name}")
    try:
        bagit = (root / "bagit.txt").read_text(encoding="ascii")
    except (OSError, UnicodeError) as error:
        raise Invalid(f"invalid bagit.txt: {error}") from error
    require(bagit == "BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n", "unsupported bagit.txt")


def validate_payload_manifest(root: Path):
    entries = parse_manifest(root / "manifest-sha256.txt", "data")
    manifest_names = {name for name, _ in entries}
    actual_names = regular_files(root, "data")
    require(manifest_names == actual_names, "payload manifest inventory differs from data/ regular files")
    require(REQUIRED_PAYLOAD <= manifest_names, f"required payload missing: {sorted(REQUIRED_PAYLOAD - manifest_names)}")
    command_json = {name for name in manifest_names if re.fullmatch(r"data/commands/[^/]+\.json", name)}
    assertion_json = {name for name in manifest_names if re.fullmatch(r"data/assertions/[^/]+\.json", name)}
    require(command_json, "no command JSON payload")
    require(assertion_json, "no assertion JSON payload")
    for name, digest in entries:
        path = root / name
        require(path.is_file() and not path.is_symlink(), f"manifest target is not regular file: {name}")
        require(sha256_file(path) == digest, f"payload checksum mismatch: {name}")
    return manifest_names


def validate_tag_manifest(root: Path):
    entries = parse_manifest(root / "tagmanifest-sha256.txt")
    names = {name for name, _ in entries}
    require(names == TAGGED_FILES, f"tag manifest inventory mismatch: {sorted(names)}")
    for name, digest in entries:
        require(sha256_file(root / name) == digest, f"tag checksum mismatch: {name}")


def schema_major(value, label):
    require_type(value, dict, label)
    version = value.get("schema_version")
    require_type(version, str, f"{label}.schema_version")
    require(version.split(".", 1)[0] == "1", f"{label}: unsupported schema major {version!r}")


def require_strings(values, label, nonempty=False):
    require_type(values, list, label)
    if nonempty:
        require(values, f"{label} must not be empty")
    require(all(isinstance(value, str) and value for value in values), f"{label} must contain non-empty strings")


def validate_command(command, label, payload_names, root):
    schema_major(command, label)
    for field in ("id", "cwd", "started_at", "stdout_path", "stderr_path"):
        require_type(command.get(field), str, f"{label}.{field}")
        require(command[field] != "", f"{label}.{field} must not be empty")
    require_strings(command.get("argv"), f"{label}.argv", nonempty=True)
    require_type(command.get("env"), dict, f"{label}.env")
    require(all(isinstance(key, str) and isinstance(value, str) for key, value in command["env"].items()), f"{label}.env must map strings to strings")
    unknown_env = set(command["env"]) - ENV_ALLOWLIST
    require(not unknown_env, f"{label}.env has unknown keys: {sorted(unknown_env)}")
    secret_env = [key for key in command["env"] if SECRET_KEY.search(key)]
    require(not secret_env, f"{label}.env has secret-bearing keys: {sorted(secret_env)}")
    require_type(command.get("duration_ns"), int, f"{label}.duration_ns")
    require(command["duration_ns"] >= 0, f"{label}.duration_ns must be nonnegative")
    if "attempt_count" in command:
        require_type(command["attempt_count"], int, f"{label}.attempt_count")
        require(command["attempt_count"] >= 1, f"{label}.attempt_count must be positive")
        require_type(command.get("retry_attempts"), list, f"{label}.retry_attempts")
        require(len(command["retry_attempts"]) == command["attempt_count"] - 1, f"{label}.retry_attempts count mismatch")
        for index, attempt in enumerate(command["retry_attempts"], 1):
            require_type(attempt, dict, f"{label}.retry_attempts[{index}]")
            require(attempt.get("attempt") == index, f"{label}.retry_attempts[{index}].attempt mismatch")
            require_type(attempt.get("duration_ns"), int, f"{label}.retry_attempts[{index}].duration_ns")
            require(attempt["duration_ns"] >= 0, f"{label}.retry_attempts[{index}].duration_ns must be nonnegative")
            require(attempt.get("exit_code") == 2, f"{label}.retry_attempts[{index}].exit_code must be 2")
            retry_base = f"data/commands/retries/{command['id']}-{index:02d}"
            for field, suffix in (("stdout_path", ".stdout"), ("stderr_path", ".stderr")):
                require(attempt.get(field) == retry_base + suffix, f"{label}.retry_attempts[{index}].{field} mismatch")
                require(attempt[field] in payload_names, f"{label}.retry_attempts[{index}].{field} is not manifested")
            retry_stdout = load_json(root / attempt["stdout_path"])
            require_type(retry_stdout, dict, f"{label}.retry_attempts[{index}].stdout")
            retry_error = retry_stdout.get("error")
            require_type(retry_error, dict, f"{label}.retry_attempts[{index}].stdout.error")
            require(retry_error.get("kind") == "log_busy", f"{label}.retry_attempts[{index}] is not log_busy")
    require_type(command.get("exit_code"), int, f"{label}.exit_code")
    if command["cwd"] != ".":
        safe_path(command["cwd"])
    expected_base = f"data/commands/{command['id']}"
    require(label == expected_base + ".json", f"command filename does not match id: {label}")
    for field, suffix in (("stdout_path", ".stdout"), ("stderr_path", ".stderr")):
        name = command[field]
        safe_path(name, "data")
        require(name == expected_base + suffix, f"{label}.{field} basename does not match id")
        require(name in payload_names, f"{label}.{field} is not manifested")


def validate_assertion(assertion, label, payload_names):
    schema_major(assertion, label)
    for field in ("claim_id", "status", "oracle", "detail"):
        require_type(assertion.get(field), str, f"{label}.{field}")
        require(assertion[field] != "", f"{label}.{field} must not be empty")
    require(assertion["status"] in STATUSES, f"{label}.status is unknown")
    require_strings(assertion.get("evidence"), f"{label}.evidence", nonempty=True)
    require_type(assertion.get("evidence_sha256"), dict, f"{label}.evidence_sha256")
    require(set(assertion["evidence"]) == set(assertion["evidence_sha256"]), f"{label}: evidence/hash key mismatch")
    require(len(assertion["evidence"]) == len(set(assertion["evidence"])), f"{label}: duplicate evidence path")
    for name in assertion["evidence"]:
        safe_path(name, "data")
        require(name in payload_names, f"{label}: evidence is absent/unmanifested: {name}")
        digest = assertion["evidence_sha256"][name]
        require(isinstance(digest, str) and HEX256.fullmatch(digest), f"{label}: malformed evidence digest")


def load_and_validate_schemas(root: Path, payload_names):
    report = load_json(root / "data/report.json")
    schema_major(report, "report")
    require(report.get("schema_version") == "1.0", "report.schema_version must equal '1.0'")
    require(report.get("suite") == "hugit-cli-local-journey", "report.suite mismatch")
    for field in ("run_id", "generated_at", "overall_status"):
        require_type(report.get(field), str, f"report.{field}")
        require(report[field] != "", f"report.{field} must not be empty")
    require_strings(report.get("selected_claims"), "report.selected_claims", nonempty=True)
    require_type(report.get("assertions"), list, "report.assertions")
    require_type(report.get("summary"), dict, "report.summary")

    metadata = {}
    for name in ("source.json", "binary.json", "environment.json"):
        metadata[name] = load_json(root / "data" / name)
        schema_major(metadata[name], name)
    binary = metadata["binary.json"]
    for field in ("selected_path", "retained_path", "sha256", "version_stdout", "source_provenance"):
        require_type(binary.get(field), str, f"binary.json.{field}")
        require(binary[field], f"binary.json.{field} must not be empty")
    require(binary["retained_path"] == "data/hugit", "binary retained_path mismatch")
    require(HEX256.fullmatch(binary["sha256"]) is not None, "binary sha256 malformed")
    require(binary["source_provenance"] == "unbound-selected-executable", "binary source provenance must remain explicitly unbound")
    require(sha256_file(root / binary["retained_path"]) == binary["sha256"], "retained binary checksum mismatch")

    claims = load_json(root / "data/claims.json")
    schema_major(claims, "claims")
    mapping = claims.get("claims")
    require_type(mapping, dict, "claims.claims")
    for claim_id, claim in mapping.items():
        require(isinstance(claim_id, str) and claim_id, "claim id must be non-empty string")
        require_type(claim, dict, f"claim {claim_id}")
        for field in ("ledger_command", "expected_taxonomy", "oracle"):
            require_type(claim.get(field), str, f"claim {claim_id}.{field}")
            require(claim[field] != "", f"claim {claim_id}.{field} must not be empty")
        require(claim["expected_taxonomy"] in STATUSES, f"claim {claim_id}: unknown expected taxonomy")
        require(claim_id in CLAIM_CONTRACT, f"unknown claim contract: {claim_id}")
        expected_command, expected_taxonomy, expected_oracle = CLAIM_CONTRACT[claim_id]
        require(claim == {"ledger_command": expected_command, "expected_taxonomy": expected_taxonomy, "oracle": expected_oracle}, f"claim contract mismatch: {claim_id}")

    commands = {}
    command_files = sorted(name for name in payload_names if re.fullmatch(r"data/commands/[^/]+\.json", name))
    for name in command_files:
        command = load_json(root / name)
        validate_command(command, name, payload_names, root)
        require(command["id"] not in commands, f"duplicate command id: {command['id']}")
        commands[command["id"]] = command
    require(set(commands) == set(COMMAND_PREFIXES), "command inventory differs from journey-v1 contract")
    for command_id, (kind, *prefix) in COMMAND_PREFIXES.items():
        argv = commands[command_id]["argv"]
        expected_program = binary["selected_path"] if kind == "hugit" else "git"
        require(argv[0] == expected_program, f"command program mismatch: {command_id}")
        require(argv[1:1 + len(prefix)] == list(prefix), f"command argv prefix mismatch: {command_id}")
        if command_id in ("boundary-ws", "boundary-dispatch"):
            require(commands[command_id]["exit_code"] != 0, f"boundary command did not refuse: {command_id}")
        else:
            require(commands[command_id]["exit_code"] == 0, f"journey command failed: {command_id}")
    version = commands["binary-version"]
    require(version["exit_code"] == 0, "binary-version command failed")
    require((root / version["stdout_path"]).read_text(encoding="utf-8").strip() == binary["version_stdout"], "binary version output mismatch")

    assertion_files = sorted(name for name in payload_names if re.fullmatch(r"data/assertions/[^/]+\.json", name))
    assertions_by_file = {}
    for name in assertion_files:
        assertion = load_json(root / name)
        validate_assertion(assertion, name, payload_names)
        require(name == f"data/assertions/{assertion['claim_id']}.json", f"assertion filename does not match claim_id: {name}")
        assertions_by_file[name] = assertion

    resolved = []
    referenced_files = []
    for number, item in enumerate(report["assertions"]):
        if isinstance(item, str):
            safe_path(item, "data")
            require(item in assertions_by_file, f"report assertion reference not manifested assertion JSON: {item}")
            referenced_files.append(item)
            resolved.append(assertions_by_file[item])
        elif isinstance(item, dict):
            validate_assertion(item, f"report.assertions[{number}]", payload_names)
            matches = [name for name, assertion in assertions_by_file.items() if assertion == item]
            require(len(matches) == 1, f"inline assertion {number} must match exactly one standalone assertion")
            referenced_files.append(matches[0])
            resolved.append(item)
        else:
            raise Invalid(f"report.assertions[{number}] must be object or reference")
    require(set(referenced_files) == set(assertion_files), "standalone assertion inventory must be referenced exactly once")
    require(len(referenced_files) == len(set(referenced_files)), "duplicate assertion file reference")
    return report, mapping, commands, resolved


def validate_claim_coverage(report, claims, assertions):
    selected = report["selected_claims"]
    require(tuple(selected) == CLAIM_ORDER, "selected claims differ from frozen journey-v1 order")
    require(len(selected) == len(set(selected)), "duplicate selected claim")
    require(set(selected) == set(claims), "selected claims and claims.json differ")
    assertion_ids = [item["claim_id"] for item in assertions]
    require(len(assertion_ids) == len(set(assertion_ids)), "duplicate assertion claim_id")
    require(set(assertion_ids) == set(selected), "selected claims and assertions differ")
    require(assertion_ids == selected, "assertions must follow selected_claims order")
    for assertion in assertions:
        claim = claims[assertion["claim_id"]]
        require(assertion["oracle"] == claim["oracle"], f"oracle mismatch for {assertion['claim_id']}")
    return True


def validate_evidence(root: Path, assertions):
    for assertion in assertions:
        for name, expected in assertion["evidence_sha256"].items():
            require(sha256_file(root / name) == expected, f"evidence checksum mismatch for {assertion['claim_id']}: {name}")


def validate_aggregates(report, claims, assertions):
    counts = {status: 0 for status in STATUSES}
    expected_match = True
    passing = True
    for assertion in assertions:
        status = assertion["status"]
        counts[status] += 1
        expected = claims[assertion["claim_id"]]["expected_taxonomy"]
        expected_match = expected_match and status == expected
        passing = passing and status in PASS_STATUSES
    require(report["summary"] == counts, "report.summary differs from recomputed taxonomy counts")
    recomputed = "pass" if expected_match and passing else "fail"
    require(report["overall_status"] == recomputed, f"overall_status must be {recomputed!r}")


def tar_member_kind(member):
    if member.isfile():
        return "file"
    if member.isdir():
        return "dir"
    if member.issym():
        return "symlink"
    if member.islnk():
        return "hardlink"
    if member.ischr() or member.isblk() or member.isfifo():
        return "special"
    return "unknown"


def inspect_normalized_tar(path: Path, label: str):
    require(path.stat().st_size <= MAX_ARCHIVE_BYTES, f"{label} exceeds compressed/archive byte limit")
    try:
        archive = tarfile.open(path, "r:*")
    except (OSError, tarfile.TarError) as error:
        raise Invalid(f"cannot open {label}: {error}") from error
    with archive:
        members = []
        extracted_bytes = 0
        for member in archive:
            require(len(members) < MAX_ARCHIVE_MEMBERS, f"{label} exceeds member-count limit")
            require(member.size <= MAX_MEMBER_BYTES, f"{label} member exceeds byte limit: {member.name}")
            extracted_bytes += member.size
            require(extracted_bytes <= MAX_EXTRACTED_BYTES, f"{label} exceeds extracted-byte limit")
            members.append(member)
        require(members, f"{label} is empty")
        names = []
        exact = set()
        normalized = set()
        folded = set()
        for member in members:
            name = member.name
            safe_path(name)
            require(name not in exact, f"duplicate tar member: {name}")
            exact.add(name)
            nfc = unicodedata.normalize("NFC", name)
            folded_name = nfc.casefold()
            require(nfc not in normalized, f"Unicode-normalization tar collision: {name}")
            require(folded_name not in folded, f"case-fold tar collision: {name}")
            normalized.add(nfc)
            folded.add(folded_name)
            kind = tar_member_kind(member)
            require(kind in ("file", "dir"), f"forbidden tar member type: {name} ({kind})")
            require(member.uid == 0 and member.gid == 0, f"tar owner ids must be zero: {name}")
            require(member.uname == "" and member.gname == "", f"tar owner names must be empty: {name}")
            require(member.mtime == 0, f"tar mtime must equal zero: {name}")
            mode = stat.S_IMODE(member.mode)
            if kind == "dir":
                require(mode == 0o755, f"tar directory mode mismatch for {name}: {mode:04o}")
            elif name.startswith(".git/hooks/"):
                require(mode == 0o755, f"tar hook must be executable: {name}")
            else:
                require(mode in (0o644, 0o755), f"tar regular mode must be 0644 or 0755: {name}")
            names.append(name)
        require(names == sorted(names, key=lambda item: item.encode("utf-8")), "tar members are not byte-sorted")
        return {member.name: member for member in members}


def inspect_repository_tar(path: Path):
    by_name = inspect_normalized_tar(path, "repository.tar")
    names = set(by_name)
    require(not any(name == "repository" or name.startswith("repository/") for name in names), "repository.tar must use repo-relative members")
    require(".git/objects/info/alternates" not in names, "external Git alternates forbidden")
    for name in (".git", ".git/hooks", ".git/objects", ".git/hugit"):
        require(name in by_name and by_name[name].isdir(), f"repository archive missing directory {name}")
    for name in (".git/HEAD", ".git/config", ".git/index"):
        require(name in by_name and by_name[name].isfile(), f"repository archive missing file {name}")
    require(any(name.startswith(".git/hooks/") and by_name[name].isfile() for name in names), "repository archive has no hook content")
    require(any(re.fullmatch(r"\.git/objects/[0-9a-f]{2}/[0-9a-f]{38}", name) for name in names), "repository archive has no loose Git object")
    has_refs = any(name.startswith(".git/refs/") and by_name[name].isfile() for name in names)
    has_packed_refs = ".git/packed-refs" in by_name and by_name[".git/packed-refs"].isfile()
    require(has_refs or has_packed_refs, "repository archive has no refs or packed-refs")
    require(any(name.startswith(".git/hugit/") and by_name[name].isfile() for name in names), "repository archive has no hugit state")
    return names


def inspect_export_tar(path: Path):
    by_name = inspect_normalized_tar(path, "export.tar")
    names = set(by_name)
    for name in ("repo.git", "repo.git/objects", "repo.git/refs"):
        require(name in by_name and by_name[name].isdir(), f"export archive missing directory {name}")
    for name in ("repo.git/HEAD", "repo.git/config"):
        require(name in by_name and by_name[name].isfile(), f"export archive missing file {name}")
    require(any(name.startswith("repo.git/refs/") and by_name[name].isfile() for name in names), "export archive has no retained ref")
    for name in ("export.json", "redaction-manifest.json"):
        require(name in by_name and by_name[name].isfile(), f"export archive missing file {name}")
    metadata = {}
    try:
        with tarfile.open(path, "r:*") as archive:
            for name in ("export.json", "redaction-manifest.json"):
                stream = archive.extractfile(name)
                require(stream is not None, f"cannot read {name} from export archive")
                metadata[name] = json.loads(stream.read().decode("utf-8"), object_pairs_hook=duplicate_safe_object)
                require_type(metadata[name], dict, name)
    except (OSError, UnicodeError, json.JSONDecodeError, DuplicateKey, tarfile.TarError) as error:
        raise Invalid(f"invalid export metadata JSON: {error}") from error
    exported = metadata["export.json"]
    events = exported.get("events")
    require_type(events, list, "export.json.events")
    require(len(events) == 1, "export corpus must contain exactly one event")
    event = events[0]
    require_type(event, dict, "export event")
    require(event.get("kind") == "campaign.opened", "export event kind mismatch")
    payload = json.loads(event.get("payload", ""), object_pairs_hook=duplicate_safe_object)
    require(payload == {"campaign": "export-evidence", "charter": "export evidence", "owner": "user:evidence"}, "export event payload mismatch")
    require(metadata["redaction-manifest.json"].get("removals") == [], "unexpected export redactions")
    return names


def extract_repository(path: Path, destination: Path):
    try:
        with tarfile.open(path, "r:*") as archive:
            archive.extractall(destination)
    except (OSError, tarfile.TarError) as error:
        raise Invalid(f"safe repository extraction failed: {error}") from error


def run_git_fsck(repository: Path, bare=False):
    git = shutil.which("git")
    require(git is not None, "stock git not found")
    env = {
        "PATH": os.environ.get("PATH", ""),
        "HOME": str(repository.parent),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_OPTIONAL_LOCKS": "0",
        "LC_ALL": "C",
    }
    try:
        command = [git, f"--git-dir={repository}", "fsck", "--full"] if bare else [git, "-C", str(repository), "fsck", "--full"]
        result = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            timeout=60,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise Invalid(f"git fsck failed to run: {error}") from error
    require(result.returncode == 0, f"git fsck --full failed: {result.stderr.decode('utf-8', 'replace').strip()}")
    if bare:
        try:
            refs = subprocess.run(
                [git, f"--git-dir={repository}", "for-each-ref", "--format=%(refname)"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=env,
                timeout=30,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise Invalid(f"git for-each-ref failed to run: {error}") from error
        require(refs.returncode == 0 and refs.stdout.strip(), "export has no parseable retained ref")
        require(refs.stdout.decode("utf-8", "strict").splitlines() == ["refs/heads/main"], "export ref set mismatch")
        def bare_output(*arguments):
            result = subprocess.run(
                [git, f"--git-dir={repository}", *arguments],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=env,
                timeout=30,
                check=False,
            )
            require(result.returncode == 0, f"export Git semantic check failed: {' '.join(arguments)}")
            return result.stdout
        require(bare_output("log", "-1", "--format=%s", "refs/heads/main").decode().strip() == "hugit export: synthetic snapshot", "export commit subject mismatch")
        require(bare_output("ls-tree", "-r", "--name-only", "refs/heads/main").decode().splitlines() == ["REFS"], "export tree shape mismatch")
        require(bare_output("show", "refs/heads/main:REFS") == b"", "export REFS payload mismatch")


def lp(text):
    encoded = text.encode("utf-8")
    require(len(encoded) <= 0xFFFFFFFF, "event field too large")
    return struct.pack(">I", len(encoded)) + encoded


def event_hash(record):
    preimage = lp(record["prev_hash"]) + lp(record["kind"])
    preimage += struct.pack(">I", len(record["principal_chain"]))
    preimage += b"".join(lp(item) for item in record["principal_chain"])
    preimage += lp(record["payload"]) + struct.pack(">Q", record["seq"])
    return hashlib.sha256(preimage).hexdigest()


def verify_event_chain(repository: Path):
    path = repository / ".git/hugit/event-log.json"
    require(path.is_file() and not path.is_symlink(), "canonical event-log.json missing")
    records = load_json(path)
    require_type(records, list, "event log")
    redactions = {}
    for record in records:
        if not isinstance(record, dict) or record.get("kind") != "provenance.redaction":
            continue
        try:
            marker = json.loads(record.get("payload", ""), object_pairs_hook=duplicate_safe_object)
        except (json.JSONDecodeError, DuplicateKey):
            continue
        if not isinstance(marker, dict):
            continue
        redacted_seq = marker.get("redacted_seq")
        original = marker.get("original_this_hash")
        redacted = marker.get("redacted_this_hash")
        if type(redacted_seq) is int and redacted_seq >= 0 and isinstance(original, str) and original and isinstance(redacted, str) and redacted:
            redactions[redacted_seq] = (original, redacted)
    previous = "0" * 64
    required_fields = {"seq", "prev_hash", "this_hash", "kind", "principal_chain", "payload", "recorded_at"}
    for position, record in enumerate(records):
        require_type(record, dict, f"event[{position}]")
        require(set(record) == required_fields, f"event[{position}] fields differ from EventRecord v1")
        require_type(record["seq"], int, f"event[{position}].seq")
        require(record["seq"] == position, f"event[{position}] sequence mismatch")
        require_type(record["recorded_at"], int, f"event[{position}].recorded_at")
        require(record["recorded_at"] >= 0, f"event[{position}].recorded_at must be nonnegative")
        for field in ("prev_hash", "this_hash", "kind", "payload"):
            require_type(record[field], str, f"event[{position}].{field}")
        require_strings(record["principal_chain"], f"event[{position}].principal_chain")
        require(record["prev_hash"] == previous, f"event[{position}] prev_hash mismatch")
        require(HEX256.fullmatch(record["this_hash"]) is not None, f"event[{position}] malformed this_hash")
        try:
            json.loads(record["payload"], object_pairs_hook=duplicate_safe_object)
        except (json.JSONDecodeError, DuplicateKey) as error:
            raise Invalid(f"event[{position}] payload is not readable unique-key JSON: {error}") from error
        computed = event_hash(record)
        if position in redactions:
            original, redacted = redactions[position]
            require(record["this_hash"] == original, f"event[{position}] preserved redaction hash mismatch")
            require(computed == redacted, f"event[{position}] redacted content hash mismatch")
        else:
            require(record["this_hash"] == computed, f"event[{position}] this_hash mismatch")
        previous = record["this_hash"]


def json_payload(record, label):
    try:
        value = json.loads(record["payload"], object_pairs_hook=duplicate_safe_object)
    except (json.JSONDecodeError, DuplicateKey, KeyError, TypeError) as error:
        raise Invalid(f"{label} has invalid payload: {error}") from error
    require_type(value, dict, f"{label}.payload")
    return value


def command_stdout(root: Path, commands, command_id):
    require(command_id in commands, f"semantic oracle missing command: {command_id}")
    return load_json(root / commands[command_id]["stdout_path"])


def command_option(commands, command_id, flag):
    argv = commands[command_id]["argv"]
    require(argv.count(flag) == 1, f"{command_id} must contain exactly one {flag}")
    index = argv.index(flag)
    require(index + 1 < len(argv), f"{command_id} has no value for {flag}")
    return argv[index + 1]


def git_output(repository: Path, *arguments):
    git = shutil.which("git")
    require(git is not None, "stock git not found")
    env = {
        "PATH": os.environ.get("PATH", ""),
        "HOME": str(repository.parent),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_OPTIONAL_LOCKS": "0",
        "LC_ALL": "C",
    }
    try:
        result = subprocess.run(
            [git, "-C", str(repository), *arguments],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise Invalid(f"stock git oracle failed to run: {error}") from error
    require(result.returncode == 0, f"stock git oracle failed: {result.stderr.decode('utf-8', 'replace').strip()}")
    return result.stdout.decode("utf-8", "strict").strip()


def caller_boundary_detail(detail, subject):
    normalized = detail.lower().replace("_", "-").replace(" ", "-")
    require("caller-supplied" in normalized, f"{subject} detail must state caller-supplied boundary")
    require("authentic" in normalized and ("not" in normalized or "no-" in normalized), f"{subject} detail must disclaim authenticity")


def semantic_oracles(root: Path, repository: Path, report, claims, commands, assertions):
    selected = report["selected_claims"]
    unknown = set(selected) - set(CLAIM_CONTRACT)
    require(not unknown, f"unknown selected claims: {sorted(unknown)}")
    assertions_by_id = {item["claim_id"]: item for item in assertions}
    records = load_json(repository / ".git/hugit/event-log.json")
    require_type(records, list, "event log")
    events = []
    for number, record in enumerate(records):
        events.append((record, json_payload(record, f"event[{number}]")))

    def matching(kind, predicate=lambda payload: True):
        return [(record, payload) for record, payload in events if record.get("kind") == kind and predicate(payload)]

    head = git_output(repository, "rev-parse", "HEAD")
    branch = git_output(repository, "branch", "--show-current")
    changed_paths = git_output(repository, "diff-tree", "--root", "--no-commit-id", "--name-only", "-r", head).splitlines()
    captured = matching("ref.update", lambda payload: payload.get("target") == head)
    require(command_option(commands, "intent-new", "--id") == "intent-evidence", "intent-new id argv mismatch")
    require(command_option(commands, "intent-new", "--campaign") == "evidence", "intent-new campaign argv mismatch")
    require(command_option(commands, "intent-new", "--charter") == "prove local journey", "intent-new charter argv mismatch")
    require(command_option(commands, "intent-new", "--acceptance") == "all semantic oracles pass", "intent-new acceptance argv mismatch")
    require(command_option(commands, "pr-open", "--pr") == "PR-EVIDENCE", "pr-open id argv mismatch")
    require(command_option(commands, "pr-open", "--intent") == "intent-evidence", "pr-open intent argv mismatch")
    require(command_option(commands, "pr-open", "--commit") == head, "pr-open commit argv mismatch")
    require(command_option(commands, "verdict-record", "--intent") == "intent-evidence", "verdict intent argv mismatch")
    require(command_option(commands, "verdict-record", "--lens") == "security", "verdict lens argv mismatch")
    require(command_option(commands, "verdict-record", "--result") == "approve", "verdict result argv mismatch")
    usage_argv = {flag: command_option(commands, "ctx-usage", flag) for flag in ("--intent", "--model", "--input", "--output", "--cache-read", "--cache-write")}
    require(usage_argv == {"--intent":"intent-evidence", "--model":"claude-opus-4-8", "--input":"1500000", "--output":"200000", "--cache-read":"4000000", "--cache-write":"1000000"}, "ctx-usage argv mismatch")
    require(command_option(commands, "pr-queue", "--pr") == "PR-EVIDENCE", "pr-queue argv mismatch")
    require(command_option(commands, "pr-land", "--pr") == "PR-EVIDENCE", "pr-land argv mismatch")
    opened = matching("pr.opened", lambda payload: payload.get("pr_id") == "PR-EVIDENCE")
    intent_events = matching("intent.landed", lambda payload: payload.get("intent_id") == "intent-evidence")
    verdicts = matching("verdict.recorded", lambda payload: payload.get("intent") == "intent-evidence")
    usages = matching("ctx.usage", lambda payload: payload.get("target_id") == "intent-evidence")
    landed = matching("pr.landed", lambda payload: payload.get("pr_id") == "PR-EVIDENCE")
    intent_envelopes = matching("intent.envelope", lambda payload: payload.get("intent_id") == "intent-evidence")
    pr_envelopes = matching("pr.envelope", lambda payload: payload.get("intent_id") == "PR-EVIDENCE")

    hook_kinds = {
        "post-commit": "commit", "post-checkout": "checkout", "pre-push": "push-attempt",
        "post-merge": "merge", "post-rewrite": "rewrite", "reference-transaction": "reference-transaction",
    }
    setup_ok = True
    for hook_name, kind in hook_kinds.items():
        hook = repository / ".git/hooks" / hook_name
        text = hook.read_text(encoding="utf-8") if hook.is_file() else ""
        active = [line.strip() for line in text.splitlines() if line.strip() and not line.lstrip().startswith("#")]
        setup_ok = setup_ok and os.access(hook, os.X_OK)
        setup_ok = setup_ok and "# hugit-hook-version: 1" in text
        setup_ok = setup_ok and any(f"capture --kind {kind}" in line for line in active)
        capture_index = next((index for index, line in enumerate(active) if f"capture --kind {kind}" in line), None)
        setup_ok = setup_ok and capture_index is not None
        if capture_index is not None:
            setup_ok = setup_ok and not any(line.startswith(("exit", "return", "exec ")) for line in active[:capture_index])
    post_commit = (repository / ".git/hooks/post-commit").read_text(encoding="utf-8")
    post_active = [line.strip() for line in post_commit.splitlines() if line.strip() and not line.lstrip().startswith("#")]
    setup_ok = setup_ok and 'HUGIT_BIN="${HUGIT_BIN:-hugit}"' in post_active
    setup_ok = setup_ok and any(line.startswith("OID=$(git rev-parse HEAD") for line in post_active)
    setup_ok = setup_ok and any(line.startswith("BRANCH=$(git symbolic-ref --quiet --short HEAD") for line in post_active)
    setup_ok = setup_ok and any(line.startswith('( "$HUGIT_BIN" capture --kind commit') for line in post_active)

    capture_ok = len(captured) == 1
    if captured:
        capture_record, capture_payload = captured[0]
        capture_ok = capture_ok and capture_payload.get("branch") == "main" and branch == "main"
        capture_ok = capture_ok and capture_payload.get("ref") == "refs/heads/main"
        capture_ok = capture_ok and capture_payload.get("files") == changed_paths
        capture_ok = capture_ok and "orchestrator:hugit-hook" in capture_record.get("principal_chain", [])
    receipts = repository / ".git/hugit/receipts"
    pending = receipts.exists() and any(receipts.iterdir())
    receipt_ok = bool(captured and isinstance(captured[0][1].get("receipt_id"), str) and captured[0][1]["receipt_id"])
    receipt_ok = receipt_ok and not pending and not (repository / ".git/hugit/event-log.json.lock").exists()

    sidecar = load_json(repository / ".hugit/intents.json")
    require_type(sidecar, dict, "intent sidecar")
    sidecars = sidecar.get("sidecars")
    require_type(sidecars, list, "intent sidecar.sidecars")
    intent_rows = [row for row in sidecars if isinstance(row, dict) and row.get("intent_id") == "intent-evidence"]
    intent_ok = len(intent_events) == 1 and len(intent_rows) == 1
    if intent_rows:
        intent_ok = intent_ok and intent_rows[0].get("charter") == "prove local journey"
        intent_ok = intent_ok and intent_rows[0].get("acceptance") == ["all semantic oracles pass"]
        intent_ok = intent_ok and intent_rows[0].get("authoritative") is False
    binding_ok = len(opened) == 1 and opened[0][1].get("commit_ids") == [head] and opened[0][1].get("intent_ids") == ["intent-evidence"]
    pr_open_ok = binding_ok and opened[0][1].get("campaign") == "evidence"

    cold = command_stdout(root, commands, "check-cold")
    warm = command_stdout(root, commands, "check-warm")
    changed = command_stdout(root, commands, "check-changed")
    require_type(cold, dict, "check-cold stdout")
    require_type(warm, dict, "check-warm stdout")
    require_type(changed, dict, "check-changed stdout")
    sentinel = repository / ".git/hugit/check-sentinel.txt"
    sentinel_lines = sentinel.read_text(encoding="utf-8").splitlines() if sentinel.is_file() else []
    check_ok = len(sentinel_lines) == 2
    check_ok = check_ok and cold.get("cache_hit") is False and cold.get("local_executions") == 1
    check_ok = check_ok and warm.get("cache_hit") is True and warm.get("local_executions") == 0
    check_ok = check_ok and changed.get("cache_hit") is False and changed.get("local_executions") == 1
    check_ok = check_ok and cold.get("memo_key") == warm.get("memo_key") and changed.get("memo_key") != cold.get("memo_key")

    verdict_ok = len(verdicts) == 1 and verdicts[0][1].get("verdict") == "approve"
    verdict_ok = verdict_ok and "security:approve" in verdicts[0][1].get("claims_checked", [])
    tokens = {"input": 1_500_000, "output": 200_000, "cache_read": 4_000_000, "cache_write": 1_000_000, "total": 6_700_000}
    usage_ok = len(usages) == 1 and usages[0][1].get("tokens") == tokens and usages[0][1].get("model") == "claude-opus-4-8"
    usage_ok = usage_ok and usages[0][1].get("target_kind") == "intent" and usages[0][1].get("source") == "provider_usage"
    price_ok = len(pr_envelopes) == 1
    if pr_envelopes:
        price_payload = pr_envelopes[0][1]
        price_ok = price_ok and price_payload.get("metrics", {}).get("cost_usd_micros") == 20_750_000
        price_ok = price_ok and price_payload.get("metrics", {}).get("tokens") == tokens
        price_ok = price_ok and price_payload.get("snapshot", {}).get("env_manifest") == "price_card=pc-2026-07"
    queued = matching("pr.queued", lambda payload: payload.get("pr_id") == "PR-EVIDENCE")
    land_ok = len(queued) == len(landed) == len(intent_envelopes) == len(pr_envelopes) == 1
    if landed:
        land_ok = land_ok and landed[0][1].get("campaign") == "evidence"
    if intent_envelopes:
        intent_metrics = intent_envelopes[0][1].get("metrics", {})
        intent_tokens = intent_metrics.get("tokens", {})
        land_ok = land_ok and intent_metrics.get("cost_usd_micros") == 0
        land_ok = land_ok and isinstance(intent_tokens, dict) and all(intent_tokens.get(key) == 0 for key in tokens)

    ledger = command_stdout(root, commands, "ledger")
    require_type(ledger, dict, "ledger stdout")
    campaign_intents = {payload.get("intent_id") for _, payload in intent_events if payload.get("campaign") == "evidence"}
    opened_for_campaign = [payload for _, payload in opened if payload.get("campaign") == "evidence"]
    landed_prs = {payload.get("pr_id") for _, payload in landed}
    done_intents = {intent for payload in opened_for_campaign if payload.get("pr_id") in landed_prs for intent in payload.get("intent_ids", [])}
    approved_intents = {payload.get("intent") for _, payload in verdicts if payload.get("verdict") == "approve"}
    expected_counts = (len(campaign_intents), len(campaign_intents & done_intents), len(campaign_intents & approved_intents))
    campaigns = ledger.get("campaigns", [])
    ledger_row = next((row for row in campaigns if isinstance(row, dict) and row.get("campaign") == "evidence"), {})
    ledger_entries = ledger.get("entries", [])
    ledger_ids = {row.get("intent_id") for row in ledger_entries if isinstance(row, dict)}
    ledger_ok = (ledger_row.get("asked"), ledger_row.get("done"), ledger_row.get("proven")) == expected_counts
    ledger_ok = ledger_ok and ledger_ids == campaign_intents and len(ledger_entries) == len(campaign_intents)

    watch = command_stdout(root, commands, "watch")
    require_type(watch, dict, "watch stdout")
    lines = watch.get("lines", [])
    watch_ok = isinstance(lines, list) and watch.get("count") == len(records) and len(lines) == len(records)
    watch_ok = watch_ok and [line.get("seq") for line in lines if isinstance(line, dict)] == [record["seq"] for record in records]
    classes = {"ref.update": "git-activity", "intent.landed": "landing", "verdict.recorded": "verdict"}
    redacted_kinds = {"ref.update", "pr.opened", "check.recorded", "verdict.recorded"}
    for line, record in zip(lines, records):
        if not isinstance(line, dict):
            watch_ok = False
            continue
        text = line.get("text", "")
        watch_ok = watch_ok and line.get("class") == classes.get(record["kind"], "other")
        watch_ok = watch_ok and f"seq={record['seq']} kind={record['kind']}" in text
        watch_ok = watch_ok and (("[REDACTED]" in text) == (record["kind"] in redacted_kinds))

    export_seed = command_stdout(root, commands, "export-log-seed")
    export_result = command_stdout(root, commands, "export")
    export_ok = isinstance(export_seed, dict) and export_seed.get("campaign") == "export-evidence"
    export_ok = export_ok and export_seed.get("charter") == "export evidence" and export_seed.get("owner") == "user:evidence"
    exported = export_result.get("exported", {}) if isinstance(export_result, dict) else {}
    export_ok = export_ok and exported.get("schema_version") == "1.0.0"
    export_ok = export_ok and PurePosixPath(exported.get("envelope_json", "")).name == "export.json"
    export_ok = export_ok and PurePosixPath(exported.get("redaction_manifest", "")).name == "redaction-manifest.json"
    export_ok = export_ok and PurePosixPath(exported.get("git_dir", "")).name == "repo.git"

    outcomes = {
        "C-SETUP-REPO": setup_ok,
        "C-CAPTURE-COMMIT": capture_ok,
        "C-CAPTURE-RECEIPT-ID": receipt_ok,
        "C-COMMIT-INTENT-BIND": binding_ok,
        "C-INTENT-NEW": intent_ok,
        "C-PR-OPEN": pr_open_ok,
        "C-CHECK-RUN": check_ok,
        "C-VERDICT-RECORD": verdict_ok,
        "C-CTX-USAGE": usage_ok,
        "C-CTX-USAGE-PRICE": price_ok,
        "C-PR-LAND": land_ok,
        "C-LEDGER": ledger_ok,
        "C-WATCH": watch_ok,
        "C-EXPORT-GIT": export_ok,
    }

    caller_boundary_detail(assertions_by_id["C-VERDICT-RECORD"]["detail"], "verdict") if "C-VERDICT-RECORD" in selected else None
    caller_boundary_detail(assertions_by_id["C-CTX-USAGE"]["detail"], "usage") if "C-CTX-USAGE" in selected else None
    if "C-CTX-USAGE-PRICE" in selected:
        caller_boundary_detail(assertions_by_id["C-CTX-USAGE-PRICE"]["detail"], "pricing")

    for claim_id, command_id in (("C-SCOPE-WS", "boundary-ws"), ("C-SCOPE-DISPATCH", "boundary-dispatch")):
        if claim_id not in selected:
            continue
        meta = commands.get(command_id)
        require(meta is not None, f"semantic oracle missing command: {command_id}")
        value = command_stdout(root, commands, command_id)
        error = value.get("error") if isinstance(value, dict) else None
        structured = isinstance(error, dict) and isinstance(error.get("kind"), str) and bool(error["kind"])
        structured = structured and isinstance(error.get("fix"), str) and bool(error["fix"])
        before = meta.get("state_sha256_before")
        after = meta.get("state_sha256_after")
        unchanged = isinstance(before, str) and HEX256.fullmatch(before) and before == after and meta.get("state_unchanged") is True
        outcomes[claim_id] = meta["exit_code"] != 0 and structured and bool(unchanged)

    if "C-SCOPE-RUNNER" in selected:
        assertion = assertions_by_id["C-SCOPE-RUNNER"]
        detail = assertion["detail"].lower().replace("_", "-").replace(" ", "-")
        require(claims["C-SCOPE-RUNNER"]["expected_taxonomy"] == "not_applicable", "runner scope claim must be not_applicable")
        require(assertion["status"] == "not_applicable", "runner scope assertion must be not_applicable")
        require("legacy" in detail and "source-reachable" in detail and "intentionally-unexecuted" in detail, "runner scope detail must state legacy source-reachable/intentionally unexecuted boundary")

    for claim_id in selected:
        assertion = assertions_by_id[claim_id]
        expected = claims[claim_id]["expected_taxonomy"]
        if claim_id == "C-SCOPE-RUNNER":
            continue
        require(claim_id in outcomes, f"no semantic oracle implemented for {claim_id}")
        require(outcomes[claim_id], f"semantic oracle failed for {claim_id}")
        require(assertion["status"] == expected, f"assertion status disagrees with semantic oracle for {claim_id}")


def validate_semantic_boundary(root: Path):
    try:
        limitations = (root / "data/limitations.md").read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise Invalid(f"cannot read limitations: {error}") from error
    require(limitations == EXPECTED_LIMITATIONS_TEXT, "limitations.md differs from frozen semantic-boundary text")
    structured = load_json(root / "data/limitations.json")
    require(structured == EXPECTED_LIMITATIONS, "limitations.json differs from frozen semantic-boundary contract")
    source = load_json(root / "data/source.json")
    binary = load_json(root / "data/binary.json")
    require(source.get("source_to_binary_binding") == "not_established", "source-to-binary limitation missing")
    require(binary.get("source_provenance") == "unbound-selected-executable", "binary provenance limitation missing")


class Verifier:
    def __init__(self, root: Path):
        self.root = root
        self.results = []
        self.context = {}

    def check(self, name, function):
        try:
            value = function()
            self.context[name] = value
            self.results.append({"name": name, "status": "pass"})
            return value
        except (Invalid, OSError, ValueError, KeyError, TypeError, IndexError, tarfile.TarError) as error:
            detail = str(error)
            self.results.append({"name": name, "status": "fail", "detail": detail})
            print(f"{name}: {detail}", file=sys.stderr)
            return None

    def run(self):
        self.check("package_inventory", lambda: validate_bag_inventory(self.root))
        payload = self.check("payload_manifest", lambda: validate_payload_manifest(self.root))
        self.check("tag_manifest", lambda: validate_tag_manifest(self.root))
        schemas = None
        if payload is not None:
            schemas = self.check("schemas", lambda: load_and_validate_schemas(self.root, payload))
        else:
            self.results.append({"name": "schemas", "status": "fail", "detail": "blocked by payload_manifest"})
        coverage_ok = None
        if schemas is not None:
            report, claims, _commands, assertions = schemas
            coverage_ok = self.check("claim_coverage", lambda: validate_claim_coverage(report, claims, assertions))
            self.check("evidence", lambda: validate_evidence(self.root, assertions))
            self.check("aggregates", lambda: validate_aggregates(report, claims, assertions))
        else:
            for name in ("claim_coverage", "evidence", "aggregates"):
                self.results.append({"name": name, "status": "fail", "detail": "blocked by schemas"})

        repository_tar = self.root / "data/repository.tar"
        export_tar = self.root / "data/export.tar"
        repository_ok = self.check("repository_archive", lambda: inspect_repository_tar(repository_tar))
        export_ok = self.check("export_archive", lambda: inspect_export_tar(export_tar))
        if repository_ok is not None or export_ok is not None:
            with tempfile.TemporaryDirectory(prefix="hugit-evidence-verify-") as temporary:
                repository = Path(temporary) / "repository"
                export = Path(temporary) / "export"
                if repository_ok is not None:
                    repository.mkdir()
                    try:
                        extract_repository(repository_tar, repository)
                    except Invalid as error:
                        detail = str(error)
                        for name in ("git_fsck", "event_chain", "semantic_oracles"):
                            self.results.append({"name": name, "status": "fail", "detail": detail})
                            print(f"{name}: {detail}", file=sys.stderr)
                    else:
                        self.check("git_fsck", lambda: run_git_fsck(repository))
                        self.check("event_chain", lambda: verify_event_chain(repository))
                        if schemas is not None and coverage_ok is not None:
                            report, claims, commands, assertions = schemas
                            self.check("semantic_oracles", lambda: semantic_oracles(self.root, repository, report, claims, commands, assertions))
                        else:
                            self.results.append({"name": "semantic_oracles", "status": "fail", "detail": "blocked by schemas or claim_coverage"})
                else:
                    for name in ("git_fsck", "event_chain", "semantic_oracles"):
                        self.results.append({"name": name, "status": "fail", "detail": "blocked by repository_archive"})
                if export_ok is not None:
                    export.mkdir()
                    try:
                        extract_repository(export_tar, export)
                    except Invalid as error:
                        self.results.append({"name": "export_git_fsck", "status": "fail", "detail": str(error)})
                        print(f"export_git_fsck: {error}", file=sys.stderr)
                    else:
                        self.check("export_git_fsck", lambda: run_git_fsck(export / "repo.git", bare=True))
                else:
                    self.results.append({"name": "export_git_fsck", "status": "fail", "detail": "blocked by export_archive"})
        else:
            for name in ("git_fsck", "event_chain", "semantic_oracles", "export_git_fsck"):
                self.results.append({"name": name, "status": "fail", "detail": "blocked by archives"})

        self.check("semantic_boundary", lambda: validate_semantic_boundary(self.root))
        valid = all(item["status"] == "pass" for item in self.results)
        taxonomy_counts = {status: 0 for status in STATUSES}
        if schemas is not None:
            for assertion in schemas[3]:
                taxonomy_counts[assertion["status"]] += 1
        return {
            "schema_version": "1.0",
            "overall_status": "valid" if valid else "invalid",
            "taxonomy_counts": taxonomy_counts,
            "checks": self.results,
            "limitations": [
                "No benchmark command was executed.",
                "Historical no-mutation claims validate retained before/after digests but cannot recreate prior state.",
                "Event chain is unkeyed and does not authenticate a writer with full repository access.",
            ],
        }, valid


def main(argv):
    if len(argv) != 2:
        print("usage: verify-evidence-report.py REPORT_DIR", file=sys.stderr)
        return 2
    source = Path(argv[1])
    temporary = None
    initial = []
    try:
        if source.is_file():
            inspect_normalized_tar(source, "evidence archive")
            temporary = tempfile.TemporaryDirectory(prefix="hugit-evidence-outer-")
            root = Path(temporary.name)
            extract_repository(source, root)
            initial.append({"name": "outer_archive", "status": "pass"})
        else:
            root = source
        verifier = Verifier(root)
        verifier.results.extend(initial)
        report, valid = verifier.run()
    except (Invalid, OSError, ValueError, tarfile.TarError) as error:
        report = {
            "schema_version": "1.0",
            "overall_status": "invalid",
            "taxonomy_counts": {status: 0 for status in STATUSES},
            "checks": [{"name": "outer_archive", "status": "fail", "detail": str(error)}],
            "limitations": ["Input archive failed before report verification."],
        }
        valid = False
    finally:
        if temporary is not None:
            temporary.cleanup()
    print(json.dumps(report, sort_keys=True, separators=(",", ":")))
    return 0 if valid else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
