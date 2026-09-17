#!/usr/bin/env python3
"""Independent Git fixtures for the read-only acquisition verifier. No Hugit execution."""
from __future__ import annotations
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import verify_inventory as v

@unittest.skipUnless(os.name == 'posix', 'POSIX descriptor-relative verifier')
class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.source = self.root / 'source'; self.source.mkdir()
        for name, data, mode in [('a./tool.sh', b'#!/bin/sh\nexit 0\n', 0o755),
                                 ('a/x.rs', b'fn main() {}\n', 0o644), ('z.txt', b'data\n', 0o644)]:
            path = self.source / name; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data); path.chmod(mode)
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(self.root),
               'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null', 'LC_ALL': 'C'}
        def git(*args):
            return subprocess.run(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'core.autocrlf=false',
                '-c', 'core.filemode=true', *args], cwd=self.source, env=env, check=True,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10).stdout
        git('init', '--quiet'); git('add', '--all')
        self.tree_oid = git('write-tree').decode().strip()
        self.index = self.root / 'index.z'; self.index.write_bytes(git('ls-files', '--stage', '-z'))
        self.tree = self.root / 'tree.z'; self.tree.write_bytes(git('ls-tree', '-r', '-z', self.tree_oid))
        entries = v.parse_enumeration(self.tree.read_bytes(), False)
        self.plan_data = {'tasks': [{'id': 'HUG-001', 'owner_lane': 'A',
                                   'write_set': ['a/x.rs', 'new/file.json']}]}
        self.plan = self.root / 'plan.json'; self.write(self.plan, self.plan_data)
        digest = hashlib.sha256(self.plan.read_bytes()).hexdigest()
        self.inv = {'source_commit': '1' * 40, 'source_tree': self.tree_oid,
                    'plan_file_sha256': digest, 'file_count': 3,
                    'classes': {'configuration_or_other_text': 3}, 'files': []}
        for raw, (mode, oid) in entries.items():
            name = raw.decode(); data = (self.source / name).read_bytes()
            self.inv['files'].append({'path': name, 'path_bytes_base64': base64.b64encode(raw).decode(),
                'classification': 'configuration_or_other_text', 'owner_lane': 'A', 'git_kind': 'blob',
                'git_mode': mode, 'git_oid': oid, 'sha256': hashlib.sha256(data).hexdigest(),
                'size_bytes': len(data), 'semantic_review': {'status': 'not_reviewed', 'ranges': [], 'reviewer': None}})
        self.bind = {'source_commit': '1' * 40, 'source_tree': self.tree_oid,
                     'plan_file_sha256': digest, 'binding_count': 2, 'bindings': []}
        for name in self.plan_data['tasks'][0]['write_set']:
            value = entries.get(name.encode())
            self.bind['bindings'].append({'wp_id': 'HUG-001', 'path': name, 'owner_lane': 'A',
                'exists_at_subject': value is not None, 'existing_git_oid': value[1] if value else None,
                'operation': 'edit_existing' if value else 'create_new', 'ancestor_file_conflicts': []})
        self.inventory = self.root / 'inventory.json'; self.bindings = self.root / 'bindings.json'
        self.save()

    @staticmethod
    def write(path, obj): path.write_text(json.dumps(obj), encoding='utf-8')
    def save(self): self.write(self.inventory, self.inv); self.write(self.bindings, self.bind)
    def args(self):
        return dict(source=self.source, inventory=self.inventory, bindings=self.bindings,
                    plan=self.plan, index=self.index, tree=self.tree, expected_tree=self.tree_oid)
    def verify(self): return v.verify(**self.args())
    def reject(self, code=None):
        with self.assertRaises((v.Rejected, OSError, ValueError)) as result: self.verify()
        if code: self.assertEqual(str(result.exception), code)

    def test_positive_and_no_false_admission(self):
        before = {p: p.read_bytes() for p in (self.inventory, self.bindings, self.plan, self.index, self.tree)}
        report = self.verify()
        self.assertEqual(report['files_verified'], 3); self.assertEqual(report['bindings_verified'], 2)
        for key in ('whole_wp_ready', 'consumer_admission', 'runtime_executed', 'semantic_review_accepted'):
            self.assertIs(report[key], False)
        self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_exact_git_tree(self):
        self.assertEqual(v.git_tree_oid(v.parse_enumeration(self.tree.read_bytes(), False)), self.tree_oid)
    def test_missing_manifest_file(self):
        self.inv['files'].pop(); self.save(); self.reject('MANIFEST_SET_MISMATCH')
    def test_duplicate_manifest_file(self):
        self.inv['files'].append(self.inv['files'][0]); self.save(); self.reject('DUPLICATE_MANIFEST_PATH')
    def test_added_source_file(self):
        (self.source / 'unclassified.sh').write_text('exit 0'); self.reject('SOURCE_SET_MISMATCH')
    def test_missing_source_file(self):
        (self.source / 'z.txt').unlink(); self.reject('SOURCE_SET_MISMATCH')
    def test_wrong_blob(self):
        (self.source / 'z.txt').write_bytes(b'evil\n'); self.reject('BLOB_MISMATCH')
    def test_wrong_executable_mode(self):
        (self.source / 'z.txt').chmod(0o755); self.reject('MODE_MISMATCH')
    def test_symlink_file(self):
        (self.source / 'z.txt').unlink(); (self.source / 'z.txt').symlink_to('/dev/null'); self.reject('SOURCE_SYMLINK')
    def test_symlink_directory(self):
        (self.source / 'elsewhere').symlink_to(self.root, target_is_directory=True); self.reject('SOURCE_SYMLINK')
    def test_fifo_does_not_hang(self):
        os.mkfifo(self.source / 'fifo'); self.reject('SOURCE_SPECIAL_FILE')
    def test_duplicate_json_key(self):
        self.inventory.write_text('{"files":[],"files":[]}'); self.reject('DUPLICATE_JSON_KEY')
    def test_nonfinite_json(self):
        self.inventory.write_text('{"x":NaN}'); self.reject('NONFINITE_JSON')
    def test_json_budget(self):
        with patch.object(v, 'MAX_JSON', 1): self.reject('INPUT_BUDGET_OR_TYPE')
    def test_enum_budget(self):
        with patch.object(v, 'MAX_ENUM', 1): self.reject('INPUT_BUDGET_OR_TYPE')
    def test_file_budget(self):
        with patch.object(v, 'MAX_FILE', 1): self.reject('FILE_BUDGET')
    def test_total_budget(self):
        with patch.object(v, 'MAX_TOTAL', 1): self.reject('TOTAL_BUDGET')
    def test_record_budget(self):
        with patch.object(v, 'MAX_RECORDS', 2): self.reject('RECORD_BUDGET')
    def test_missing_binding(self):
        self.bind['bindings'].pop(); self.save(); self.reject('BINDING_SET_MISMATCH')
    def test_duplicate_binding(self):
        self.bind['bindings'].append(self.bind['bindings'][0]); self.save(); self.reject('BINDING_SET_MISMATCH')
    def test_false_existence(self):
        self.bind['bindings'][1]['exists_at_subject'] = True; self.save(); self.reject('EXISTENCE_MISMATCH')
    def test_false_owner(self):
        self.bind['bindings'][0]['owner_lane'] = 'B'; self.save(); self.reject('BINDING_OWNER_MISMATCH')
    def test_wrong_plan(self):
        self.plan.write_text('{"tasks":[]}'); self.reject('PLAN_MISMATCH')
    def test_wrong_source_tree(self):
        args = self.args(); args['expected_tree'] = '0' * 40
        with self.assertRaisesRegex(v.Rejected, '^SOURCE_TREE_MISMATCH$'): v.verify(**args)
    def test_index_stage(self):
        self.index.write_bytes(self.index.read_bytes().replace(b' 0\t', b' 1\t', 1)); self.reject('ENUM_STAGE_OR_KIND')
    def test_unterminated_enum(self):
        self.index.write_bytes(self.index.read_bytes()[:-1]); self.reject('ENUM_TERMINATOR')
    def test_index_tree_disagreement(self):
        self.index.write_bytes(self.index.read_bytes().replace(b'z.txt', b'y.txt')); self.reject('INDEX_TREE_MISMATCH')
    def test_tree_order(self):
        rows = self.tree.read_bytes().split(b'\0')[:-1]; self.tree.write_bytes(b'\0'.join(reversed(rows)) + b'\0')
        self.reject('TREE_ORDER')
    def test_path_traversal(self):
        self.inv['files'][0]['path'] = '../outside'; self.save(); self.reject('PATH_TRAVERSAL')
    def test_false_review_locator(self):
        self.inv['files'][0]['semantic_review']['status'] = 'reviewed'; self.save(); self.reject('REVIEW_WITHOUT_LOCATOR')
    def test_recovery_is_deterministic(self):
        before = self.verify(); bytes_inv = self.inventory.read_bytes()
        self.inventory.unlink(); self.inventory.write_bytes(bytes_inv)
        self.assertEqual(before, self.verify())
    def test_bounded_cli_error(self):
        self.inventory.write_bytes(b'[' * 10000)
        command = [sys.executable, str(Path(v.__file__))]
        for key, value in self.args().items(): command += ['--' + key.replace('_', '-'), str(value)]
        result = subprocess.run(command, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 1); self.assertLess(len(result.stdout), 250)
        self.assertNotIn(str(self.root).encode(), result.stdout); self.assertEqual(result.stderr, b'')

if __name__ == '__main__': unittest.main()
