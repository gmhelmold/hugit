#!/usr/bin/env python3
"""Controls for the native evidence gate, not simulated product qualification."""
import copy
import unittest

import qualify_native_coordination as gate


def logs(cell):
    unix = gate.CELLS[cell][0] != 'Windows'
    groups = {'unit': gate.UNIT, 'integration': gate.COMMON | (gate.UNIX_ONLY if unix else set())}
    value = {}
    for group, names in groups.items():
        helper = {gate.HELPER} if group == 'integration' else set()
        value[group+'-list'] = ''.join(n+': test\n' for n in sorted(names | helper))
        value[group] = ''.join('test '+n+' ... ok\n' for n in sorted(names))
        if helper:
            value[group] += 'test subprocess_fixture ... ignored, subprocess helper\n'
        value[group] += f'test result: ok. {len(names)} passed; 0 failed; {len(helper)} ignored; 0 measured; 0 filtered out;\n'
    if unix:
        value['integration'] += 'owner_paused_ms=130000 stable_file_preserved=true\n'
    return value


def subject(cell):
    system, arch, host, fs = gate.CELLS[cell]
    return {'schema': gate.SCHEMA, 'cell': cell, 'system': system, 'architecture': arch,
            'filesystem': fs, 'rustc': 'rustc 1.96.0\nhost: '+host, 'target': host,
            'fixture_root': '/fixture', 'rust_temp_dir': '/fixture',
            'files': dict.fromkeys(gate.SOURCES, 'a'*64),
            'checkout': 'b'*40, 'tree': 'c'*40, 'subject_head': 'd'*40}


class GateControls(unittest.TestCase):
    def test_all_four_declared_cells(self):
        for cell in gate.CELLS:
            with self.subTest(cell=cell):
                gate.validate_identity(subject(cell), cell)
                result = gate.outcomes(cell, logs(cell))
                self.assertEqual(result['unit']['passed'], 2)
                self.assertEqual(result['integration']['passed'], 19 if cell.startswith('windows') else 23)
                self.assertEqual(len(result['unix_only_not_executed']), 4 if cell.startswith('windows') else 0)

    def test_wrong_identity_fields_refused(self):
        cell = 'macos-arm64-apfs'
        mutations = {'schema': 'old', 'cell': 'linux-x64-ext4', 'system': 'Linux', 'architecture': 'x86_64',
                     'filesystem': 'ext4', 'target': 'x86_64-apple-darwin', 'rustc': 'host: x86_64-apple-darwin',
                     'rust_temp_dir': '/different', 'checkout': '', 'tree': 'not-git', 'subject_head': ''}
        for field, value in mutations.items():
            with self.subTest(field=field):
                item = subject(cell); item[field] = value
                with self.assertRaises(ValueError): gate.validate_identity(item, cell)

    def test_source_set_or_digest_missing(self):
        cell = 'linux-x64-ext4'
        for mode in ('missing', 'extra', 'digest'):
            with self.subTest(mode=mode):
                item = subject(cell)
                if mode == 'missing': item['files'].pop(gate.SOURCES[0])
                if mode == 'extra': item['files']['other'] = 'a'*64
                if mode == 'digest': item['files'][gate.SOURCES[0]] = ''
                with self.assertRaises(ValueError): gate.validate_identity(item, cell)

    def test_jointly_omitted_test_cannot_redefine_expected_set(self):
        cell = 'linux-x64-ext4'; value = logs(cell); name = sorted(gate.COMMON)[0]
        value['integration-list'] = value['integration-list'].replace(name+': test\n', '')
        value['integration'] = value['integration'].replace('test '+name+' ... ok\n', '')
        with self.assertRaises(ValueError): gate.outcomes(cell, value)

    def test_log_mutations_refused(self):
        cell = 'linux-x64-ext4'; base = logs(cell); name = sorted(gate.COMMON)[0]
        changes = [
            ('unit-list', ''), ('unit', ''), ('integration-list', ''), ('integration', ''),
            ('integration-list', base['integration-list']+name+': test\n'),
            ('integration', base['integration']+'test '+name+' ... ok\n'),
            ('integration', base['integration'].replace('test '+name+' ... ok', 'test '+name+' ... FAILED')),
            ('integration', base['integration'].replace('0 failed', '1 failed')),
            ('integration', base['integration'].replace('1 ignored', '2 ignored')),
            ('integration', base['integration'].replace('owner_paused_ms=130000', 'owner_paused_ms=129999')),
            ('integration', base['integration'].replace('owner_paused_ms=130000', 'pause_missing')),
            ('integration', base['integration']+'owner_paused_ms=130001\n'),
            ('integration', base['integration']+'test arbitrary ... ignored\n'),
        ]
        for index, (field, value) in enumerate(changes):
            with self.subTest(index=index):
                altered = copy.deepcopy(base); altered[field] = value
                with self.assertRaises(ValueError): gate.outcomes(cell, altered)

    def test_windows_cannot_claim_unix_pause(self):
        cell = 'windows-x64-ntfs'; value = logs(cell)
        value['integration'] += 'owner_paused_ms=130000\n'
        with self.assertRaises(ValueError): gate.outcomes(cell, value)

    def test_unexpected_passed_test_refused(self):
        cell = 'macos-intel-apfs'; value = logs(cell)
        value['integration'] += 'test unrelated ... ok\n'
        with self.assertRaises(ValueError): gate.outcomes(cell, value)

    def test_native_windows_newlines_are_normalized(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'log'
            path.write_bytes(b'test case ... ok\r\n')
            self.assertEqual(gate.read_log(path), 'test case ... ok\n')

    def test_verification_checks_files_cleanup_and_checkout(self):
        import hashlib
        import json
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        cell = 'linux-x64-ext4'
        for mutation in ('none', 'source', 'leftover', 'filesystem', 'checkout', 'tree', 'dirty', 'missing_log'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); evidence = root / 'evidence'; fixtures = root / 'fixtures'
                evidence.mkdir(); fixtures.mkdir()
                item = subject(cell)
                item['fixture_root'] = item['rust_temp_dir'] = str(fixtures.resolve())
                for name in gate.SOURCES:
                    path = root / name; path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b'unchanged')
                    item['files'][name] = hashlib.sha256(b'unchanged').hexdigest()
                (evidence / 'subject.json').write_text(json.dumps(item))
                for name, text in logs(cell).items(): (evidence / (name+'.log')).write_text(text)
                if mutation == 'source': (root / gate.SOURCES[0]).write_bytes(b'changed')
                if mutation == 'leftover': (fixtures / 'hugit-native-lock-1').mkdir()
                if mutation == 'missing_log': (evidence / 'unit.log').unlink()
                def command(*args):
                    if 'status' in args: return ' M source' if mutation == 'dirty' else ''
                    field = 'tree' if args[-1] == 'HEAD^{tree}' else 'checkout'
                    return 'e'*40 if mutation == field else item[field]
                with patch.object(gate, 'filesystem', return_value='apfs' if mutation == 'filesystem' else 'ext4'), patch.object(gate, 'output', side_effect=command):
                    if mutation == 'none':
                        gate.verify(root, evidence, fixtures, cell)
                        self.assertTrue((evidence / 'results.json').is_file())
                    else:
                        with self.assertRaises(ValueError): gate.verify(root, evidence, fixtures, cell)
                        self.assertFalse((evidence / 'results.json').exists())

    def test_no_python_assertions_used_for_gate(self):
        import ast
        from pathlib import Path
        tree = ast.parse(Path(gate.__file__).read_text())
        self.assertFalse(any(isinstance(n, ast.Assert) for n in ast.walk(tree)))


if __name__ == '__main__':
    unittest.main(verbosity=2)
