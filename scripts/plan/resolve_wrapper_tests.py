#!/usr/bin/env python3
"""Resolve the 52 pinned legacy wrapper declarations; never execute shell/Cargo.
This is a bounded diagnostic for SC01, NOT a general shell interpreter or a WP gate.
Run with the historical source, acquisition manifest and retained Cargo basis.
"""
from __future__ import annotations
import argparse
from collections import Counter
import hashlib
import json
import os
import stat
from pathlib import Path
import re
import shlex
import tomllib

SOURCE = '96ddcdade3d56d7c2cf4a01c294a56c3aaa4e58c'
INVENTORY_SHA = 'b731c476042b11e49087799c69b3d6a3bbe771e172972e09a0d8feb2aec34ace'
BASIS_SHA = 'be3834ac208692e4348a2204246f29225701551d02168ac0208ec1436c5fafe8'
MAX_INPUT = 16 * 1024 * 1024
MAX_SOURCE_FILE = 8 * 1024 * 1024
MAX_SOURCE_TOTAL = 256 * 1024 * 1024
VARIABLE = re.compile(r'\$\{([A-Z_0-9]+)\}|\$([A-Z_0-9]+)')


class Invalid(ValueError):
    """A fixed diagnostic code; never echo source paths or payloads in errors."""


def read_regular(root: Path, relative: str, limit: int) -> bytes:
    """Bounded POSIX read; reject symlinks and special files, including FIFOs."""
    parts = relative.split('/')
    if (not relative or len(relative.encode()) > 4096 or len(parts) > 64
            or any(p in ('', '.', '..') for p in parts) or '\\' in relative or '\0' in relative):
        raise Invalid('INPUT_PATH_INVALID')
    if not all(hasattr(os, k) for k in ('O_NOFOLLOW', 'O_DIRECTORY', 'O_NONBLOCK')):
        raise Invalid('UNSUPPORTED_PLATFORM')
    fds = []
    try:
        fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        fds.append(fd)
        for part in parts[:-1]:
            fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            fds.append(fd)
        fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        fds.append(fd)
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
            raise Invalid('INPUT_TYPE_OR_LIMIT')
        chunks = []; size = 0
        while True:
            block = os.read(fd, min(65536, limit + 1 - size))
            if not block:
                break
            size += len(block)
            if size > limit:
                raise Invalid('INPUT_LIMIT')
            chunks.append(block)
        after = os.fstat(fd)
        fields = lambda x: (x.st_size, x.st_mtime_ns, x.st_ctime_ns)
        if size != before.st_size or fields(before) != fields(after):
            raise Invalid('INPUT_CHANGED')
        return b''.join(chunks)
    except OSError as exc:
        raise Invalid('INPUT_UNAVAILABLE') from exc
    finally:
        for fd in reversed(fds):
            os.close(fd)


def checked_json(path: Path, expected: str) -> dict:
    data = read_regular(path.parent, path.name, MAX_INPUT)
    if hashlib.sha256(data).hexdigest() != expected:
        raise Invalid('INPUT_IDENTITY')
    return json.loads(data)


def declarations(text: str) -> tuple[dict | None, dict]:
    """Resolve only literal assignments and final argv present in these wrappers.
    Descriptive strings, comments and --list probes are not invocation evidence.
    Unknown expansions remain unknown and are refused for an asserted invocation.
    """
    variables: dict[str, str] = {}
    assignments: dict[str, dict] = {}
    calls: list[dict] = []
    for number, line in enumerate(text.splitlines(), 1):
        statement = line.strip()
        if not statement or statement.startswith('#'):
            continue
        match = re.fullmatch(r'(?:export )?([A-Z_0-9]+)=(.*)', statement)
        if match and '$(' not in match[2] and '`' not in match[2]:
            try:
                words = shlex.split(match[2], comments=True)
            except ValueError:
                words = []
            if len(words) == 1:
                value = VARIABLE.sub(lambda m: variables.get(m[1] or m[2], m[0]), words[0])
                variables[match[1]] = value
                assignments[match[1]] = {'line': number, 'resolved_value': value}
        if '--list' in statement or '$(' in statement or '`' in statement:
            continue
        try:
            tokens = shlex.split(statement.removesuffix('\\').strip(), comments=True)
        except ValueError:
            continue
        positions = [i for i in range(len(tokens)-1) if tokens[i:i+2] == ['cargo', 'test']]
        if not positions:
            continue
        position = positions[-1]
        # Accepted forms: direct cargo invocation, or check DESCRIPTION cargo ...
        if position != 0 and not (tokens[0] == 'check' and position == 2):
            continue
        argv = [VARIABLE.sub(lambda m: variables.get(m[1] or m[2], m[0]), t) for t in tokens[position:]]
        if any('$' in token or '`' in token for token in argv):
            raise Invalid('UNRESOLVED_INVOCATION')
        calls.append({'line': number, 'argv': argv, 'source_line': line})
    if len(calls) > 1:
        raise Invalid('MULTIPLE_FINAL_INVOCATIONS')
    return (calls[0] if calls else None), assignments


def resolve(source: Path, inventory_path: Path, basis_path: Path) -> dict:
    inventory = checked_json(inventory_path, INVENTORY_SHA)
    basis = checked_json(basis_path, BASIS_SHA)
    if inventory['source_commit'] != SOURCE or basis['source_commit'] != SOURCE:
        raise Invalid('SUBJECT_IDENTITY')
    records = {row['path']: row for row in inventory['files']}
    read_files: set[str] = set()
    total_read = 0
    def read(path: str) -> bytes:
        nonlocal total_read
        if path not in records:
            raise Invalid('UNTRACKED_REFERENCE')
        record = records[path]
        data = read_regular(source, path, MAX_SOURCE_FILE)
        total_read += len(data)
        if total_read > MAX_SOURCE_TOTAL:
            raise Invalid('SOURCE_TOTAL_LIMIT')
        if hashlib.sha256(data).hexdigest() != record['sha256']:
            raise Invalid('SOURCE_IDENTITY')
        read_files.add(path)
        return data
    package_names = {p['id']: p['name'] for p in basis['cargo_packages']}
    manifests = {m['package']: m for m in basis['cargo_manifests'] if m['in_workspace']}
    # Cross-check every workspace package name against its actual manifest bytes.
    for name, entry in manifests.items():
        if tomllib.loads(read(entry['path']).decode())['package']['name'] != name:
            raise Invalid('PACKAGE_NAME_MISMATCH')
    targets: dict[tuple[str, str], str] = {}
    for row in basis['workspace_targets']:
        target = row['target']
        if target['kind'] == ['test']:
            key = (package_names[row['package_id']], target['name'])
            if key in targets:
                raise Invalid('AMBIGUOUS_TEST_TARGET')
            targets[key] = target['src_path'].removeprefix('$REPO/')
    expected = sorted(p for p in records if re.fullmatch(r'tests/acceptance/[^/]+/run\.sh', p))
    if len(expected) != 52:
        raise Invalid('WRAPPER_DENOMINATOR')
    entries = []
    for path in expected:
        raw = read(path)
        call, assignments = declarations(raw.decode())
        package = call['argv'][call['argv'].index('-p')+1] if call and '-p' in call['argv'] else None
        target = call['argv'][call['argv'].index('--test')+1] if call and '--test' in call['argv'] else None
        resolved = targets.get((package, target))
        if call is None:
            if path != 'tests/acceptance/wp-c1/run.sh':
                raise Invalid('INVOCATION_UNRESOLVED')
            status = 'document_checks_only'
        elif '--workspace' in call['argv']:
            status = 'workspace_suite_not_one_test'
        elif target is None:
            status = 'package_suite_not_one_test'
        elif package not in manifests:
            status = 'package_absent'
        elif resolved is None:
            status = 'test_target_absent'
        else:
            status = 'test_target_resolved'
            read(resolved)
        entry = {
            'wrapper': path, 'wrapper_sha256': records[path]['sha256'],
            'declared_invocation': call, 'package': package, 'named_test': target,
            'resolution': status, 'test_source': resolved,
            'test_source_sha256': records[resolved]['sha256'] if resolved else None,
            'declared_acceptance_file': assignments.get('ACCEPTANCE_FILE'),
            'tests_executed': False, 'semantic_equivalence_claimed': False,
        }
        declaration = entry['declared_acceptance_file']
        if declaration:
            value = declaration['resolved_value']
            declaration['in_pinned_inventory'] = value in records
            declaration['matches_cargo_target'] = value == resolved if resolved else None
        if status in ('package_absent', 'test_target_absent'):
            entry['disposition'] = 'Unavailable legacy target; route to HUG-004. Do not restore retired services or substitute a similarly named oracle.'
        else:
            entry['disposition'] = 'Inventory linkage only; does not qualify executing the wrapper or the referenced test.'
        entries.append(entry)
    return {'schema_version': '1.0', 'source_commit': SOURCE,
            'scope': 'SC01 legacy wrapper target resolution, not runtime execution',
            'counts': dict(Counter(e['resolution'] for e in entries)),
            'wrapper_count': len(entries), 'source_files_hash_verified': len(read_files),
            'entries': entries, 'whole_wp_ready': False, 'independent_review': False,
            'limitations': ['Fixed historical Cargo observation, not a new cargo metadata run.',
                           'Restricted literal/variable declaration reader, not arbitrary shell evaluation.',
                           'Source and input artifacts must be quiescent and trusted; no hostile-writer claim.',
                           'Resolved targets can still contain legacy semantics, mocks, skips or external calls.']}


def write_report(source: Path, output: Path, result: dict) -> None:
    source = source.resolve(strict=True)
    destination = output.parent.resolve(strict=True) / output.name
    if source == destination or source in destination.parents or destination in source.parents:
        raise Invalid('OUTPUT_INSIDE_SOURCE')
    try:
        with destination.open('x', encoding='utf-8') as stream:
            stream.write(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2)+'\n')
    except FileExistsError as exc:
        raise Invalid('OUTPUT_EXISTS') from exc


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--source', type=Path, required=True)
    ap.add_argument('--inventory', type=Path, required=True)
    ap.add_argument('--basis', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    args = ap.parse_args()
    try:
        if args.source.is_symlink():
            raise Invalid('INPUT_PATH_INVALID')
        result = resolve(args.source, args.inventory, args.basis)
        write_report(args.source, args.out, result)
    except (OSError, ValueError, KeyError, TypeError, IndexError, RecursionError) as exc:
        code = str(exc) if isinstance(exc, Invalid) else 'SETUP_OR_SCHEMA_INVALID'
        print(json.dumps({'valid': False, 'error': code, 'whole_wp_ready': False,
                          'independent_review': False}, sort_keys=True))
        return 2
    print(json.dumps({'valid': True, 'counts': result['counts'],
                      'wrapper_count': result['wrapper_count'],
                      'source_files_hash_verified': result['source_files_hash_verified'],
                      'whole_wp_ready': False}, sort_keys=True))
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
