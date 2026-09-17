#!/usr/bin/env python3
"""Adversarial checks of the automation map. Never runs an inventoried script."""
import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import verify_automation as v

ROOT = Path('.')
SOURCE = None


class AutomationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.doc, _ = v.load(ROOT, 'docs/audit/automation-contracts.json')
        cls.inv, _ = v.load(ROOT, 'docs/audit/source-inventory.json')
        cls.plan, _ = v.load(SOURCE, 'docs/plan/standalone/v3/backlog.json')

    def run_map(self, doc=None, inv=None):
        return v.verify(self.doc if doc is None else doc,
                        self.inv if inv is None else inv, SOURCE, self.plan)

    def reject(self, change, code):
        doc = copy.deepcopy(self.doc)
        change(doc)
        with self.assertRaisesRegex(v.Invalid, '^' + code + '$'):
            self.run_map(doc)

    def test_positive(self):
        r = self.run_map()
        self.assertEqual(r['automation_files'], 81)
        self.assertEqual(len(r['external_configuration_wrappers']), 17)
        self.assertFalse(any(r['authority'].values()))

    def test_repeated_result(self):
        self.assertEqual(self.run_map(), self.run_map())

    def test_all_authority_flags(self):
        for flag in v.FLAGS:
            with self.subTest(flag=flag):
                self.reject(lambda d: d['authority'].__setitem__(flag, True), 'AUTHORITY_ESCALATION')

    def test_source_commit(self):
        self.reject(lambda d: d.__setitem__('source_commit', '0' * 40), 'SOURCE_IDENTITY')

    def test_source_tree(self):
        self.reject(lambda d: d.__setitem__('source_tree', '1' * 40), 'SOURCE_IDENTITY')

    def test_missing_script(self):
        self.reject(lambda d: d['entries'].pop(), 'AUTOMATION_SET')

    def test_duplicate_script(self):
        self.reject(lambda d: d['entries'].append(d['entries'][0]), 'DUPLICATE_ID')

    def test_unknown_script(self):
        self.reject(lambda d: d['entries'].append({'path': 'scripts/unknown.py'}), 'AUTOMATION_SET')

    def test_unknown_owner(self):
        self.reject(lambda d: d['profiles']['plan_ci'].__setitem__('destination_wp', 'HUG-999'), 'PROFILE_CONTRACT')

    def test_empty_condition(self):
        self.reject(lambda d: d['profiles']['ci'].__setitem__('condition', ''), 'PROFILE_CONTRACT')

    def test_wrong_entry_digest(self):
        self.reject(lambda d: d['entries'][0].__setitem__('sha256', '0' * 64), 'ENTRY_DIGEST')

    def test_unknown_profile(self):
        self.reject(lambda d: d['entries'][0].__setitem__('profile', 'unknown'), 'PROFILE_BINDING')

    def test_external_wrapper_downgraded(self):
        self.reject(lambda d: next(e for e in d['entries'] if e['profile'] == 'legacy_external').__setitem__('profile', 'legacy_wrapper'), 'EXTERNAL_BOUNDARY')

    def test_false_external_classification(self):
        self.reject(lambda d: next(e for e in d['entries'] if e['profile'] == 'legacy_wrapper').__setitem__('profile', 'legacy_external'), 'EXTERNAL_BOUNDARY')

    def test_release_write_not_erased(self):
        self.reject(lambda d: d['profiles']['release_ci']['may_effects'].remove('github_release_write'), 'CRITICAL_EFFECT_REMOVED')

    def test_sync_write_not_erased(self):
        self.reject(lambda d: d['profiles']['import_sync']['may_effects'].remove('optional_github_api_write'), 'CRITICAL_EFFECT_REMOVED')

    def test_output_replace_not_erased(self):
        self.reject(lambda d: d['profiles']['evidence_producer']['may_effects'].remove('output_replace'), 'CRITICAL_EFFECT_REMOVED')

    def test_wrong_anchor_hash(self):
        self.reject(lambda d: d['critical_anchors'][0].__setitem__('sha256', '0' * 64), 'SPAN_DIGEST')

    def test_wrong_anchor_range(self):
        self.reject(lambda d: d['critical_anchors'][0].__setitem__('first', 0), 'ANCHOR_RANGE')

    def test_duplicate_anchor(self):
        self.reject(lambda d: d['critical_anchors'].append(d['critical_anchors'][0]), 'ANCHOR_BINDING')

    def test_source_corruption(self):
        inv = copy.deepcopy(self.inv)
        inv['files'][0]['sha256'] = '0' * 64
        with self.assertRaisesRegex(v.Invalid, '^SOURCE_DIGEST$'):
            self.run_map(inv=inv)

    def test_aggregate_limit(self):
        with patch.object(v, 'MAX_SOURCE', 1):
            with self.assertRaisesRegex(v.Invalid, '^SOURCE_BYTE_LIMIT$'):
                self.run_map()

    def test_denominator_includes_new_script_forms(self):
        for path, mode, body in [('new.py','100644',b''), ('no-ext','100755',b''),
                                 ('custom','100644',b'#!/bin/sh\n'),
                                 ('.github/workflows/x.yaml','100644',b'')]:
            self.assertTrue(v.selected(path, mode, body))
        self.assertFalse(v.selected('test.rs', '100644', b'#![cfg(unix)]\n'))

    def test_no_follow(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / 'data').write_bytes(b'ordinary')
            (root / 'link').symlink_to(root / 'data')
            with self.assertRaisesRegex(v.Invalid, '^FILE_UNAVAILABLE$'):
                v.read_under(root, 'link', 100)

    def test_json_duplicate_key(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / 'a.json').write_text('{"x":1,"x":2}')
            with self.assertRaisesRegex(v.Invalid, '^JSON_DUPLICATE_KEY$'):
                v.load(root, 'a.json')


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--root', type=Path, default=Path('.'))
    ap.add_argument('--source', type=Path, required=True)
    args, rest = ap.parse_known_args()
    ROOT = args.root
    SOURCE = args.source
    unittest.main(argv=['test_automation.py'] + rest)
