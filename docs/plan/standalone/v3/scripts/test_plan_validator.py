#!/usr/bin/env python3
"""Adversarial tests of this plan's validator. Not product test evidence."""
from __future__ import annotations
import argparse,copy,json,tempfile,shutil,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import validate_plan as v

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);ap.add_argument('--part',type=int,choices=range(4));a=ap.parse_args();p=json.loads((a.root/'backlog.json').read_text());cases=[]
 def task(q,k):return next(t for t in q['tasks'] if t['id']==k)
 counter=0
 def mutate(name,fn,expect_code=None):
  nonlocal counter
  slot=counter%4;counter+=1
  if a.part is not None and slot!=a.part:return
  q=copy.deepcopy(p);fn(q);r=v.validate(q);codes={e['code'] for e in r['errors']};passed=not r['valid'] and (not expect_code or expect_code in codes)
  cases.append({'case':name,'expected':'reject','detected':passed,'codes':sorted(codes)})
 r=v.validate(p);cases.append({'case':'control_valid_source','expected':'accept','detected':r['valid']})
 for t in p['tasks']:
  for axis in sorted(v.AXIOMS):mutate('remove_'+t['id']+'_'+axis,lambda q,k=t['id'],ax=axis:task(q,k)['axioms'].pop(ax),'AXIOM_GROUPS')
 for t in p['tasks']:
  if t['requires_runtime_protocol_freeze']:
   mutate('freeze_every_consumer_'+t['id'],lambda q,k=t['id']:task(q,k).__setitem__('depends_on',[]),'FREEZE_ANCESTOR')
 for i in range(13):mutate('protocol_version_'+str(i),lambda q,i=i:q['protocol_registry'][i].__setitem__('version','99.0'),'PROTOCOL_VERSION')
 for name,fn,code in [
  ('baseline_invalid',lambda q:q.__setitem__('source_commit','not-a-commit'),'SOURCE_COMMIT'),
  ('owner_wrong',lambda q:q['tests'][0].__setitem__('owner_package','HUG-060'),'TEST_OWNER'),
  ('full_capabilities_empty',lambda q:q['profiles']['full'].__setitem__('capabilities',[]),'FULL_CAPABILITIES'),
  ('full_041_removed',lambda q:q['profiles']['full']['required_packages'].remove('HUG-041'),'FULL_CAPABILITIES'),
  ('two_families_one_path',lambda q:q['tests'][-1].__setitem__('artifact',q['tests'][-2]['artifact']),'EVIDENCE_COLLISION'),
  ('consumer_digest_wrong',lambda q:task(q,'HUG-020')['protocol_bindings'][1].__setitem__('digest','0'*64),'CONSUMER_BINDING'),
  ('assertion_removed_suite_still_present',lambda q:q['assertions'].remove(next(z for z in q['assertions'] if z['id']=='A-HUG-036-INV01')),'CRITERION_ASSERTIONS'),
  ('criterion_statement_changed',lambda q:task(q,'HUG-036')['axioms']['invariants'][0].__setitem__('statement','Goals criados passam a indicar todo trabalho concluído.'),'ASSERTION_CRITERION_DIGEST'),
  ('wave_cycle',lambda q:task(q,'HUG-001')['depends_on'].append('HUG-060'),'DAG_CYCLE'),
  ('post_release_omitted',lambda q:q['profiles']['full'].__setitem__('post_publication_tests',[]),'RELEASE_POST'),
  ('circular_closed_before_publish',lambda q:q['release_lifecycle']['pre_promotion'].__setitem__('requires_HUG056_closed',True),'RELEASE_CYCLE'),
  ('evidence_inside_subject',lambda q:q['release_lifecycle'].__setitem__('evidence_changes_subject',True),'RELEASE_CYCLE'),
  ('cell_missing_from_family',lambda q:next(t for t in q['tests'] if t['id']=='T-HUG-049-P')['required_cells'].remove('windows-x64-ntfs'),'MATRIX_DENOMINATOR'),
  ('mutated_protocol_body',lambda q:q['protocol_registry'][2].__setitem__('normative_body',q['protocol_registry'][2]['normative_body']+'\nGC may delete pinned objects.'),'PROTOCOL_DIGEST'),
  ('wrong_assertion_owner',lambda q:next(x for x in q['assertions'] if x['id']=='A-HUG-036-INV01').__setitem__('owner_package','HUG-060'),'ASSERTION_CRITERION_BINDING'),
 ]:mutate(name,fn,code)
 def predicate(q,id,field,val):
  x=next(z for z in q['assertions'] if z['id']==id);x['then']['predicate']['equals'][field]=val;x['oracle_digest']=v.sha(v.canonical({k:w for k,w in x.items() if k not in ['oracle_digest','status']}))
 mutate('ledger_done_true_not_caught_by_family_label',lambda q:predicate(q,'A-HUG-036-INV01','done',4),'LEDGER_ASSERTION')
 mutate('cross_policy_assertion_weakened',lambda q:predicate(q,'A-HUG-017-INV03','unauthorized_reads',1),'POLICY_DEDUPE_ASSERTION')
 for axis in sorted(v.AXIOMS):
  mutate('legacy_empty_'+axis,lambda q,ax=axis:task(q,'HUG-017')['axioms'].__setitem__(ax,[]),'AXIOM_EMPTY')
  mutate('legacy_waive_'+axis,lambda q,ax=axis:task(q,'HUG-017')['axioms'][ax][0].__setitem__('mandatory',False),'CRITERION_WAIVER')
 for name,fn,code in [
  ('legacy_terminal_loses_041',lambda q:[t.__setitem__('depends_on',[d for d in t['depends_on'] if d!='HUG-041']) for t in q['tasks']],'FULL_CLOSURE'),
  ('legacy_unknown_dependency',lambda q:task(q,'HUG-002')['depends_on'].append('HUG-999'),'DEPENDENCY'),
  ('legacy_unknown_test_reference',lambda q:task(q,'HUG-018')['axioms']['dod'][0]['test_refs'].append('T-NOT-THERE'),'CRITERION_TEST_BINDING'),
  ('legacy_evidence_missing',lambda q:task(q,'HUG-039')['axioms']['dod'][0].__setitem__('evidence_artifacts',[]),'CRITERION_ARTIFACT'),
  ('legacy_generic_rollback',lambda q:task(q,'HUG-017')['rollback'].__setitem__('procedure',task(q,'HUG-016')['rollback']['procedure']),'GENERIC_ROLLBACK'),
  ('legacy_missing_write_lease',lambda q:task(q,'HUG-016').__setitem__('exclusive_resources',[]),'WRITE_LEASE'),
  ('legacy_slice_waits_docker',lambda q:task(q,'HUG-060')['depends_on'].append('HUG-040'),'EARLY_VALUE'),
  ('legacy_goal_discovery_removed',lambda q:task(q,'HUG-015')['depends_on'].remove('HUG-057'),'SEMANTIC_DEPENDENCY'),
  ('legacy_bootstrap_backup_removed',lambda q:task(q,'HUG-018')['depends_on'].remove('HUG-058'),'SEMANTIC_DEPENDENCY'),
  ('legacy_review_expected_changed',lambda q:q['review_resolutions'][0].__setitem__('original_acceptance','Changed'),'REVIEW_ACCEPTANCE'),
  ('legacy_review_finding_dropped',lambda q:q['review_resolutions'].pop(),'REVIEW_COVERAGE'),
  ('legacy_full_no_tests',lambda q:q['profiles']['full'].__setitem__('required_tests',[]),'FULL_TESTS'),
  ('legacy_cap_unknown_package',lambda q:q['capabilities'][0]['required_packages'].append('HUG-999'),'CAP_GATE_COVERAGE'),
  ('legacy_slice_claims_mcp',lambda q:q['profiles']['early-slice']['capabilities'].append('evidence_read'),'CAP_PROFILE_COVERAGE'),
  ('legacy_waivers_enabled',lambda q:q.__setitem__('quality_waivers_allowed',True),'WAIVER'),
  ('legacy_external_dependency',lambda q:q.__setitem__('external_service_required',True),'STANDALONE'),
  ('legacy_path_traversal',lambda q:task(q,'HUG-001')['write_set'].append('../outside'),'WRITE_PATH'),
  ('legacy_skipped_test',lambda q:q['tests'][0].__setitem__('status','skipped'),'TEST_EXECUTION_STATE'),
  ('legacy_false_code_change_claim',lambda q:q.__setitem__('code_changes_made',True),'SCOPE'),
  ('legacy_empty_expected',lambda q:q['tests'][0].__setitem__('expected',''),'TEST_SPEC'),
  ('legacy_missing_full_gate',lambda q:q['profiles']['full']['required_gates'].remove('G4'),'FULL_GATES'),
 ]:mutate(name,fn,code)
 with tempfile.TemporaryDirectory(prefix='hugit-v3-validator-') as td:
  root=Path(td)/'plan';shutil.copytree(a.root,root,ignore=shutil.ignore_patterns('validation','__pycache__'))
  for name,path,action in [('protocol_missing','contratos/P02.md','remove'),('protocol_contradiction','contratos/P02.md','append'),('wp_dependency_prose_drift','work-packages/HUG-020.md','replace'),('sidecar_capability_drift','capabilities.json','append')]:
   fp=root/path;old=fp.read_bytes()
   if action=='remove':fp.unlink()
   elif action=='append':fp.write_bytes(old+b'\nNORMATIVE CONFLICT\n')
   else:fp.write_bytes(old.replace(b'HUG-059',b'HUG-999'))
   errs=v.validate_directory(root,p);codes=sorted({x['code'] for x in errs});cases.append({'case':name,'expected':'reject','detected':bool(errs),'codes':codes});fp.write_bytes(old)
  cases.append({'case':'control_restored_views','expected':'accept','detected':not v.validate_directory(root,p)})
 report={'scope':'plan_validator_only_not_product','canonical_plan_digest':v.sha(v.canonical(p)),'validator_sha256':v.sha(Path(v.__file__).read_bytes()),'cases':cases,'passed':sum(c['detected'] for c in cases),'total':len(cases),'all_passed':all(c['detected'] for c in cases)}
 out=a.root/('validation/mutation-tests-v3.json' if a.part is None else f'validation/mutation-tests-part{a.part}.json');out.parent.mkdir(exist_ok=True);out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['scope','passed','total','all_passed']},indent=2));return 0 if report['all_passed'] else 1
if __name__=='__main__':raise SystemExit(main())
