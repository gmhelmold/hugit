#!/usr/bin/env python3
"""Check the bounded HUG-001 annotation, not semantic truth or WP acceptance.

No network, subprocesses, imported source code, or writes to the inspected tree.
Requires POSIX no-follow descriptor opens and a quiescent source snapshot.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys

MAX_JSON = 16 * 1024 * 1024
MAX_BLOB = 8 * 1024 * 1024
MAX_SOURCE = 256 * 1024 * 1024
MAX_ANCHORS = 256
FLAGS = ('whole_wp_ready', 'independent_review', 'product_tests_executed',
         'compiler_call_graph', 'consumer_admission')

class Invalid(ValueError):
    pass

def require(ok, code):
    if not ok:
        raise Invalid(code)

def digest(raw):
    return hashlib.sha256(raw).hexdigest()

def oid(raw):
    return hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest()

def parts(path):
    require(isinstance(path, str) and 0 < len(path.encode()) <= 4096, 'PATH_FORMAT')
    bits = path.split('/')
    require(len(bits) <= 64 and all(b not in ('', '.', '..') for b in bits)
            and '\\' not in path and '\0' not in path, 'PATH_FORMAT')
    return bits

def read_under(root, path, limit):
    """Never follow a leaf or intermediate symlink; refuse special files."""
    require(hasattr(os, 'O_NOFOLLOW') and os.open in os.supports_dir_fd, 'UNSUPPORTED_PLATFORM')
    bits = parts(path)
    fds = []
    try:
        fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        fds.append(fd)
        for part in bits[:-1]:
            fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            fds.append(fd)
        leaf = os.open(bits[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        fds.append(leaf)
        before = os.fstat(leaf)
        require(stat.S_ISREG(before.st_mode) and before.st_size <= limit, 'FILE_TYPE_OR_LIMIT')
        chunks = []; size = 0
        while True:
            chunk = os.read(leaf, min(65536, limit + 1 - size))
            if not chunk:
                break
            chunks.append(chunk); size += len(chunk)
            require(size <= limit, 'FILE_LIMIT')
        after = os.fstat(leaf)
        require((before.st_size, before.st_mtime_ns) == (after.st_size, after.st_mtime_ns)
                and size == before.st_size, 'SOURCE_CHANGED')
        return b''.join(chunks)
    except OSError as exc:
        raise Invalid('FILE_UNAVAILABLE') from exc
    finally:
        for fd in reversed(fds):
            os.close(fd)

def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, 'JSON_DUPLICATE_KEY')
        result[key] = value
    return result

def load(root, path):
    raw = read_under(root, path, MAX_JSON)
    try:
        return json.loads(raw, object_pairs_hook=pairs), raw
    except (UnicodeError, json.JSONDecodeError, RecursionError) as exc:
        raise Invalid('JSON_INVALID') from exc

def keyed(rows, key):
    require(isinstance(rows, list) and len(rows) <= 50000, 'ROW_LIMIT')
    result = {}
    for row in rows:
        name = row[key]
        require(isinstance(name, str) and name not in result, 'DUPLICATE_ID')
        result[name] = row
    return result

def verify(doc, catalog, inventory, basis, source):
    require(doc['schema_version'] == '1.0', 'SCHEMA_VERSION')
    require(all(doc['authority'].get(k) is False for k in FLAGS), 'AUTHORITY_ESCALATION')
    for key in ('source_commit', 'source_tree'):
        require(doc[key] == catalog[key] == inventory[key] == basis[key], 'SOURCE_IDENTITY')
    files = keyed(inventory['files'], 'path')
    require(len(files) == inventory['file_count'], 'SOURCE_COUNT')
    expected_routes = {r[0]: r for r in catalog['routes']}
    require(len(expected_routes) == len(catalog['routes']), 'CATALOG_DUPLICATE')
    routes = keyed(doc['routes'], 'id')
    require(set(routes) == set(expected_routes), 'ROUTE_SET')
    tests = {r[0]: r for r in catalog['source_tests']}
    profiles = doc['profiles']
    require(isinstance(profiles, dict) and 0 < len(profiles) <= 128, 'PROFILE_SET')
    for key, row in routes.items():
        require(row['test_refs'] == expected_routes[key][5], 'TEST_BINDING')
        require(row['profiles'] and len(row['profiles']) == len(set(row['profiles']))
                and set(row['profiles']) <= set(profiles), 'PROFILE_BINDING')
        for ref in row['test_refs']:
            require(ref in tests and tests[ref][5] != 'fixture_only_excluded', 'TEST_AUTHORITY')
    expected_variants = {(v[0],v[1]): v[6] for v in catalog['variants']}
    actual_variants = {(v['command'],v['selector']): v['boundary'] for v in doc['variants']}
    require(len(actual_variants) == len(doc['variants'])
            and actual_variants == expected_variants, 'VARIANT_SET')
    # Critical distinctions must not vanish while generic integrity checks pass.
    guards = {'cli:check/run': {'check','resolve'}, 'cli:capture': {'capture','drain'},
              'cli:fleet': {'projection'}, 'cli:watch': {'projection'},
              'cli:pr/land': {'dispatch_residue'}, 'cli:land/queue': {'queue_sim'},
              'cli:dock/land': {'dock_sim'}, 'mcp:cost-attest': {'mcp_http'},
              'mcp:liveness-probe': {'mcp_http'}, 'mcp:capture': {'mcp_child'},
              'mcp:land-status': {'mcp_child'}}
    for route, required in guards.items():
        require(required <= set(routes[route]['profiles']), 'CRITICAL_BOUNDARY_MISSING')
    required_effects = {'check': {'cache_write','check_subprocess'},
                        'projection': {'projection_status_write'},
                        'mcp_child': {'configured_subprocess'},
                        'mcp_http': {'legacy_network_http'},
                        'queue_sim': {'simulated_validation'},
                        'dock_sim': {'simulated_validation'},
                        'resolve': {'runtime_migration_write'}}
    for name, required in required_effects.items():
        require(required <= set(profiles[name]['effects']), 'EFFECT_REMOVED')
    anchors = []
    for profile in profiles.values():
        require(profile['effects'] and profile['condition'] and profile['call_path']
                and re.fullmatch(r'HUG-\d{3}',profile['destination_wp']), 'PROFILE_FORMAT')
        require(profile['anchors'], 'ANCHOR_MISSING')
        anchors.extend(profile['anchors'])
    anchors.extend(doc['generator_boundary']['anchors'])
    require(len(anchors) <= MAX_ANCHORS, 'ANCHOR_LIMIT')
    cache = {}; total = 0
    for path, first, last, expected in anchors:
        require(path in files and type(first) is int and type(last) is int
                and 1 <= first <= last, 'ANCHOR_RANGE')
        if path not in cache:
            data = read_under(source, path, MAX_BLOB)
            total += len(data); require(total <= MAX_SOURCE, 'SOURCE_BYTE_LIMIT')
            require(digest(data) == files[path]['sha256'], 'SOURCE_DIGEST')
            cache[path] = data.splitlines(keepends=True)
        lines = cache[path]
        require(last <= len(lines) and digest(b''.join(lines[first-1:last])) == expected, 'SPAN_DIGEST')
    # Every workspace member and binary must remain in the denominator.
    libs = keyed(doc['workspace_libraries'], 'name')
    manifests = {m['package']: m for m in basis['cargo_manifests'] if m['in_workspace']}
    require(set(libs) == set(manifests) and len(libs) == basis['workspace_member_count'], 'WORKSPACE_SET')
    for name, lib in libs.items():
        require(lib['manifest'] == manifests[name]['path'] and lib['manifest'] in files
                and lib['features'] == manifests[name]['features'] and lib['role'] and lib['note'], 'LIBRARY_CLASSIFICATION')
    bins = keyed(doc['workspace_binaries'], 'name')
    expected_bins = {t['target']['name']: t['target']['src_path'].removeprefix('$REPO/')
                     for t in basis['workspace_targets'] if t['target']['kind'] == ['bin']}
    require(set(bins) == set(expected_bins), 'BINARY_SET')
    require(all(b['path'] == expected_bins[name] and b['path'] in files
                for name,b in bins.items()), 'BINARY_PATH')
    return {'schema_version':'1.0','valid':True,'scope':doc['scope'],
            'source_commit':doc['source_commit'],'source_tree':doc['source_tree'],
            'routes':len(routes),'variants':len(actual_variants),'profiles':len(profiles),
            'source_files_verified':len(cache),'source_bytes_verified':total,
            'workspace_libraries':len(libs),'workspace_binaries':len(bins),
            'authority':{k:False for k in FLAGS},
            'limits':['Checks identity, association and explicit distinctions; not semantic truth of every annotation.',
                      'No product test or new Cargo resolution executed by this verifier.']}

def main():
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument('--root', type=Path, default=Path('.'))
    a.add_argument('--source', type=Path, required=True)
    a.add_argument('--basis', type=Path, required=True)
    a.add_argument('--report', type=Path)
    args = a.parse_args()
    try:
        doc,_ = load(args.root,'docs/audit/surface-contracts.json')
        cat,raw = load(args.root,'docs/audit/reachability.json')
        inv,_ = load(args.root,'docs/audit/source-inventory.json')
        base,braw = load(args.basis.parent,args.basis.name)
        require(oid(raw) == doc['catalog_blob_oid'], 'CATALOG_IDENTITY')
        mb=cat['mechanical_basis']
        require(oid(braw)==mb['git_blob_oid'] and digest(braw)==mb['sha256'], 'BASIS_IDENTITY')
        report = verify(doc,cat,inv,base,args.source)
    except (Invalid, KeyError, TypeError, ValueError, IndexError, RecursionError) as exc:
        report={'valid':False,'error':str(exc) if isinstance(exc,Invalid) else 'SCHEMA_INVALID',
                'authority':{k:False for k in FLAGS}}
    text=json.dumps(report,sort_keys=True,indent=2)+'\n'
    if args.report:
        args.report.write_text(text)
    print(text,end='')
    return 0 if report['valid'] else 1

if __name__ == '__main__':
    sys.exit(main())
