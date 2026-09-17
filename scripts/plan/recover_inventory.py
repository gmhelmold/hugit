#!/usr/bin/env python3
"""Rebuild HUG-001 derived manifests from pinned source and retained observations.

No Git, Cargo, network, or inspected code is executed. The old annotated catalog
is a *curation input*: only human-authored selections/statements are retained.
File records, bindings, counts and authority fields are recomputed, never copied
from the manifests being recovered. Requires a quiescent POSIX source snapshot.
"""
from __future__ import annotations
import argparse
import base64
from collections import Counter, defaultdict
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import tomllib

SOURCE = '96ddcdade3d56d7c2cf4a01c294a56c3aaa4e58c'
TREE = '0e74dd8f693b0d660c3ce87def679e8a23b1d1dc'
BASIS_OID = 'd41be8a776eee25ca7278e86dfd0c1c48ba3724c'
BASIS_SHA = 'be3834ac208692e4348a2204246f29225701551d02168ac0208ec1436c5fafe8'
CURATION_OID = '8a94bdcfd8ac1d6f7431189b9f6176b513a29c75'
CURATION_SHA = '73d395dfae5ae9956a9a90d0e2d403a5490b5e1afbdb3e208adc0a851c7106c2'
PLAN_SHA = '5234276523649da9b35abe578ed6cbf2ce25c43651bfc0f6ab2d549b37e01409'
PLAN_PATH = 'docs/plan/standalone/v3/backlog.json'
OUTPUTS = ('source-inventory.json', 'reachability.json', 'path-bindings.json')
MAX_INPUT = 16 * 1024 * 1024
MAX_FILE = 8 * 1024 * 1024
MAX_SOURCE = 256 * 1024 * 1024
MAX_ROWS = 50000
CURATED = ('source_identity', 'source_files', 'route_columns', 'routes',
           'variant_columns', 'variants', 'test_columns', 'source_tests',
           'helper_columns', 'helpers', 'locator_corrections',
           'historical_observations', 'limitations', 'retained_inputs',
           'remaining_coverage', 'closure')

class Invalid(ValueError):
    pass

def require(ok, code):
    if not ok:
        raise Invalid(code)

def sha(raw):
    return hashlib.sha256(raw).hexdigest()

def oid(raw, kind=b'blob'):
    return hashlib.sha1(kind+b' '+str(len(raw)).encode()+b'\0'+raw).hexdigest()

def encoded(obj):
    return (json.dumps(obj, ensure_ascii=True, sort_keys=True, indent=2)+'\n').encode()

def pairs(rows):
    result = {}
    for key, value in rows:
        require(key not in result, 'JSON_DUPLICATE_KEY'); result[key] = value
    return result

def read(root, path, limit=MAX_INPUT):
    require(isinstance(path, str) and 0 < len(path.encode()) <= 4096, 'PATH_INVALID')
    bits = path.split('/')
    require(len(bits) <= 64 and all(x not in ('', '.', '..') for x in bits)
            and '\\' not in path and '\0' not in path, 'PATH_INVALID')
    require(hasattr(os, 'O_NOFOLLOW') and os.open in os.supports_dir_fd, 'PLATFORM_UNSUPPORTED')
    fds = []
    try:
        fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW); fds.append(fd)
        for bit in bits[:-1]:
            fd = os.open(bit, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd); fds.append(fd)
        fd = os.open(bits[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd); fds.append(fd)
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and before.st_size <= limit, 'INPUT_TYPE_OR_LIMIT')
        chunks = []; size = 0
        while True:
            block = os.read(fd, min(65536, limit + 1 - size))
            if not block: break
            size += len(block); require(size <= limit, 'INPUT_LIMIT'); chunks.append(block)
        after = os.fstat(fd)
        require(size == before.st_size and (before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                == (after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'SOURCE_CHANGED')
        return b''.join(chunks)
    except OSError as exc:
        raise Invalid('SETUP_INPUT_UNAVAILABLE') from exc
    finally:
        for fd in reversed(fds): os.close(fd)

def read_path(path):
    return read(path.parent, path.name)

def parse(raw):
    return json.loads(raw, object_pairs_hook=pairs)

def tree_records(tree_raw, index_raw):
    require(len(tree_raw) <= MAX_INPUT and len(index_raw) <= MAX_INPUT, 'INPUT_LIMIT')
    def decode(raw, index):
        require(raw.endswith(b'\0'), 'ENUMERATION_FORMAT')
        result = {}
        for row in raw[:-1].split(b'\0'):
            attrs, name = row.split(b'\t', 1); a, b, c = attrs.split(b' ')
            mode, identity = a.decode(), (b if index else c).decode()
            require((c == b'0' if index else b == b'blob') and mode in ('100644', '100755'), 'ENUMERATION_KIND')
            require(re.fullmatch('[0-9a-f]{40}', identity) and name not in result, 'ENUMERATION_IDENTITY')
            path = name.decode('utf-8'); require(path.encode() == name, 'PATH_ENCODING')
            result[name] = (mode, identity)
            require(len(result) <= MAX_ROWS, 'ROW_LIMIT')
        return result
    rows = decode(tree_raw, False)
    require(rows == decode(index_raw, True), 'INDEX_TREE_MISMATCH')
    root = {}
    for name, item in rows.items():
        node = root; bits = name.split(b'/')
        for bit in bits[:-1]:
            node = node.setdefault(bit, {}); require(isinstance(node, dict), 'TREE_COLLISION')
        require(bits[-1] not in node, 'TREE_COLLISION'); node[bits[-1]] = item
    def fold(node):
        body = bytearray(); previous = None
        for name, value in node.items():
            directory = isinstance(value, dict); key = name + (b'/' if directory else b'')
            require(previous is None or previous < key, 'TREE_ORDER'); previous = key
            mode, identity = ('40000', fold(value)) if directory else value
            body.extend(mode.encode()+b' '+name+b'\0'+bytes.fromhex(identity))
        return oid(body, b'tree')
    require(fold(root) == TREE, 'SOURCE_TREE_MISMATCH')
    return rows

def classify(path, data):
    if path.startswith('docs/plan/standalone/v3/'): return 'plan_tooling' if '/scripts/' in path else 'plan_normative_or_generated'
    if path.startswith('engine-snapshots/'): return 'historical_engine_snapshot'
    if path.startswith('.github/workflows/'): return 'ci_workflow'
    if path.endswith('Cargo.toml') or path == 'Cargo.lock': return 'cargo_manifest_or_lock'
    if path.startswith('conformance/') or '/fixtures/' in path: return 'fixture_or_conformance'
    if '/tests/' in path or path.startswith('tests/'): return 'test_source_or_data'
    if path.endswith('.rs'): return 'rust_source'
    if path.endswith(('.py', '.sh', '.js', '.ts')) or data.startswith(b'#!'): return 'script_or_adapter'
    if path.startswith(('.claude/', 'skills/')): return 'agent_instruction_or_hook'
    if path.endswith(('.md', '.html')): return 'documentation_or_presentation'
    return 'binary_data' if b'\0' in data else 'configuration_or_other_text'

def lane(path):
    for prefix, owner in (('crates/hugit-refstore/', 'B'), ('crates/hugit-checks/', 'C'),
                          ('crates/hugit-queue/', 'C'), ('crates/hugit-policy/', 'D'),
                          ('crates/hugit-mcp/', 'G'), ('crates/hugit-ledger/', 'G'),
                          ('.github/', 'H'), ('tests/', 'I')):
        if path.startswith(prefix): return owner
    return 'A'

def check_dependencies(basis, lock):
    require(basis['locked_packages'] == lock['package'], 'LOCK_DEPENDENCY_MISMATCH')
    packages = basis['cargo_packages']; ids = [p['id'] for p in packages]
    require(len(ids) == len(set(ids)) <= MAX_ROWS, 'DEPENDENCY_DUPLICATE')
    require(Counter((p['name'], p['version'], p.get('source')) for p in packages)
            == Counter((p['name'], p['version'], p.get('source')) for p in lock['package']), 'DEPENDENCY_SET_MISMATCH')
    graphs = basis['resolved_graphs']; require(set(graphs) == {'default', 'all-features'}, 'GRAPH_PROFILE_MISSING')
    all_nodes = set()
    for graph in graphs.values():
        nodes = graph['nodes']; names = [n['id'] for n in nodes]
        require(len(names) == len(set(names)) <= MAX_ROWS, 'GRAPH_DUPLICATE')
        names = set(names); require(names <= set(ids), 'UNCLASSIFIED_DEPENDENCY')
        for node in nodes:
            require(set(node['dependencies']) <= names, 'UNCLASSIFIED_DEPENDENCY')
            require(set(node['dependencies']) == {d['pkg'] for d in node['deps']}, 'GRAPH_EDGE_MISMATCH')
        all_nodes.update(names)
    require(all_nodes == set(ids), 'DEPENDENCY_SET_MISMATCH')

def source_paths(source):
    found = set(); visited = 0
    def walk(path, prefix, depth):
        nonlocal visited
        require(depth <= 64, 'DIRECTORY_DEPTH')
        with os.scandir(path) as entries:
            for entry in entries:
                if not prefix and entry.name == '.git': continue
                visited += 1; require(visited <= MAX_ROWS * 2, 'DIRECTORY_LIMIT')
                name = prefix + entry.name
                require(not entry.is_symlink(), 'SOURCE_SYMLINK')
                if entry.is_dir(follow_symlinks=False): walk(entry.path, name+'/', depth+1)
                else:
                    require(entry.is_file(follow_symlinks=False), 'SOURCE_SPECIAL_FILE')
                    found.add(name.encode()); require(len(found) <= MAX_ROWS, 'ROW_LIMIT')
    try: walk(source, '', 0)
    except OSError as exc: raise Invalid('SETUP_INPUT_UNAVAILABLE') from exc
    return found

def build(source, rows, basis, curated):
    require(source_paths(source) == set(rows), 'SOURCE_SET_MISMATCH')
    require(basis['source_commit'] == curated['source_commit'] == SOURCE
            and basis['source_tree'] == curated['source_tree'] == TREE, 'SUBJECT_MISMATCH')
    files = []; total = 0; relevant = {}; previous = None
    for name, (mode, identity) in rows.items():
        require(previous is None or previous < name, 'PATH_ORDER'); previous = name
        path = name.decode(); data = read(source, path, MAX_FILE); total += len(data)
        require(total <= MAX_SOURCE, 'SOURCE_BYTE_LIMIT'); require(oid(data) == identity, 'SOURCE_DIGEST')
        files.append(dict(path=path, path_bytes_base64=base64.b64encode(name).decode(),
            git_mode=mode, git_kind='blob', git_oid=identity, sha256=sha(data), size_bytes=len(data),
            classification=classify(path, data), owner_lane=lane(path), byte_inspection='verified_git_object',
            semantic_review=dict(status='not_reviewed', reviewer=None, ranges=[])))
        if path in (PLAN_PATH, 'Cargo.lock') or path.endswith('Cargo.toml'): relevant[path] = data
    require(sha(relevant[PLAN_PATH]) == PLAN_SHA, 'PLAN_IDENTITY')
    plan = parse(relevant[PLAN_PATH]); tasks = plan['tasks']; require(len(tasks) <= MAX_ROWS, 'ROW_LIMIT')
    check_dependencies(basis, tomllib.loads(relevant['Cargo.lock'].decode()))
    origin = dict(source_commit=SOURCE, source_tree=TREE, plan_file_sha256=PLAN_SHA,
                  plan_version=plan['plan_version'], tool_version='hug-001-inventory-1',
                  runtime_execution='not_run', qualification='partial_mechanical_inventory_not_wp_acceptance')
    inventory = dict(origin, files=files, file_count=len(files), classes=dict(Counter(f['classification'] for f in files)),
        semantic_reviewed_files=0, scope_exclusions=[], limits=[
        'All tracked paths remain in the denominator. Byte verification is not semantic review.',
        'Gitlinks, if present, identify external trees; their contents are not silently counted.'])
    bypath = {f['path']: f for f in files}; shared = defaultdict(list); bindings = []
    for task in tasks:
        for path in task['write_set']: shared[path].append(task['id'])
    for task in tasks:
        for path in task['write_set']:
            require(not PurePosixPath(path).is_absolute() and '..' not in PurePosixPath(path).parts
                    and not any(x in path for x in '*?['), 'PLAN_PATH_INVALID')
            item = bypath.get(path); conflicts = [str(a) for a in PurePosixPath(path).parents if str(a) in bypath]
            others = [x for x in shared[path] if x != task['id']]
            bindings.append(dict(wp_id=task['id'], path=path, owner_lane=task['owner_lane'],
                exists_at_subject=item is not None, existing_git_oid=item['git_oid'] if item else None,
                operation='edit_existing' if item else 'create_new', ancestor_file_conflicts=conflicts,
                shared_with=others, integrator_coordination_required=bool(others) or path in ('Cargo.toml', 'Cargo.lock'),
                admission='blocked_by_path_conflict' if conflicts else 'path_resolved_not_execution_authorized'))
    binding_doc = dict(origin, bindings=bindings, binding_count=len(bindings), unique_paths=len(shared),
        unknown_existing_paths=0, rule='Bindings pin existence only. Ownership of shared registries and semantic destination must be reviewed before dispatch.')
    # Selection and prose cannot be inferred from code. Preserve them from the
    # pinned curation; intentionally do not consume its derived count/flags.
    catalog = {key: curated[key] for key in CURATED}
    for group in ('routes', 'variants', 'source_tests', 'helpers'):
        require(isinstance(catalog[group], list) and len(catalog[group]) <= MAX_ROWS, 'ROW_LIMIT')
    for file_id, path in catalog['source_files'].items(): require(path in bypath, 'CURATION_SOURCE_MISSING')
    excluded = [row[0] for row in catalog['source_tests'] if row[5] == 'fixture_only_excluded']
    routes = catalog['routes']; cli = [r[0] for r in routes if r[0].startswith('cli:')]
    catalog.update(schema_revision='hug001-composed-1', source_commit=SOURCE, source_tree=TREE,
        plan_version=plan['plan_version'], plan_file_sha256=PLAN_SHA, whole_wp_ready=False,
        independent_review=False, transitive_effect_map_complete=False, product_tests_executed_this_increment=False,
        qualification='bounded_inventory_increment_not_whole_wp_acceptance', excluded_source_tests=excluded,
        mechanical_basis=dict(commit='5dcd4355c8ed896e168f7181e1d5add806ef6aaf', path='docs/audit/reachability.json',
            git_blob_oid=BASIS_OID, sha256=BASIS_SHA, size_bytes=6456776, required=True, preserved_sections=sorted(basis),
            resolution='git show <commit>:<path> in a full clone; verify Git OID and SHA-256 before loading. Missing history is a setup_error, never an empty inventory. This commit remains an ancestor; no network fallback or execution of referenced content.'),
        counts=dict(tracked_files=len(files), workspace_members=basis['workspace_member_count'], resolved_packages=len(basis['cargo_packages']),
            lexical_candidates=len(basis['surface_candidates']), path_bindings=len(bindings), distinct_bound_paths=len(shared),
            cli_roots=len({r.split('/')[0] for r in cli}), cli_leaves=len(cli), mcp_tools=sum(r[0].startswith('mcp:') for r in routes),
            behavior_variants=len(catalog['variants']), source_tests_selected=len(catalog['source_tests']),
            fixture_tests_excluded=len(excluded), assertion_helpers=len(catalog['helpers'])))
    require(catalog['closure']['accepted_criteria'] == [] and catalog['closure']['consumer_admission'] is False, 'AUTHORITY_ESCALATION')
    return dict(zip(OUTPUTS, (inventory, catalog, binding_doc)))

def render_catalog(doc):
    # Preserve the committed table layout, not a copy of the original JSON bytes.
    order = ('schema_revision', 'source_commit', 'source_tree', 'plan_version', 'plan_file_sha256',
        'whole_wp_ready', 'independent_review', 'transitive_effect_map_complete', 'product_tests_executed_this_increment',
        'qualification', 'mechanical_basis', 'counts', 'source_identity', 'source_files', 'route_columns', 'routes',
        'variant_columns', 'variants', 'test_columns', 'source_tests', 'helper_columns', 'helpers', 'excluded_source_tests',
        'locator_corrections', 'historical_observations', 'limitations', 'retained_inputs', 'remaining_coverage', 'closure')
    require(set(order) == set(doc), 'CATALOG_FIELDS')
    compact = lambda x: json.dumps(x, ensure_ascii=False, separators=(',', ':'))
    lines = ['{']
    for index, key in enumerate(order):
        comma = ',' if index < len(order)-1 else ''
        if key in ('routes', 'variants', 'source_tests'):
            lines.append('  '+json.dumps(key)+': [')
            lines.extend(compact(row)+(',' if i < len(doc[key])-1 else '') for i,row in enumerate(doc[key]))
            lines.append(']'+comma)
        else: lines.append('  '+json.dumps(key)+': '+compact(doc[key])+comma)
    return ('\n'.join(lines)+'\n}\n').encode()

def recover(source, tree_raw, index_raw, basis_raw, curation_raw):
    require(oid(basis_raw) == BASIS_OID and sha(basis_raw) == BASIS_SHA, 'BASIS_IDENTITY')
    require(oid(curation_raw) == CURATION_OID and sha(curation_raw) == CURATION_SHA, 'CURATION_IDENTITY')
    result = build(source, tree_records(tree_raw, index_raw), parse(basis_raw), parse(curation_raw))
    return {name: render_catalog(obj) if name == 'reachability.json' else encoded(obj) for name,obj in result.items()}

def main():
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('source', 'tree', 'index', 'basis', 'curation', 'out'): p.add_argument('--'+key, type=Path, required=True)
    args = p.parse_args()
    try:
        require(not args.source.is_symlink(), 'SOURCE_SYMLINK')
        source = args.source.resolve(strict=True); parent = args.out.parent.resolve(strict=True)
        destination = parent / args.out.name
        require(destination != source and source not in destination.parents and destination not in source.parents, 'OUTPUT_OVERLAP')
        require(not args.out.exists() and not args.out.is_symlink(), 'OUTPUT_EXISTS')
        documents = recover(source, read_path(args.tree), read_path(args.index), read_path(args.basis), read_path(args.curation))
        args.out.mkdir()
        for name, data in documents.items():
            with (args.out/name).open('xb') as output: output.write(data)
        report = dict(valid=True, source_commit=SOURCE, outputs_sha256={n:sha(b) for n,b in documents.items()},
            whole_wp_ready=False, independent_review=False, consumer_admission=False,
            scope='derived_manifest_reconstruction; retained curation and Cargo observations are required inputs')
    except (Invalid, OSError, KeyError, TypeError, ValueError, IndexError, RecursionError) as exc:
        report = dict(valid=False, status='setup_or_validation_error', error=str(exc) if isinstance(exc,Invalid) else 'SETUP_OR_SCHEMA_INVALID',
                      whole_wp_ready=False, consumer_admission=False)
    print(json.dumps(report, sort_keys=True)); return 0 if report['valid'] else 1

if __name__ == '__main__': sys.exit(main())
