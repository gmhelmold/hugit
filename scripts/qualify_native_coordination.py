#!/usr/bin/env python3
"""Identify and verify the existing native-lock suite; never grant v1 admission.

This gate has its own required test-name set. A truncated --list and test log
must not jointly redefine what qualifies. Unix-only scenarios remain explicit
coverage gaps on Windows, not successful Windows executions.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import subprocess
import sys

SCHEMA = "hugit.native-coordination-observation/2"
CELLS = {
    "linux-x64-ext4": ("Linux", "x86_64", "x86_64-unknown-linux-gnu", "ext4"),
    "macos-arm64-apfs": ("Darwin", "arm64", "aarch64-apple-darwin", "apfs"),
    "macos-intel-apfs": ("Darwin", "x86_64", "x86_64-apple-darwin", "apfs"),
    "windows-x64-ntfs": ("Windows", "amd64", "x86_64-pc-windows-msvc", "ntfs"),
}
SOURCES = (
    "crates/hugit-refstore/src/coordination.rs",
    "crates/hugit-cli/tests/lock_protocol_v2.rs",
    ".github/workflows/native-coordination.yml",
    "scripts/qualify_native_coordination.py",
    "scripts/test_native_coordination_evidence.py",
)
UNIT = {
    "coordination::release_regressions::releasing_resource_unlocks_even_while_a_pre_exec_duplicate_exists",
    "coordination::release_regressions::releasing_admission_unlocks_even_while_a_pre_exec_duplicate_exists",
}
COMMON = set("""
compatible_contenders_are_exclusive_and_release_never_unlinks_or_truncates
reverse_order_schedule_fails_before_open_and_never_deadlocks
equal_and_reverse_lexical_keys_are_refused_without_new_files
held_handle_budget_is_checked_before_creating_another_file
namespace_initialization_never_adopts_existing_or_legacy_data
invalid_keys_and_special_lockfiles_are_refused
process_death_releases_native_lease_without_deleting_its_file
exec_child_does_not_inherit_parent_lock_ownership
disable_requires_all_holders_to_release_and_preserves_files
disabled_namespace_refuses_old_handles_and_reopen_without_new_files
failed_first_acquisition_does_not_leak_admission
malformed_or_missing_state_never_resets_to_active
previous_experimental_protocol_is_not_upgraded_or_adopted
protocol_changes_invalidate_previously_opened_coordinators
process_death_releases_admission_and_disable_stays_persistent
exec_child_cannot_keep_admission_alive_after_parent_release
partial_release_and_failed_extension_retain_admission
concurrent_admission_and_disable_have_a_single_safe_winner
every_partial_disable_value_is_refused_without_repair
""".split())
UNIX_ONLY = set("""
symlink_and_hardlink_lockfiles_are_refused_without_changing_the_target
paused_owner_130_seconds_is_not_stolen_and_death_releases_the_lease
disabled_namespace_remains_inspectable_without_write_access
linked_state_is_refused_and_external_bytes_are_preserved
""".split())
HELPER = "subprocess_fixture"


def require(ok: bool, reason: str) -> None:
    if not ok:
        raise ValueError(reason)


def output(*args: str) -> str:
    return subprocess.check_output(args, text=True, timeout=60).strip()


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_log(path: Path) -> str:
    require(path.is_file() and not path.is_symlink(), "missing or linked log")
    with path.open("rb") as stream:
        raw = stream.read(8 * 1024 * 1024 + 1)
    require(len(raw) <= 8 * 1024 * 1024, "oversize log")
    return raw.decode("utf-8").replace("\r\n", "\n")


def filesystem(path: Path) -> str:
    system = platform.system()
    if system == "Linux":
        return output("findmnt", "--noheadings", "--output", "FSTYPE", "--target", str(path)).lower()
    if system == "Darwin":
        device = output("df", "-P", str(path)).splitlines()[-1].split()[0]
        info = plistlib.loads(subprocess.check_output(["diskutil", "info", "-plist", device], timeout=60))
        return info.get("FilesystemType", "").lower()
    if system == "Windows":
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        volume_path = kernel.GetVolumePathNameW
        volume_path.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD]
        volume_path.restype = wintypes.BOOL
        volume_info = kernel.GetVolumeInformationW
        volume_info.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD,
                                ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(wintypes.DWORD),
                                ctypes.POINTER(wintypes.DWORD), wintypes.LPWSTR, wintypes.DWORD]
        volume_info.restype = wintypes.BOOL
        root = ctypes.create_unicode_buffer(32768)
        name = ctypes.create_unicode_buffer(256)
        if not volume_path(str(path), root, len(root)) or not volume_info(
                root.value, None, 0, None, None, None, name, len(name)):
            raise ctypes.WinError(ctypes.get_last_error())
        return name.value.lower()
    raise ValueError("unsupported observation platform")


def validate_identity(subject: dict, cell: str) -> None:
    require(subject.get("schema") == SCHEMA and subject.get("cell") == cell, "subject schema/cell mismatch")
    system, arch, host, fs = CELLS[cell]
    require(subject.get("system") == system and subject.get("architecture", "").lower() == arch,
            "native architecture/system mismatch")
    require(subject.get("filesystem", "").lower() == fs, "fixture filesystem mismatch")
    require(re.findall(r"^host: (.+)$", subject.get("rustc", ""), re.M) == [host], "Rust host mismatch")
    require(subject.get("target") == host, "cross target is not native qualification")
    require(subject.get("fixture_root") == subject.get("rust_temp_dir"), "Rust fixture root differs")
    require(bool(subject.get("fixture_root")), "empty fixture root")
    require(set(subject.get("files", {})) == set(SOURCES), "source exact-set mismatch")
    require(all(re.fullmatch(r"[a-f0-9]{64}", x) for x in subject["files"].values()), "invalid source digest")
    for field in ("checkout", "tree", "subject_head"):
        require(bool(re.fullmatch(r"[a-f0-9]{40}", subject.get(field, ""))), "invalid Git identity")


def identify(root: Path, evidence: Path, fixtures: Path, cell: str) -> None:
    require(fixtures.is_dir(), "fixture root missing")
    host = CELLS[cell][2]
    # Execute a tiny native Rust probe: Python's TMPDIR is not proof of the path
    # selected by std::env::temp_dir(), especially on Windows.
    probe_source = evidence / "temp-probe.rs"
    probe_source.write_text('fn main() { println!("{}", std::env::temp_dir().display()); }\n')
    probe = evidence / ("temp-probe.exe" if platform.system() == "Windows" else "temp-probe")
    compiled = subprocess.run(["rustc", "--crate-name", "native_temp_probe", "--target", host,
                               str(probe_source), "-o", str(probe)], capture_output=True, timeout=60)
    (evidence / "temp-probe-build.log").write_bytes(compiled.stdout + compiled.stderr)
    require(compiled.returncode == 0, "native fixture probe did not compile")
    observed_temp = Path(output(str(probe))).resolve(strict=True)
    subject = {
        "schema": SCHEMA, "cell": cell, "system": platform.system(),
        "architecture": platform.machine(), "filesystem": filesystem(observed_temp),
        "rustc": output("rustc", "--version", "--verbose"), "target": host,
        "fixture_root": str(fixtures.resolve(strict=True)), "rust_temp_dir": str(observed_temp),
        "temp_probe_sha256": digest(probe),
        "checkout": output("git", "-C", str(root), "rev-parse", "HEAD"),
        "tree": output("git", "-C", str(root), "rev-parse", "HEAD^{tree}"),
        "subject_head": os.environ["SUBJECT_HEAD"],
        "run_id": os.environ["GITHUB_RUN_ID"], "attempt": os.environ["GITHUB_RUN_ATTEMPT"],
        "files": {name: digest(root / name) for name in SOURCES},
    }
    (evidence / "subject.json").write_text(json.dumps(subject, indent=2) + "\n")
    validate_identity(subject, cell)
    print(json.dumps(subject, indent=2))


def outcomes(cell: str, logs: dict[str, str]) -> dict:
    unix = CELLS[cell][0] != "Windows"
    groups = {"unit": UNIT, "integration": COMMON | (UNIX_ONLY if unix else set())}
    result = {}
    for group, expected in groups.items():
        listed = re.findall(r"^([A-Za-z0-9_:]+): test$", logs[group + "-list"], re.M)
        wanted = expected | ({HELPER} if group == "integration" else set())
        require(len(listed) == len(wanted) and set(listed) == wanted, group + " required selection differs")
        log = logs[group]
        passed = re.findall(r"^test ([A-Za-z0-9_:]+) \.\.\. ok$", log, re.M)
        ignored = re.findall(r"^test ([A-Za-z0-9_:]+) \.\.\. ignored", log, re.M)
        require(len(passed) == len(expected) and set(passed) == expected, group + " passed names differ")
        require(ignored == ([HELPER] if group == "integration" else []), group + " unexpected ignore")
        summaries = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", log, re.M)
        require(summaries == [(str(len(expected)), "0", str(len(ignored)))], group + " summary mismatch")
        result[group] = {"passed": len(passed), "names": sorted(passed)}
    pauses = [int(n) for n in re.findall(r"owner_paused_ms=(\d+)", logs["integration"])]
    require((len(pauses) == 1 and pauses[0] >= 130000) if unix else not pauses, "pause evidence differs")
    result.update({"owner_paused_ms": pauses, "unix_only_not_executed": [] if unix else sorted(UNIX_ONLY)})
    return result


def verify(root: Path, evidence: Path, fixtures: Path, cell: str) -> None:
    subject = json.loads((evidence / "subject.json").read_text())
    validate_identity(subject, cell)
    require(str(fixtures.resolve(strict=True)) == subject["fixture_root"], "fixture root changed")
    require(filesystem(fixtures) == CELLS[cell][3], "fixture filesystem changed")
    logs = {name: read_log(evidence / (name + ".log"))
            for name in ("unit-list", "unit", "integration-list", "integration")}
    result = outcomes(cell, logs)
    leftovers = [p.name for p in fixtures.iterdir()
                 if p.name.startswith(("hugit-native-lock-", "hugit-lock-release-"))]
    require(not leftovers, "owned fixtures remain: " + str(leftovers[:10]))
    require(all(digest(root / p) == d for p, d in subject["files"].items()), "source changed")
    require(output("git", "-C", str(root), "rev-parse", "HEAD") == subject["checkout"], "checkout changed")
    require(output("git", "-C", str(root), "rev-parse", "HEAD^{tree}") == subject["tree"], "tree changed")
    require(not output("git", "-C", str(root), "status", "--porcelain=v1", "--untracked-files=all"), "dirty checkout")
    result.update({"schema": SCHEMA, "cell": cell, "subject_sha256": digest(evidence / "subject.json"),
                   "logs_sha256": {name: digest(evidence / (name + ".log")) for name in logs},
                   "source_unchanged": True, "owned_fixture_leftovers": leftovers,
                   "scope": "existing applicable suite only; not full HUG-008, v1 cutover or power-loss qualification"})
    (evidence / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("identify", "verify"))
    parser.add_argument("--cell", choices=CELLS, required=True)
    args = parser.parse_args()
    evidence = Path(os.environ["EVIDENCE_DIR"])
    try:
        {"identify": identify, "verify": verify}[args.mode](
            Path.cwd(), evidence, Path(os.environ["NATIVE_FIXTURES"]), args.cell)
        return 0
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        diagnostic = {"mode": args.mode, "cell": args.cell, "error": type(error).__name__, "reason": str(error)[:2048]}
        (evidence / (args.mode + "-failure.json")).write_text(json.dumps(diagnostic, indent=2) + "\n")
        print(json.dumps(diagnostic), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
