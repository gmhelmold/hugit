#!/usr/bin/env python3
"""Verify retained HUG-001 acquisition. No subprocess, network, or product execution.
Requires POSIX descriptor-relative, no-follow opens. A passing report is not WP acceptance.
The caller supplies a trusted source-tree digest and retained Git index/tree enumerations.
"""
from __future__ import annotations
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
from collections import Counter

MAX_JSON = 16 * 1024 * 1024
MAX_ENUM = 16 * 1024 * 1024
MAX_RECORDS = 50000
MAX_FILE = 8 * 1024 * 1024
MAX_TOTAL = 256 * 1024 * 1024
MAX_PATH = 4096
MAX_DEPTH = 64
CHUNK = 65536
CLASSES = frozenset(('agent_instruction_or_hook', 'cargo_manifest_or_lock', 'ci_workflow',
    'configuration_or_other_text', 'documentation_or_presentation', 'fixture_or_conformance',
    'historical_engine_snapshot', 'plan_normative_or_generated', 'plan_tooling', 'rust_source',
    'script_or_adapter', 'test_source_or_data'))

class Rejected(ValueError):
    pass

def require(ok: bool, code: str) -> None:
    if not ok:
        raise Rejected(code)

def hex40(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{40}', value) is not None

def path_bytes(value: str) -> bytes:
    require(isinstance(value, str), 'PATH_TYPE')
    raw = value.encode('utf-8', errors='strict')
    parts = value.split('/')
    require(0 < len(raw) <= MAX_PATH and len(parts) <= MAX_DEPTH, 'PATH_BUDGET')
    require(not any(p in ('', '.', '..') for p in parts), 'PATH_TRAVERSAL')
    require(not any(c in value for c in ('\0', '\\', ':')), 'PATH_UNSUPPORTED')
    return raw

def read_input(path: Path, limit: int) -> bytes:
    # Explicit input paths belong to the caller; source paths are opened beneath a dirfd.
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_size <= limit, 'INPUT_BUDGET_OR_TYPE')
        data = stream.read(limit + 1)
        require(len(data) <= limit, 'INPUT_BUDGET')
        return data

def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'DUPLICATE_JSON_KEY')
        result[key] = value
    return result

def load_json(path: Path):
    data = read_input(path, MAX_JSON)
    value = json.loads(data, object_pairs_hook=unique_object,
        parse_constant=lambda _: (_ for _ in ()).throw(Rejected('NONFINITE_JSON')))
    require(isinstance(value, dict), 'DOCUMENT_TYPE')
    return value, data

def parse_enumeration(raw: bytes, index: bool) -> dict[bytes, tuple[str, str]]:
    require(not raw or raw.endswith(b'\0'), 'ENUM_TERMINATOR')
    require(raw.count(b'\0') <= MAX_RECORDS, 'RECORD_BUDGET')
    result = {}
    for row in raw.split(b'\0')[:-1]:
        head, sep, path = row.partition(b'\t')
        require(bool(sep), 'ENUM_FORMAT')
        fields = head.decode('ascii').split(' ')
        require(len(fields) == 3, 'ENUM_FORMAT')
        mode, oid = (fields[0], fields[1]) if index else (fields[0], fields[2])
        require(fields[2] == '0' if index else fields[1] == 'blob', 'ENUM_STAGE_OR_KIND')
        require(mode in ('100644', '100755') and hex40(oid), 'OBJECT_UNSUPPORTED')
        require(path_bytes(path.decode('utf-8')) == path, 'PATH_ENCODING')
        require(path not in result, 'DUPLICATE_PATH')
        result[path] = (mode, oid)
    return result

def git_tree_oid(entries: dict[bytes, tuple[str, str]]) -> str:
    root = {}
    # Git ls-tree order is retained. Each directory is checked for canonical ordering.
    for path, blob in entries.items():
        parts = path.split(b'/'); node = root
        for part in parts[:-1]:
            node = node.setdefault(part, {})
            require(isinstance(node, dict), 'FILE_DIRECTORY_COLLISION')
        require(parts[-1] not in node, 'FILE_DIRECTORY_COLLISION')
        node[parts[-1]] = blob
    def fold(node):
        body = bytearray(); previous = None
        for name, value in node.items():
            directory = isinstance(value, dict)
            key = name + (b'/' if directory else b'')
            require(previous is None or previous < key, 'TREE_ORDER')
            previous = key
            mode, oid = ('40000', fold(value)) if directory else value
            body.extend(mode.encode() + b' ' + name + b'\0' + bytes.fromhex(oid))
        return hashlib.sha1(b'tree ' + str(len(body)).encode() + b'\0' + body).hexdigest()
    return fold(root)

def source_fd(root_fd: int, path: str) -> int:
    current = os.dup(root_fd)
    try:
        parts = path.split('/')
        for part in parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=current)
            os.close(current); current = child
        return os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=current)
    finally:
        os.close(current)

def enumerate_source(root_fd: int) -> set[bytes]:
    found = set(); visited = 0
    def walk(fd, prefix, depth):
        nonlocal visited
        require(depth <= MAX_DEPTH, 'DIRECTORY_DEPTH')
        with os.scandir(fd) as rows:
            for row in rows:
                if not prefix and row.name == '.git':
                    continue
                visited += 1
                require(visited <= MAX_RECORDS * 2, 'DIRECTORY_BUDGET')
                path = prefix + row.name; raw = path_bytes(path)
                require(not row.is_symlink(), 'SOURCE_SYMLINK')
                if row.is_dir(follow_symlinks=False):
                    child = os.open(row.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                    try: walk(child, path + '/', depth + 1)
                    finally: os.close(child)
                else:
                    require(row.is_file(follow_symlinks=False), 'SOURCE_SPECIAL_FILE')
                    found.add(raw)
                    require(len(found) <= MAX_RECORDS, 'RECORD_BUDGET')
    walk(root_fd, '', 0)
    return found

def verify(source: Path, inventory: Path, bindings: Path, plan: Path,
           index: Path, tree: Path, expected_tree: str) -> dict:
    require(hex40(expected_tree), 'EXPECTED_TREE_INVALID')
    require(all(hasattr(os, k) for k in ('O_NOFOLLOW', 'O_DIRECTORY', 'O_NONBLOCK')),
            'PLATFORM_UNSUPPORTED')
    inv, _ = load_json(inventory); bind, _ = load_json(bindings); spec, plan_bytes = load_json(plan)
    indexed = parse_enumeration(read_input(index, MAX_ENUM), True)
    treed = parse_enumeration(read_input(tree, MAX_ENUM), False)
    require(indexed == treed, 'INDEX_TREE_MISMATCH')
    require(git_tree_oid(treed) == expected_tree, 'SOURCE_TREE_MISMATCH')
    require(inv['source_tree'] == bind['source_tree'] == expected_tree, 'MANIFEST_TREE_MISMATCH')
    require(hex40(inv['source_commit']) and inv['source_commit'] == bind['source_commit'], 'SUBJECT_MISMATCH')
    digest = hashlib.sha256(plan_bytes).hexdigest()
    require(inv['plan_file_sha256'] == bind['plan_file_sha256'] == digest, 'PLAN_MISMATCH')
    files = inv['files']; require(isinstance(files, list) and len(files) <= MAX_RECORDS, 'RECORD_BUDGET')
    by_path = {}; total = 0; classes = Counter(); declared_reviews = 0
    for item in files:
        path = path_bytes(item['path'])
        require(path not in by_path, 'DUPLICATE_MANIFEST_PATH')
        require(base64.b64decode(item['path_bytes_base64'], validate=True) == path, 'PATH_BYTES_MISMATCH')
        require(item['classification'] in CLASSES, 'UNKNOWN_CLASS')
        require(isinstance(item['owner_lane'], str) and bool(item['owner_lane']) and item['git_kind'] == 'blob', 'MISSING_OWNER_OR_KIND')
        review = item['semantic_review']
        require(review['status'] in ('not_reviewed', 'partial', 'reviewed'), 'REVIEW_STATUS')
        if review['status'] == 'not_reviewed':
            require(not review['ranges'] and review['reviewer'] is None, 'REVIEW_CONTRADICTION')
        else:
            declared_reviews += 1
            require(review['reviewer'] and review['ranges'], 'REVIEW_WITHOUT_LOCATOR')
        size = item['size_bytes']
        require(type(size) is int and 0 <= size <= MAX_FILE, 'FILE_BUDGET')
        total += size; require(total <= MAX_TOTAL, 'TOTAL_BUDGET')
        require(treed.get(path) == (item['git_mode'], item['git_oid']), 'MANIFEST_OBJECT_MISMATCH')
        classes[item['classification']] += 1; by_path[path] = item
    require(set(by_path) == set(treed) and inv['file_count'] == len(files), 'MANIFEST_SET_MISMATCH')
    require(dict(classes) == inv['classes'], 'CLASS_COUNT_MISMATCH')
    fd = os.open(source, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    bytes_read = 0
    try:
        require(enumerate_source(fd) == set(treed), 'SOURCE_SET_MISMATCH')
        for raw, item in by_path.items():
            with os.fdopen(source_fd(fd, item['path']), 'rb') as stream:
                before = os.fstat(stream.fileno())
                require(stat.S_ISREG(before.st_mode) and before.st_size == item['size_bytes'], 'FILE_TYPE_OR_SIZE')
                require(('100755' if before.st_mode & 0o111 else '100644') == item['git_mode'], 'MODE_MISMATCH')
                sha = hashlib.sha256(); oid = hashlib.sha1(b'blob ' + str(before.st_size).encode() + b'\0')
                n = 0
                while True:
                    chunk = stream.read(min(CHUNK, before.st_size - n + 1))
                    if not chunk: break
                    n += len(chunk); require(n <= before.st_size, 'SOURCE_CHANGED')
                    sha.update(chunk); oid.update(chunk)
                after = os.fstat(stream.fileno())
                require(n == before.st_size and (before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                    (after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'SOURCE_CHANGED')
                require(sha.hexdigest() == item['sha256'] and oid.hexdigest() == item['git_oid'], 'BLOB_MISMATCH')
                bytes_read += n
    finally:
        os.close(fd)
    tasks = {}; expected = {}
    require(isinstance(spec['tasks'], list) and len(spec['tasks']) <= MAX_RECORDS, 'TASK_BUDGET')
    for task in spec['tasks']:
        require(task['id'] not in tasks, 'DUPLICATE_TASK'); tasks[task['id']] = task
        for path in task['write_set']:
            path_bytes(path); key = (task['id'], path)
            require(key not in expected, 'DUPLICATE_WRITE_SET'); expected[key] = task
            require(len(expected) <= MAX_RECORDS, 'BINDING_BUDGET')
    require(isinstance(bind['bindings'], list) and len(bind['bindings']) <= MAX_RECORDS, 'BINDING_BUDGET')
    seen = set()
    for item in bind['bindings']:
        key = (item['wp_id'], item['path']); raw = path_bytes(item['path'])
        require(key in expected and key not in seen, 'BINDING_SET_MISMATCH'); seen.add(key)
        actual = by_path.get(raw); exists = actual is not None
        require(type(item['exists_at_subject']) is bool and item['exists_at_subject'] == exists, 'EXISTENCE_MISMATCH')
        require(item['existing_git_oid'] == (actual['git_oid'] if exists else None), 'BINDING_OID_MISMATCH')
        require(item['owner_lane'] == expected[key]['owner_lane'], 'BINDING_OWNER_MISMATCH')
        require(item['operation'] == ('edit_existing' if exists else 'create_new'), 'OPERATION_MISMATCH')
        prefixes = item['path'].split('/')[:-1]; prefix = ''; conflicts = []
        for part in prefixes:
            prefix = prefix + '/' + part if prefix else part
            if prefix.encode() in by_path: conflicts.append(prefix)
        require(conflicts == item['ancestor_file_conflicts'] and not conflicts, 'ANCESTOR_FILE_CONFLICT')
    require(seen == set(expected) and bind['binding_count'] == len(seen), 'BINDING_SET_MISMATCH')
    return {'schema': 'hug001-acquisition-check/1', 'valid': True, 'expected_tree': expected_tree,
        'source_commit_claim': inv['source_commit'], 'commit_object_verified_here': False,
        'files_verified': len(files), 'bytes_hashed': bytes_read, 'bindings_verified': len(seen),
        'declared_semantic_reviews': declared_reviews, 'semantic_review_accepted': False,
        'whole_wp_ready': False, 'consumer_admission': False, 'runtime_executed': False,
        'limits': {'json_bytes_per_input': MAX_JSON, 'records': MAX_RECORDS, 'file_bytes': MAX_FILE,
                   'source_bytes_total': MAX_TOTAL, 'chunk_bytes': CHUNK, 'path_bytes': MAX_PATH},
        'limitations': ['Checks retained acquisition, not fresh Cargo resolution or full transitive coverage.',
            'POSIX-only verifier; source tree must be quiescent; hostile same-user writers are outside this check.',
            'Tree/object identities and declared metadata do not establish independent semantic review.']}

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'inventory', 'bindings', 'plan', 'index', 'tree'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--expected-tree', required=True)
    args = vars(parser.parse_args())
    try:
        report = verify(**args)
    except (Rejected, OSError, ValueError, KeyError, TypeError, RecursionError, AttributeError) as error:
        # No paths, payloads, or arbitrary exception text in bounded machine diagnostics.
        code = str(error)[:80] if isinstance(error, Rejected) else type(error).__name__
        report = {'valid': False, 'error': code, 'whole_wp_ready': False, 'consumer_admission': False}
    print(json.dumps(report, ensure_ascii=True, sort_keys=True))
    return 0 if report['valid'] else 1

if __name__ == '__main__':
    raise SystemExit(main())
