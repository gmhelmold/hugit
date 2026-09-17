#!/usr/bin/env python3
"""Tests of this local diagnostic, not execution of Hugit or legacy wrappers."""
import argparse
import hashlib
import json
import os
import io
import contextlib
from unittest import mock
from pathlib import Path
import tempfile
import unittest
import resolve_wrapper_tests as diagnostic

ARGS = None

class DeclarationTests(unittest.TestCase):
    def test_literal_and_scoped_variables(self):
        call, fields = diagnostic.declarations('CRATE_NAME="hugit-queue"\nWP_ID="wp-b10"\ncargo test -p "$CRATE_NAME" --test "acceptance_${WP_ID}"\n')
        self.assertEqual(call['argv'], ['cargo','test','-p','hugit-queue','--test','acceptance_wp-b10'])

    def test_description_does_not_become_invocation(self):
        call, _ = diagnostic.declarations('check "cargo test -p imaginary" test -f Cargo.toml\n')
        self.assertIsNone(call)

    def test_comments_do_not_become_invocation(self):
        call, _ = diagnostic.declarations('# cargo test -p imaginary\n')
        self.assertIsNone(call)

    def test_list_probe_does_not_become_final_invocation(self):
        call, _ = diagnostic.declarations('TESTLIST="$(cargo test -p demo --test example -- --list)"\n')
        self.assertIsNone(call)

    def test_unknown_variable_refused(self):
        with self.assertRaisesRegex(ValueError, '^UNRESOLVED_INVOCATION$'):
            diagnostic.declarations('cargo test -p "$UNRESOLVED_PACKAGE"\n')

    def test_multiple_final_commands_are_not_chosen_silently(self):
        with self.assertRaisesRegex(ValueError, '^MULTIPLE_FINAL_INVOCATIONS$'):
            diagnostic.declarations('cargo test -p first\ncargo test -p second\n')

    def test_check_wrapper_uses_argv_not_its_description(self):
        call, _ = diagnostic.declarations('check "gate: cargo test" cargo test --workspace\n')
        self.assertEqual(call['argv'], ['cargo','test','--workspace'])

    def test_unquoted_assignment(self):
        _, fields = diagnostic.declarations('CRATE=crates/hugit-app\nACCEPTANCE_FILE="$CRATE/tests/acceptance_wp_b1.rs"\n')
        self.assertEqual(fields['ACCEPTANCE_FILE']['resolved_value'], 'crates/hugit-app/tests/acceptance_wp_b1.rs')

    def test_input_identity_refuses_changed_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'input.json'; path.write_text('{}')
            with self.assertRaisesRegex(ValueError, '^INPUT_IDENTITY$'):
                diagnostic.checked_json(path, diagnostic.BASIS_SHA)

class InputOutputTests(unittest.TestCase):
    def test_nofollow_and_special_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'file').write_bytes(b'ok')
            self.assertEqual(diagnostic.read_regular(root, 'file', 2), b'ok')
            (root/'link').symlink_to(root/'file')
            with self.assertRaisesRegex(diagnostic.Invalid, '^INPUT_UNAVAILABLE$'):
                diagnostic.read_regular(root, 'link', 10)
            (root/'dir').mkdir(); (root/'dir'/'f').write_bytes(b'x')
            (root/'alias').symlink_to(root/'dir', target_is_directory=True)
            with self.assertRaisesRegex(diagnostic.Invalid, '^INPUT_UNAVAILABLE$'):
                diagnostic.read_regular(root, 'alias/f', 10)
            if hasattr(os, 'mkfifo'):
                os.mkfifo(root/'fifo')
                with self.assertRaisesRegex(diagnostic.Invalid, '^INPUT_TYPE_OR_LIMIT$'):
                    diagnostic.read_regular(root, 'fifo', 10)

    def test_read_limits_and_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root/'file').write_bytes(b'hello')
            with self.assertRaisesRegex(diagnostic.Invalid, '^INPUT_TYPE_OR_LIMIT$'):
                diagnostic.read_regular(root, 'file', 4)
            for name in ('../file', '/file', 'a//b', 'a/./b'):
                with self.subTest(path=name), self.assertRaises(diagnostic.Invalid):
                    diagnostic.read_regular(root, name, 10)

    def test_refuses_output_in_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(diagnostic.Invalid, '^OUTPUT_INSIDE_SOURCE$'):
                diagnostic.write_report(root, root/'output.json', {})
            self.assertFalse((root/'output.json').exists())

    def test_output_is_exclusive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root/'source').mkdir()
            out = root/'report.json'
            diagnostic.write_report(root/'source', out, {'ok': True})
            before = out.read_bytes()
            with self.assertRaisesRegex(diagnostic.Invalid, '^OUTPUT_EXISTS$'):
                diagnostic.write_report(root/'source', out, {'ok': False})
            self.assertEqual(out.read_bytes(), before)

    def test_setup_error_is_bounded_and_not_approval(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); output = io.StringIO()
            argv = ['resolver', '--source', str(root), '--inventory', str(root/'missing'),
                    '--basis', str(root/'missing-basis'), '--out', str(root/'report')]
            with mock.patch('sys.argv', argv), contextlib.redirect_stdout(output):
                self.assertEqual(diagnostic.main(), 2)
            report = json.loads(output.getvalue())
            self.assertFalse(report['valid']); self.assertFalse(report['whole_wp_ready'])
            self.assertNotIn(directory, output.getvalue())
            self.assertLess(len(output.getvalue()), 256)

class PinnedSourceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.report = diagnostic.resolve(ARGS.source, ARGS.inventory, ARGS.basis)

    def test_denominator_and_authority(self):
        self.assertEqual(self.report['wrapper_count'], 52)
        self.assertFalse(self.report['whole_wp_ready'])
        self.assertTrue(all(e['tests_executed'] is False for e in self.report['entries']))

    def test_nine_legacy_targets_are_not_promoted(self):
        absent = {e['wrapper'].split('/')[-2] for e in self.report['entries'] if e['resolution'] in ('package_absent','test_target_absent')}
        self.assertEqual(absent, {'wp-b6','wp-b7','wp-c2a','wp-c2b','wp-c3','wp-c5a','wp-c9','wp-e4','wp-x4'})

    def test_x4_does_not_substitute_wire_oracle(self):
        row = next(e for e in self.report['entries'] if e['wrapper'].endswith('/wp-x4/run.sh'))
        self.assertEqual(row['named_test'], 'acceptance_x4')
        self.assertEqual(row['resolution'], 'test_target_absent')
        self.assertIsNone(row['test_source'])
        self.assertFalse(row['semantic_equivalence_claimed'])

    def test_repeated_resolution_equal(self):
        repeated = diagnostic.resolve(ARGS.source, ARGS.inventory, ARGS.basis)
        self.assertEqual(self.report, repeated)

    def test_all_source_bytes_preserved(self):
        inventory = diagnostic.checked_json(ARGS.inventory, diagnostic.INVENTORY_SHA)
        for row in inventory['files']:
            data = (ARGS.source/row['path']).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), row['sha256'], row['path'])

if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,required=True)
    p.add_argument('--inventory',type=Path,required=True)
    p.add_argument('--basis',type=Path,required=True)
    ARGS, rest = p.parse_known_args()
    unittest.main(argv=[__file__]+rest)
