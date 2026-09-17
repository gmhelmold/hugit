#!/usr/bin/env python3
"""Verify the pinned automation inventory, without executing any inventoried script.

The trusted manifest and source must be quiescent. Reuses the bounded POSIX
no-follow reader from verify_surface_contracts. A valid map is NOT WP approval.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import re
import sys
from verify_surface_contracts import (Invalid, require, read_under, load, keyed,
                                      digest, oid, MAX_BLOB, MAX_SOURCE)

FLAGS = ('whole_wp_ready', 'consumer_admission', 'independent_review',
         'scripts_executed', 'semantic_completeness_claimed')
CONFIGS = {'.claude/settings.json', '.cargo/config.toml'}
EXTERNAL = re.compile(rb'^export HUGIT_(?:CORELINK|RUNNER|GH_TEST_REPO)|\$\{HUGIT_GH_TEST_REPO', re.M)


def selected(path, mode, data):
    """Syntax-based denominator, not a claim of semantic execution reachability."""
    return (path.endswith(('.py', '.sh')) or mode == '100755'
            or (path.startswith('.github/workflows/') and path.endswith(('.yml', '.yaml')))
            or path in CONFIGS or (data.startswith(b'#!') and not data.startswith(b'#![')))


def verify(doc, inventory, source, plan):
    require(doc['schema_version'] == '1.0', 'SCHEMA_VERSION')
    require(all(doc['authority'].get(k) is False for k in FLAGS), 'AUTHORITY_ESCALATION')
    for k in ('source_commit', 'source_tree'):
        require(doc[k] == inventory[k] and re.fullmatch('[0-9a-f]{40}', doc[k]), 'SOURCE_IDENTITY')
    files = keyed(inventory['files'], 'path')
    require(len(files) == inventory['file_count'], 'SOURCE_COUNT')
    tasks = keyed(plan['tasks'], 'id')
    entries = keyed(doc['entries'], 'path')
    profiles = doc['profiles']
    require(isinstance(profiles, dict) and 0 < len(profiles) <= 64, 'PROFILE_SET')
    for p in profiles.values():
        require(p['role'] and isinstance(p['condition'], str) and p['condition']
                and isinstance(p['may_effects'], list) and p['may_effects']
                and p['destination_wp'] in tasks, 'PROFILE_CONTRACT')
    expected = {}; total = 0; verified = 0
    for path, item in files.items():
        raw = read_under(source, path, MAX_BLOB)
        total += len(raw)
        require(total <= MAX_SOURCE, 'SOURCE_BYTE_LIMIT')
        require(digest(raw) == item['sha256'] and oid(raw) == item['git_oid'], 'SOURCE_DIGEST')
        if selected(path, item['git_mode'], raw):
            expected[path] = raw
            verified += len(raw)
    require(set(entries) == set(expected), 'AUTOMATION_SET')
    external_paths = []
    for path, e in entries.items():
        require(e['profile'] in profiles, 'PROFILE_BINDING')
        require(e['sha256'] == digest(expected[path]), 'ENTRY_DIGEST')
        if path.startswith('tests/acceptance/') and path.endswith('/run.sh'):
            has_external_config = bool(EXTERNAL.search(expected[path]))
            require((e['profile'] == 'legacy_external') == has_external_config, 'EXTERNAL_BOUNDARY')
            if has_external_config:
                external_paths.append(path)
    guards = {
        'scripts/plan/sync_github.py': ('import_sync', 'optional_github_api_write'),
        'scripts/evidence_report.py': ('evidence_producer', 'output_replace'),
        '.github/workflows/release.yml': ('release_ci', 'github_release_write'),
        '.claude/settings.json': ('agent_config', 'spawn_registered_hook'),
        '.claude/hooks/forbid-sibling-paths.py': ('agent_guard', 'exit_decision'),
        'tests/acceptance/lib.sh': ('acceptance_helper', 'caller_command_execution'),
    }
    for path, (p, effect) in guards.items():
        require(entries[path]['profile'] == p and effect in profiles[p]['may_effects'], 'CRITICAL_EFFECT_REMOVED')
    anchors = doc['critical_anchors']
    require(isinstance(anchors, list) and 0 < len(anchors) <= 128, 'ANCHOR_LIMIT')
    seen = set()
    for a in anchors:
        path = a['path']; first = a['first']; last = a['last']
        key = (path, first, last)
        require(key not in seen and path in expected and a['profile'] == entries[path]['profile'], 'ANCHOR_BINDING')
        seen.add(key)
        require(type(first) is int and type(last) is int and 1 <= first <= last, 'ANCHOR_RANGE')
        lines = expected[path].splitlines(keepends=True)
        require(last <= len(lines) and digest(b''.join(lines[first-1:last])) == a['sha256'], 'SPAN_DIGEST')
    counts = {p: sum(e['profile'] == p for e in entries.values()) for p in profiles}
    return {'schema_version': '1.0', 'valid': True, 'source_commit': doc['source_commit'],
            'scope': doc['scope'], 'automation_files': len(entries),
            'automation_bytes': verified, 'all_source_files_verified': len(files),
            'all_source_bytes': total, 'profiles': counts,
            'external_configuration_wrappers': sorted(external_paths),
            'authority': {k: False for k in FLAGS},
            'limits': ['Historical source only; later integrator scripts are not retroactively included.',
                       'Source/configuration and integrity checks only; no scripts, workflows or tests of the product executed.',
                       'Changing annotation and source coherently still requires semantic review; hashes do not supply that review.']}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--root', type=Path, default=Path('.'))
    ap.add_argument('--source', type=Path, required=True)
    ap.add_argument('--report', type=Path)
    args = ap.parse_args()
    try:
        doc, _ = load(args.root, 'docs/audit/automation-contracts.json')
        inv, _ = load(args.root, 'docs/audit/source-inventory.json')
        plan, planraw = load(args.source, 'docs/plan/standalone/v3/backlog.json')
        require(digest(planraw) == inv['plan_file_sha256'], 'PLAN_IDENTITY')
        report = verify(doc, inv, args.source, plan)
    except (Invalid, KeyError, TypeError, ValueError, IndexError, RecursionError) as exc:
        report = {'valid': False, 'error': str(exc) if isinstance(exc, Invalid) else 'SCHEMA_INVALID',
                  'authority': {k: False for k in FLAGS}}
    text = json.dumps(report, sort_keys=True, indent=2) + '\n'
    if args.report:
        args.report.write_text(text)
    print(text, end='')
    return 0 if report['valid'] else 1


if __name__ == '__main__':
    sys.exit(main())
