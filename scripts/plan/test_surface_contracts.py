#!/usr/bin/env python3
"""Adversarial data checks for the annotation. Does not execute Rust or Hugit."""
import argparse
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
import verify_surface_contracts as v

ARGS = None

class SurfaceContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.doc,_=v.load(ARGS.root,'docs/audit/surface-contracts.json')
        cls.cat,_=v.load(ARGS.root,'docs/audit/reachability.json')
        cls.inv,_=v.load(ARGS.root,'docs/audit/source-inventory.json')
        cls.basis,_=v.load(ARGS.basis.parent,ARGS.basis.name)

    def good(self,doc=None,cat=None,inv=None,basis=None,source=None):
        return v.verify(self.doc if doc is None else doc,
                        self.cat if cat is None else cat,
                        self.inv if inv is None else inv,
                        self.basis if basis is None else basis,
                        ARGS.source if source is None else source)

    def mutate(self,change,code):
        doc=copy.deepcopy(self.doc); change(doc)
        with self.assertRaisesRegex(v.Invalid, '^'+code+'$'):
            self.good(doc=doc)

    def test_destination_wp_must_exist_for_all_surface_kinds(self):
        # Every claimant of HUG-001-SC01 must resolve a real work package.
        selectors = (
            lambda d: next(iter(d['profiles'].values())),
            lambda d: d['workspace_libraries'][0],
            lambda d: d['workspace_binaries'][0],
        )
        for index, select in enumerate(selectors):
            with self.subTest(surface_kind=index):
                self.mutate(lambda d: select(d).update(destination_wp='HUG-999'),
                            'DESTINATION_WP_UNKNOWN')

    def test_missing_destination_for_all_surface_kinds(self):
        for group in ('profiles', 'workspace_libraries', 'workspace_binaries'):
            with self.subTest(group=group):
                def change(d):
                    rows = d[group]
                    row = next(iter(rows.values())) if isinstance(rows, dict) else rows[0]
                    row.pop('destination_wp')
                self.mutate(change, 'DESTINATION_WP_FORMAT')

    def test_non_ascii_destination_is_not_a_work_package_id(self):
        self.mutate(lambda d: d['workspace_binaries'][0].update(destination_wp='HUG-٠٠١'),
                    'DESTINATION_WP_FORMAT')

    def test_owner_lane_required(self):
        plan, _ = v.load(ARGS.source, 'docs/plan/standalone/v3/backlog.json')
        destination = next(iter(self.doc['profiles'].values()))['destination_wp']
        for bad in (None, '', ' ', [], ' A', 'A' * 65):
            with self.subTest(owner=repr(bad)):
                modified = copy.deepcopy(plan)
                next(t for t in modified['tasks'] if t['id'] == destination)['owner_lane'] = bad
                with self.assertRaisesRegex(v.Invalid, '^DESTINATION_OWNER_MISSING$'):
                    v.resolve_destination_owners(self.doc, modified)

    def test_duplicate_destination_task_identity(self):
        plan, _ = v.load(ARGS.source, 'docs/plan/standalone/v3/backlog.json')
        plan['tasks'].append(copy.deepcopy(plan['tasks'][0]))
        with self.assertRaisesRegex(v.Invalid, '^DUPLICATE_ID$'):
            v.resolve_destination_owners(self.doc, plan)

    def test_plan_digest_must_match_inventory(self):
        inv = copy.deepcopy(self.inv)
        inv['plan_file_sha256'] = '0' * 64
        with self.assertRaisesRegex(v.Invalid, '^PLAN_IDENTITY$'):
            self.good(inv=inv)

    def test_owner_resolution_is_reported_without_approval(self):
        plan, raw = v.load(ARGS.source, 'docs/plan/standalone/v3/backlog.json')
        tasks = {t['id']: t for t in plan['tasks']}
        rows = list(self.doc['profiles'].values()) + self.doc['workspace_libraries'] + self.doc['workspace_binaries']
        expected = {r['destination_wp']: tasks[r['destination_wp']]['owner_lane'] for r in rows}
        result = self.good()
        self.assertEqual(result['destination_owners'], expected)
        self.assertEqual(result['plan_sha256'], v.digest(raw))
        self.assertTrue(all(flag is False for flag in result['authority'].values()))

    def test_positive(self):
        r=self.good()
        self.assertEqual(r['routes'],57)
        self.assertEqual(r['variants'],24)
        self.assertTrue(r['valid'])
        self.assertTrue(all(x is False for x in r['authority'].values()))

    def test_missing_route(self):
        self.mutate(lambda d:d['routes'].pop(),'ROUTE_SET')

    def test_duplicate_route(self):
        self.mutate(lambda d:d['routes'].append(d['routes'][0]),'DUPLICATE_ID')

    def test_unknown_profile(self):
        self.mutate(lambda d:d['routes'][0]['profiles'].append('unknown'),'PROFILE_BINDING')

    def test_all_critical_boundaries(self):
        for route,profile in [('cli:check/run','check'),('cli:capture','drain'),
                              ('cli:fleet','projection'),('cli:watch','projection'),
                              ('cli:pr/land','dispatch_residue'),('cli:land/queue','queue_sim'),
                              ('cli:dock/land','dock_sim'),('mcp:cost-attest','mcp_http'),
                              ('mcp:liveness-probe','mcp_http'),('mcp:capture','mcp_child'),
                              ('mcp:land-status','mcp_child')]:
            with self.subTest(route=route):
                def change(d):
                    r=next(r for r in d['routes'] if r['id']==route)
                    r['profiles']=[p for p in r['profiles'] if p!=profile] or ['value']
                self.mutate(change,'CRITICAL_BOUNDARY_MISSING')

    def test_cache_write_not_optional_with_store(self):
        self.mutate(lambda d:d['profiles']['check']['effects'].remove('cache_write'),'EFFECT_REMOVED')

    def test_projection_write_not_hidden(self):
        self.mutate(lambda d:d['profiles']['projection']['effects'].remove('projection_status_write'),'EFFECT_REMOVED')

    def test_source_identity(self):
        self.mutate(lambda d:d.update(source_commit='0'*40),'SOURCE_IDENTITY')

    def test_all_authority_escalations(self):
        for flag in v.FLAGS:
            with self.subTest(flag=flag):
                self.mutate(lambda d:d['authority'].update({flag:True}),'AUTHORITY_ESCALATION')

    def test_test_binding(self):
        self.mutate(lambda d:d['routes'][0].update(test_refs=['ST-033']),'TEST_BINDING')

    def test_fixture_cannot_be_promoted(self):
        cat=copy.deepcopy(self.cat)
        cat['source_tests'][0][5]='fixture_only_excluded'
        with self.assertRaisesRegex(v.Invalid,'^TEST_AUTHORITY$'):
            self.good(cat=cat)

    def test_hidden_variant_removed(self):
        self.mutate(lambda d:d.update(variants=[x for x in d['variants'] if 'hidden' not in x['selector']]),'VARIANT_SET')

    def test_duplicate_variant(self):
        self.mutate(lambda d:d['variants'].append(d['variants'][0]),'VARIANT_SET')

    def test_span_tampering(self):
        self.mutate(lambda d:d['profiles']['resolve']['anchors'][0].__setitem__(3,'0'*64),'SPAN_DIGEST')

    def test_anchor_out_of_range(self):
        self.mutate(lambda d:d['profiles']['resolve']['anchors'][0].__setitem__(2,999999),'SPAN_DIGEST')

    def test_missing_anchor(self):
        self.mutate(lambda d:d['profiles']['resolve'].update(anchors=[]),'ANCHOR_MISSING')

    def test_workspace_omitted(self):
        self.mutate(lambda d:d['workspace_libraries'].pop(),'WORKSPACE_SET')

    def test_feature_erased(self):
        def change(d):
            next(x for x in d['workspace_libraries'] if x['features'])['features']={}
        self.mutate(change,'LIBRARY_CLASSIFICATION')

    def test_generator_omitted(self):
        self.mutate(lambda d:d.update(workspace_binaries=[x for x in d['workspace_binaries'] if x['name']!='gen_fixtures']),'BINARY_SET')

    def test_generator_path_changed(self):
        self.mutate(lambda d:d['workspace_binaries'][0].update(path='Cargo.toml'),'BINARY_PATH')

    def test_source_corrupt(self):
        inv=copy.deepcopy(self.inv)
        path=self.doc['profiles']['resolve']['anchors'][0][0]
        next(x for x in inv['files'] if x['path']==path)['sha256']='0'*64
        with self.assertRaisesRegex(v.Invalid,'^SOURCE_DIGEST$'):
            self.good(inv=inv)

    def test_byte_identical_rerun(self):
        self.assertEqual(json.dumps(self.good(),sort_keys=True),json.dumps(self.good(),sort_keys=True))

    def test_duplicate_json(self):
        with self.assertRaisesRegex(v.Invalid,'^JSON_DUPLICATE_KEY$'):
            json.loads('{"a":1,"a":2}',object_pairs_hook=v.pairs)

    def test_unsafe_paths(self):
        for p in ('../outside','/absolute','a/./b','a//b','a\\b','a\0b','a/'*65+'b'):
            with self.subTest(path=repr(p)),self.assertRaises(v.Invalid):
                v.parts(p)

    def test_nofollow_and_limits(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            (root/'file').write_bytes(b'hello')
            self.assertEqual(v.read_under(root,'file',5),b'hello')
            with self.assertRaisesRegex(v.Invalid,'^FILE_TYPE_OR_LIMIT$'):
                v.read_under(root,'file',4)
            (root/'link').symlink_to(root/'file')
            with self.assertRaisesRegex(v.Invalid,'^FILE_UNAVAILABLE$'):
                v.read_under(root,'link',20)
            (root/'dir').mkdir(); (root/'dir'/'f').write_bytes(b'x')
            (root/'alias').symlink_to(root/'dir',target_is_directory=True)
            with self.assertRaisesRegex(v.Invalid,'^FILE_UNAVAILABLE$'):
                v.read_under(root,'alias/f',20)
            if hasattr(os,'mkfifo'):
                os.mkfifo(root/'fifo')
                with self.assertRaisesRegex(v.Invalid,'^FILE_TYPE_OR_LIMIT$'):
                    v.read_under(root,'fifo',20)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=Path('.'))
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--basis',type=Path,required=True)
    ARGS,rest=parser.parse_known_args()
    unittest.main(argv=[__file__]+rest)
