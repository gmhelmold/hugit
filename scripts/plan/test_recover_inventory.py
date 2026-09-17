#!/usr/bin/env python3
"""Recovery qualification on pinned inputs; never runs Hugit or legacy scripts."""
from __future__ import annotations
import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import recover_inventory as r

ARGS = None

class Recovery(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tree = r.read_path(ARGS.tree); cls.index = r.read_path(ARGS.index)
        cls.basis = r.read_path(ARGS.basis); cls.curation = r.read_path(ARGS.curation)
        cls.rows = r.tree_records(cls.tree, cls.index)
        cls.b = r.parse(cls.basis); cls.c = r.parse(cls.curation)
        cls.lock = r.tomllib.loads(r.read(ARGS.source, 'Cargo.lock').decode())
        cls.generated = r.recover(ARGS.source, cls.tree, cls.index, cls.basis, cls.curation)

    def test_rebuilt_bytes_match_all_three_committed_outputs(self):
        for name, raw in self.generated.items():
            expected = r.read(ARGS.root, 'docs/audit/'+name)
            with self.subTest(manifest=name):
                self.assertEqual(r.sha(raw), r.sha(expected),
                    f'{name}: generated={len(raw)} bytes reference={len(expected)} bytes')

    def test_discard_then_reconstruct_twice(self):
        with tempfile.TemporaryDirectory(prefix='hug001-recovery-') as tmp:
            out = Path(tmp)/'derived'; out.mkdir()
            # Deliberately obsolete placeholders: they must not become inputs.
            for name in r.OUTPUTS: (out/name).write_bytes(b'not the inventory\n')
            for _ in range(2):
                shutil.rmtree(out)
                self.assertFalse(out.exists())
                result = r.recover(ARGS.source, self.tree, self.index, self.basis, self.curation)
                out.mkdir()
                for name, raw in result.items(): (out/name).write_bytes(raw)
                self.assertEqual({p.name:r.sha(p.read_bytes()) for p in out.iterdir()},
                                 {n:r.sha(b) for n,b in self.generated.items()})

    def test_generated_data_passes_existing_surface_verifier(self):
        import verify_surface_contracts as v
        doc, _ = v.load(ARGS.root, 'docs/audit/surface-contracts.json')
        cat = r.parse(self.generated['reachability.json'])
        inv = r.parse(self.generated['source-inventory.json'])
        self.assertEqual(r.oid(self.generated['reachability.json']), doc['catalog_blob_oid'])
        result = v.verify(doc, cat, inv, self.b, ARGS.source)
        self.assertTrue(result['valid'])
        self.assertFalse(result['authority']['whole_wp_ready'])

    def test_curation_counts_and_authority_are_not_copied(self):
        changed = copy.deepcopy(self.c)
        changed['counts'] = {'invented': 999999}
        changed['whole_wp_ready'] = True
        changed['independent_review'] = True
        doc = r.build(ARGS.source, self.rows, self.b, changed)['reachability.json']
        expected = r.parse(self.generated['reachability.json'])
        self.assertEqual(doc['counts'], expected['counts'])
        self.assertFalse(doc['whole_wp_ready']); self.assertFalse(doc['independent_review'])

    def test_raw_curation_drift_is_rejected(self):
        with self.assertRaisesRegex(r.Invalid, '^CURATION_IDENTITY$'):
            r.recover(ARGS.source, self.tree, self.index, self.basis, self.curation+b' ')

    def test_raw_basis_drift_is_rejected(self):
        with self.assertRaisesRegex(r.Invalid, '^BASIS_IDENTITY$'):
            r.recover(ARGS.source, self.tree, self.index, self.basis+b' ', self.curation)

    def test_unclassified_transitive_dependency_is_rejected(self):
        changed = copy.deepcopy(self.b)
        node = changed['resolved_graphs']['default']['nodes'][0]
        node['dependencies'].append('unclassified-transitive-dependency')
        node['deps'].append({'pkg':'unclassified-transitive-dependency'})
        with self.assertRaisesRegex(r.Invalid, '^UNCLASSIFIED_DEPENDENCY$'):
            r.check_dependencies(changed, self.lock)

    def test_omitted_resolved_dependency_is_rejected(self):
        changed = copy.deepcopy(self.b)
        for graph in changed['resolved_graphs'].values():
            graph['nodes'] = [n for n in graph['nodes'] if n['id'] != self.b['cargo_packages'][0]['id']]
        with self.assertRaises(r.Invalid): r.check_dependencies(changed, self.lock)

    def test_dependency_package_set_must_match_lock(self):
        changed = copy.deepcopy(self.b); changed['cargo_packages'].pop()
        with self.assertRaisesRegex(r.Invalid, '^DEPENDENCY_SET_MISMATCH$'):
            r.check_dependencies(changed, self.lock)

    def test_lock_dependency_change_is_rejected(self):
        changed = copy.deepcopy(self.lock); changed['package'][0]['version'] = '999.0.0'
        with self.assertRaisesRegex(r.Invalid, '^LOCK_DEPENDENCY_MISMATCH$'):
            r.check_dependencies(self.b, changed)

    def test_graph_edge_views_must_agree(self):
        changed = copy.deepcopy(self.b)
        node = next(n for n in changed['resolved_graphs']['default']['nodes'] if n['dependencies'])
        node['deps'] = []
        with self.assertRaisesRegex(r.Invalid, '^GRAPH_EDGE_MISMATCH$'):
            r.check_dependencies(changed, self.lock)

    def test_both_graph_profiles_are_required(self):
        changed = copy.deepcopy(self.b); changed['resolved_graphs'].pop('all-features')
        with self.assertRaisesRegex(r.Invalid, '^GRAPH_PROFILE_MISSING$'):
            r.check_dependencies(changed, self.lock)

    def test_duplicate_dependency_is_rejected(self):
        changed = copy.deepcopy(self.b); changed['cargo_packages'].append(changed['cargo_packages'][0])
        with self.assertRaisesRegex(r.Invalid, '^DEPENDENCY_DUPLICATE$'):
            r.check_dependencies(changed, self.lock)

    def test_index_disagreement_is_rejected(self):
        wrong = self.index.replace(next(iter(self.rows.values()))[1].encode(), b'0'*40, 1)
        with self.assertRaisesRegex(r.Invalid, '^INDEX_TREE_MISMATCH$'):
            r.tree_records(self.tree, wrong)

    def test_unclassified_source_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp)/'source'
            shutil.copytree(ARGS.source, source, symlinks=True)
            (source/'unexpected-executable.sh').write_bytes(b'#!/bin/sh\nexit 0\n')
            with self.assertRaisesRegex(r.Invalid, '^SOURCE_SET_MISMATCH$'):
                r.build(source, self.rows, self.b, self.c)

    def test_missing_source_is_setup_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(r.Invalid, '^SETUP_INPUT_UNAVAILABLE$'):
                r.build(Path(tmp)/'absent', self.rows, self.b, self.c)

    def invoke(self, source, curation, out):
        return subprocess.run([sys.executable, str(Path(r.__file__).resolve()),
            '--source', str(source), '--tree', str(ARGS.tree), '--index', str(ARGS.index),
            '--basis', str(ARGS.basis), '--curation', str(curation), '--out', str(out)],
            capture_output=True, timeout=60, check=False)

    def test_missing_curation_has_no_success_or_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)/'new'
            p = self.invoke(ARGS.source, Path(tmp)/'absent', out)
            self.assertEqual(p.returncode, 1); self.assertFalse(out.exists())
            report = json.loads(p.stdout)
            self.assertFalse(report['valid']); self.assertEqual(report['error'], 'SETUP_INPUT_UNAVAILABLE')

    def test_existing_output_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)/'existing'; out.mkdir(); (out/'sentinel').write_bytes(b'keep')
            p = self.invoke(ARGS.source, ARGS.curation, out)
            self.assertEqual(p.returncode, 1); self.assertEqual(json.loads(p.stdout)['error'], 'OUTPUT_EXISTS')
            self.assertEqual((out/'sentinel').read_bytes(), b'keep')

    def test_output_under_source_is_rejected(self):
        out = ARGS.source/'recovery-output-must-not-be-created'
        self.assertFalse(out.exists())
        p = self.invoke(ARGS.source, ARGS.curation, out)
        self.assertEqual(p.returncode, 1); self.assertEqual(json.loads(p.stdout)['error'], 'OUTPUT_OVERLAP')
        self.assertFalse(out.exists())

    def test_cli_writes_only_new_output_and_keeps_authority_false(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)/'new'
            p = self.invoke(ARGS.source, ARGS.curation, out)
            self.assertEqual(p.returncode, 0, p.stdout)
            report = json.loads(p.stdout)
            self.assertEqual(set(report['outputs_sha256']), set(r.OUTPUTS))
            for flag in ('whole_wp_ready', 'independent_review', 'consumer_admission'): self.assertFalse(report[flag])
            for name in r.OUTPUTS: self.assertEqual((out/name).read_bytes(), self.generated[name])

    def test_nofollow_limits_and_duplicate_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root/'file').write_bytes(b'abc'); (root/'link').symlink_to('file')
            with self.assertRaisesRegex(r.Invalid, '^SETUP_INPUT_UNAVAILABLE$'): r.read(root, 'link')
            with self.assertRaisesRegex(r.Invalid, '^INPUT_TYPE_OR_LIMIT$'): r.read(root, 'file', 2)
            with self.assertRaisesRegex(r.Invalid, '^PATH_INVALID$'): r.read(root, '../file')
        with self.assertRaisesRegex(r.Invalid, '^JSON_DUPLICATE_KEY$'): r.parse(b'{"a":1,"a":2}')

if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__)
    for key in ('root', 'source', 'tree', 'index', 'basis', 'curation'): ap.add_argument('--'+key, type=Path, required=True)
    ap.add_argument('--report', type=Path, required=True)
    ARGS = ap.parse_args()
    # The tested generator never sees the expected source-inventory/bindings.
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Recovery))
    report = {'scope':'HUG-001 reconstruction and dependency controls; not independent acceptance',
              'tests_run':result.testsRun,'failures':len(result.failures),'errors':len(result.errors),
              'valid':result.wasSuccessful(),'whole_wp_ready':False,'consumer_admission':False,
              'independent_review':False,'new_cargo_resolution':False,
              'source_commit':r.SOURCE,'curation_blob_oid':r.CURATION_OID,'basis_blob_oid':r.BASIS_OID}
    if hasattr(Recovery,'generated'):
        report['regenerated_sha256'] = {n:r.sha(b) for n,b in Recovery.generated.items()}
    ARGS.report.write_text(json.dumps(report, sort_keys=True, indent=2)+'\n')
    sys.exit(0 if result.wasSuccessful() else 1)
