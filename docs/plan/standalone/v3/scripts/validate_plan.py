#!/usr/bin/env python3
"""Offline v3 plan validator. Valid structure is not proof of product correctness.
No network; no implicit writes other than --output. Exit 0 valid plan, 1 invalid/not-ready, 2 setup.
"""
from __future__ import annotations
import argparse,hashlib,json,re,sys
from collections import defaultdict
from pathlib import Path,PurePosixPath
from typing import Any
sys.path.insert(0,str(Path(__file__).resolve().parent))
import render_plan
AXIOMS={'completeness_criteria','success_criteria','quality_standards','dod','invariants'}
WPS={f'HUG-{i:03}' for i in range(1,61)}
TESTS={f'T-HUG-{i:03}-{s}' for i in range(1,61) for s in 'PNQR'}|{f'T-AR-{i:02}-C' for i in range(1,17)}|{f'T-BR-{i:02}-C' for i in range(1,11)}|{'T-HUG-056-POST'}
PROS={f'P{i:02}' for i in range(13)}
def canonical(x:Any)->bytes:return json.dumps(x,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()
def sha(x:bytes)->str:return hashlib.sha256(x).hexdigest()
def is_sha(s:Any,n=64)->bool:return isinstance(s,str) and bool(re.fullmatch('[a-f0-9]{'+str(n)+'}',s))
def safe_path(s:Any)->bool:
 return isinstance(s,str) and bool(s) and not PurePosixPath(s).is_absolute() and '..' not in PurePosixPath(s).parts and '\\' not in s and '\x00' not in s

def validate(p:dict)->dict:
 errors=[]
 def ck(ok,code,msg):
  if not ok:errors.append({'code':code,'message':msg})
 counts={}
 try:
  ck(p.get('schema')=='hugit-standalone-plan/3','SCHEMA','Expected v3 schema')
  ck(p.get('plan_version')=='3.0','PLAN_VERSION','Expected revision 3.0')
  ck(is_sha(p.get('source_commit'),40),'SOURCE_COMMIT','Baseline must be full 40-hex Git commit, not branch/text')
  ck(p.get('standalone') is True and p.get('external_service_required') is False,'STANDALONE','Standalone invariant')
  ck(p.get('scope')=='plan_revision_only' and p.get('code_changes_made') is False,'SCOPE','This delivery only edits the plan')
  ck(p.get('quality_waivers_allowed') is False and set(p.get('required_axioms',[]))==AXIOMS,'WAIVER','Five mandatory axioms')
  def idx(seq,key,code):
   result={}
   ck(isinstance(seq,list),code,'Expected list')
   for x in seq:
    ck(isinstance(x,dict) and isinstance(x.get(key),str),code,'Invalid identity')
    if not isinstance(x,dict) or not isinstance(x.get(key),str):continue
    ck(x[key] not in result,code,'Duplicate '+x[key]);result[x[key]]=x
   return result
  tasks=idx(p['tasks'],'id','WP_ID');tests=idx(p['tests'],'id','TEST_ID');pro=idx(p['protocol_registry'],'id','PROTOCOL_ID');ass=idx(p['assertions'],'id','ASSERTION_ID')
  ck(set(tasks)==WPS,'WP_COVERAGE','Exactly HUG001..060')
  ck(set(tests)==TESTS,'TEST_COVERAGE','All original families, AR/BR closures and POST required')
  ck(set(pro)==PROS,'PROTOCOL_COVERAGE','P00..P12 required')
  for k,x in pro.items():
   ck(x.get('version')=='3.0','PROTOCOL_VERSION',k)
   ck(bool(x.get('normative_body')) and bool(x.get('rules')),'PROTOCOL_BODY',k)
   ck(x.get('digest')==sha(canonical({a:b for a,b in x.items() if a!='digest'})),'PROTOCOL_DIGEST',k)
   ck(all(r.get('mandatory') is True and r.get('decision') for r in x['rules']),'PROTOCOL_RULE',k)
  ck(p.get('protocol_set_digest')==sha(canonical({k:x['digest'] for k,x in pro.items()})),'PROTOCOL_SET','Protocol set mismatch')
  graph={k:set(t.get('depends_on',[])) for k,t in tasks.items()};done=set();waves=[];anc={}
  for k,ds in graph.items():ck(ds<=WPS and k not in ds and len(ds)==len(tasks[k]['depends_on']),'DEPENDENCY',k)
  while len(done)<len(tasks):
   w=sorted(k for k,ds in graph.items() if k not in done and ds<=done)
   if not w:break
   for k in w:anc[k]=graph[k]|set().union(*(anc.get(d,set()) for d in graph[k]))
   done.update(w);waves.append(w)
  ck(done==set(tasks),'DAG_CYCLE','Dependency cycle')
  order=p.get('topological_order',[]);pos={k:i for i,k in enumerate(order)}
  ck(len(order)==60 and set(order)==WPS and all(pos.get(d,9999)<pos.get(k,-1) for k,ds in graph.items() for d in ds),'TOPO_ORDER','Topological order invalid')
  policy=p['admission_policy'];required=set(policy['required_consumer_ids']);fp=policy['freeze_producer']
  ck(fp=='HUG-059','FREEZE_PRODUCER','Wrong freeze producer')
  ck(required=={k for k,t in tasks.items() if t.get('requires_runtime_protocol_freeze') is True},'FREEZE_FLAGS','Consumers and policy disagree')
  for k in required:ck(fp in anc.get(k,set()),'FREEZE_ANCESTOR',k+' declares runtime freeze without ancestor')
  for q in ['source_discovery_prerequisite','migration_backup_prerequisite']:
   d=policy[q];ck(d['producer'] in anc.get(d['consumer'],set()),'SEMANTIC_DEPENDENCY',str(d))
  ck(anc.get('HUG-056',set())==WPS-{'HUG-056'},'FULL_CLOSURE','HUG056 must cover 59 predecessors')
  crit={};artifacts={};paths=defaultdict(list);rollback_texts=[]
  for k,t in tasks.items():
   ck(t['status']=='planned' and t['closure_status']=='not_implemented_not_verified','PLAN_EXECUTION_STATE',k+' falsely marked executed in plan delivery')
   ck(set(t['axioms'])==AXIOMS,'AXIOM_GROUPS',k)
   ck(bool(t.get('owner_lane')) and bool(t.get('write_set')),'OWNERSHIP',k)
   ck(t.get('write_set_status')=='must_be_bound_to_real_source_by_HUG-001_before_dispatch','PATH_BINDING',k)
   ck(set(t['write_set'])=={d['path'] for d in t['deliverables'] if d.get('required')},'DELIVERABLES',k)
   for path in t['write_set']:
    ck(safe_path(path) and not any(ch in path for ch in '*?['),'WRITE_PATH',k+': '+str(path));paths[path].append(k)
    ck('path:'+path in t['exclusive_resources'],'WRITE_LEASE',k+': '+path)
   expected=[{'id':id,'version':pro[id]['version'],'digest':pro[id]['digest']} for id in t['protocols'] if id in pro]
   ck(t['protocol_bindings']==expected and len(expected)==len(t['protocols']),'CONSUMER_BINDING',k)
   ck('P00' in t['protocols'],'PROTOCOL_REFERENCE',k)
   own={f'T-{k}-{s}' for s in 'PNQR'}
   ck(own<=set(t['acceptance_tests']) and set(t['acceptance_tests'])<=set(tests),'WP_TESTS',k)
   for tid in own:ck(tests[tid]['owner_package']==k,'TEST_OWNER',tid)
   rb=t.get('rollback',{});rollback_texts.append(re.sub(r'HUG-\d{3}','WP',rb.get('procedure','')));ck(bool(rb.get('procedure')) and rb.get('test_ref')==f'T-{k}-R' and rb.get('preserve_evidence') is True and rb.get('automatic_replay_of_external_side_effects') is False,'RECOVERY',k)
   for axis,cs in t['axioms'].items():
    ck(isinstance(cs,list) and bool(cs),'AXIOM_EMPTY',k+': '+axis)
    for c in cs:
     cid=c.get('id');ck(cid not in crit and isinstance(cid,str) and cid.startswith(k+'-'),'CRITERION_ID',str(cid));crit[cid]=c
     ck(c.get('mandatory') is True,'CRITERION_WAIVER',str(cid))
     ck(isinstance(c.get('statement'),str) and len(c['statement'].strip())>=20 and c['statement'].strip().lower() not in ['n/a','tbd','todo','não aplicável'],'CRITERION_TEXT',str(cid))
     refs=c.get('assertion_refs',[]);ck(bool(refs) and len(set(refs))==len(refs) and set(refs)<=set(ass),'CRITERION_ASSERTIONS',str(cid))
     family=set();arts=set()
     for aid in refs:
      if aid not in ass:continue
      a=ass[aid];ck(cid in a['criterion_ids'] and a['owner_package']==k,'ASSERTION_CRITERION_BINDING',aid)
      ck(a.get('criterion_digest')==sha(canonical({'id':cid,'statement':c['statement']})),'ASSERTION_CRITERION_DIGEST',aid)
      family.add(a['test_id']);arts.add(tests[a['test_id']]['artifact']+'#assertions/'+aid)
     ck(set(c.get('test_refs',[]))==family,'CRITERION_TEST_BINDING',str(cid))
     ck(set(c.get('evidence_artifacts',[]))==arts,'CRITERION_ARTIFACT',str(cid))
  ck(len(rollback_texts)==len(set(rollback_texts)),'GENERIC_ROLLBACK','Recovery duplicated across WPs')
  for tid,t in tests.items():
   ck(t['owner_package'] in WPS,'TEST_OWNER',tid)
   if tid.startswith('T-HUG-'):ck(t['owner_package']==tid[2:9],'TEST_OWNER',tid)
   ck(t.get('status')=='not_run','TEST_EXECUTION_STATE',tid+' claims runtime execution')
   ck(bool(t.get('procedure')) and bool(t.get('expected')) and bool(t.get('preconditions')),'TEST_SPEC',tid)
   artifact=t['artifact'];ck(safe_path(artifact) and artifact.startswith('evidence/'),'EVIDENCE_PATH',tid)
   ck(artifact not in artifacts,'EVIDENCE_COLLISION',tid+' shares a result path');artifacts[artifact]=tid
   ars=t.get('assertion_ids',[]);ck(bool(ars) and len(set(ars))==len(ars),'TEST_ASSERTIONS',tid)
   for aid in ars:ck(aid in ass and ass[aid]['test_id']==tid,'ASSERTION_OWNER',tid+': '+aid)
  for aid,a in ass.items():
   ck(a['mandatory'] is True and a['status']=='specified_not_executed','ASSERTION_STATUS',aid)
   ck(a['test_id'] in tests and aid in tests[a['test_id']]['assertion_ids'],'ASSERTION_TEST',aid)
   ck(bool(a.get('given')) and bool(a.get('when')) and bool(a.get('then',{}).get('predicate')),'ASSERTION_PREDICATE',aid)
   ck(a.get('oracle_digest')==sha(canonical({k:v for k,v in a.items() if k not in ['oracle_digest','status']})),'ASSERTION_ORACLE_DIGEST',aid)
   for cid in a['criterion_ids']:ck(cid in crit and aid in crit[cid]['assertion_refs'],'ASSERTION_REVERSE_TRACE',aid+': '+cid)
  # Review regressions protect the specific assertion, not just a family label.
  a=ass.get('A-HUG-036-INV01',{});ck(a.get('test_id')=='T-HUG-036-P' and a.get('then',{}).get('predicate',{}).get('equals')=={'asked':4,'done':0,'proven':0},'LEDGER_ASSERTION','BR02 exact ledger oracle missing')
  a=ass.get('A-HUG-017-INV03',{});ck(a.get('test_id')=='T-HUG-017-P' and a.get('then',{}).get('predicate',{}).get('equals')=={'logical_occurrences':2,'invocations':2,'policy_bindings':2,'unauthorized_reads':0,'cross_domain_leaks':0},'POLICY_DEDUPE_ASSERTION','BR02 policy scenario missing')
  caps=idx(p['capabilities'],'id','CAPABILITY_ID');gates=idx(p['gates'],'id','GATE_ID')
  scope=p['scope_contract'];ck(set(scope['required_package_ids'])==WPS and set(scope['required_capability_ids'])==set(caps),'SCOPE_DENOMINATOR','Full denominator changed/incomplete')
  profiles=p['profiles'];ck(set(profiles)=={'full','early-slice'},'PROFILES','Missing profiles')
  f=profiles['full'];ck(set(f['required_packages'])==WPS and set(f['capabilities'])==set(scope['required_capability_ids']) and len(f['capabilities'])>0,'FULL_CAPABILITIES','Full must include all capabilities')
  ck(set(f['required_tests'])==set(tests),'FULL_TESTS','Full tests incomplete')
  ck(set(f['required_gates'])=={f'G{i}' for i in range(10)},'FULL_GATES','Full gates incomplete')
  banned={f'HUG-{i:03}' for i in [18,31,40,41,42,43,44,48,49,51,52,53,54,55,56]}
  ck(not anc.get('HUG-060',set())&banned,'EARLY_VALUE','Slice waits for specialized/final delivery work')
  for pname,prof in profiles.items():
   ck(prof.get('quality_waivers_allowed') is False,'PROFILE_WAIVER',pname)
   ck(set(prof['required_tests'])<=set(tests),'PROFILE_TESTS',pname)
   ck(set(prof['required_packages'])<=WPS,'PROFILE_PACKAGES',pname)
   for w in prof['required_packages']:ck(graph[w]<=set(prof['required_packages']),'PROFILE_DEPENDENCY',pname+': '+w)
   for capid in prof['capabilities']:
    ck(capid in caps,'CAP_UNKNOWN',pname+': '+capid)
    if capid in caps:ck(set(caps[capid]['required_packages'])<=set(prof['required_packages']) and set(caps[capid]['required_tests'])<=set(prof['required_tests']),'CAP_PROFILE_COVERAGE',pname+': '+capid)
  for co in [caps,gates]:
   for k,x in co.items():
    ck(bool(x['required_packages']) and set(x['required_packages'])<=WPS and bool(x['required_tests']) and set(x['required_tests'])<=set(tests),'CAP_GATE_COVERAGE',k)
  cells=idx(p['matrix_cells'],'id','CELL_ID');obs=idx(p['obligations'],'id','OBLIGATION_ID')
  expobs={}
  for tid,t in tests.items():
   ck(bool(t.get('required_cells')) and len(set(t['required_cells']))==len(t['required_cells']) and set(t['required_cells'])<=set(cells),'TEST_CELLS',tid)
   for cell in t['required_cells']:
    expobs[tid+'@'+cell]={'id':tid+'@'+cell,'test_id':tid,'cell_id':cell,'stage':t['stage'],'required_assertions':t['assertion_ids'],'mandatory':True}
  ck(obs==expobs,'MATRIX_DENOMINATOR','Obligation cells/assertions differ from catalog')
  ck(set(f['required_obligations'])==set(obs),'FULL_OBLIGATIONS','Full cell list incomplete')
  ck(set(f['pre_promotion_tests'])=={k for k,t in tests.items() if t['stage']=='pre_promotion'},'RELEASE_PRE','Pre stage test set')
  ck(set(f['post_publication_tests'])=={'T-HUG-056-POST'},'RELEASE_POST','Post verification missing')
  ck(p['release_lifecycle']['pre_promotion']['requires_HUG056_closed'] is False and p['release_lifecycle']['post_publication']['requires_HUG056_closed'] is True and p['release_lifecycle']['evidence_changes_subject'] is False,'RELEASE_CYCLE','A/B/C or stage circularity')
  for field,prefix,n in [('review_resolutions','AR',16),('second_review_resolutions','BR',10)]:
   rr=idx(p[field],'id','REVIEW_ID');ck(set(rr)=={f'{prefix}-{i:02}' for i in range(1,n+1)},'REVIEW_COVERAGE',prefix)
   for rid,r in rr.items():
    ck(bool(r['packages']) and set(r['packages'])<=WPS and bool(r['protocols']) and set(r['protocols'])<=PROS,'REVIEW_OWNERS',rid)
    ck(r['no_waiver'] is True and r['acceptance_test'] in tests and tests[r['acceptance_test']]['expected']==r['original_acceptance'],'REVIEW_ACCEPTANCE',rid)
    if prefix=='BR':ck(r['original_section_sha256']==sha(r['original_section'].encode()),'REVIEW_SOURCE',rid)
    refs='review2_refs' if prefix=='BR' else 'review_refs'
    for k in r['packages']:ck(rid in tasks[k][refs],'REVIEW_TRACE',rid+': '+k)
  counts={'work_packages':len(tasks),'criteria':len(crit),'assertions':len(ass),'test_families':len(tests),'obligation_cells':len(obs),'protocols':len(pro),'capabilities':len(caps)}
  return {'schema':'hugit-plan-validation/3','valid':not errors,'errors':errors,'counts':counts,'ancestor_counts':{k:len(v) for k,v in anc.items()},'write_conflicts_require_serialization':{k:v for k,v in paths.items() if len(v)>1},'scope':'plan_structure_not_product_proof'}
 except (KeyError,TypeError,ValueError,AttributeError) as e:
  errors.append({'code':'STRUCTURE_ERROR','message':type(e).__name__+': '+str(e)})
  return {'valid':False,'errors':errors,'counts':counts,'scope':'plan_structure_not_product_proof'}

def validate_directory(root:Path,p:dict)->list:
 errors=[]
 try:expected=render_plan.all_views(p)
 except Exception as e:return [{'code':'RENDER_ERROR','message':str(e)}]
 for name,text in expected.items():
  path=root/name
  if path.is_symlink() or any(x.is_symlink() for x in path.parents if x!=root.parent):errors.append({'code':'NORMATIVE_SYMLINK','message':name});continue
  try:actual=path.read_text()
  except OSError:errors.append({'code':'NORMATIVE_FILE_MISSING','message':name});continue
  if actual!=text:errors.append({'code':'NORMATIVE_VIEW_DRIFT','message':name+' differs from canonical renderer'})
 for d in p['source_documents']:
  path=root/d['path']
  if not safe_path(d['path']) or path.is_symlink() or not path.is_file() or sha(path.read_bytes())!=d['sha256']:errors.append({'code':'SOURCE_HASH','message':d['path']})
 return errors

# Deterministic multi-cell preflight. It intentionally never grants release permission.
def aggregate(p:dict,ledger:dict,profile='full',stage=None)->dict:
 required=[o for o in p['obligations'] if o['id'] in p['profiles'][profile]['required_obligations'] and (stage is None or o['stage']==stage)]
 expected={(o['test_id'],a,o['cell_id']):o for o in required for a in o['required_assertions']}
 subject=ledger.get('subject_digest');plan_d=ledger.get('plan_digest');proto=ledger.get('protocol_set_digest');lock_d=ledger.get('qualification_lock_digest')
 errors=[];groups=defaultdict(list);ids={};index={x['id']:x for x in p['matrix_cells']};qlock=ledger.get('qualification_lock',{})
 if plan_d!=sha(canonical(p)):errors.append({'code':'PLAN_DIGEST_MISMATCH'})
 if proto!=p.get('protocol_set_digest'):errors.append({'code':'PROTOCOL_SET_MISMATCH'})
 if not qlock or sha(canonical(qlock))!=lock_d:errors.append({'code':'QUALIFICATION_LOCK_MISMATCH'})
 required_hashes={'subject_digest':subject,'plan_digest':plan_d,'protocol_set_digest':proto,'qualification_lock_digest':lock_d}
 for key,value in required_hashes.items():
  if not is_sha(value):errors.append({'code':'MISSING_BINDING','key':key})
 for r in ledger.get('assertion_results',[]):
  key=(r.get('test_id'),r.get('assertion_id'),r.get('cell_id'))
  if key not in expected:errors.append({'code':'UNEXPECTED_OBLIGATION','key':list(key)});continue
  ok=True
  for field,val in required_hashes.items():
   if r.get(field)!=val or not is_sha(r.get(field)):errors.append({'code':'EVIDENCE_BINDING_MISMATCH','key':list(key),'field':field});ok=False
  cell=index[key[2]];env=r.get('environment',{})
  if not env or env!=qlock.get(key[2]) or any(env.get(f)!=cell[f] for f in ['target','filesystem','adapter','configuration']) or not is_sha(env.get('environment_digest')) or not all(env.get(f) for f in ['os_version','tool_versions','executable_digests']):
   errors.append({'code':'ENVIRONMENT_MISMATCH','key':list(key)});ok=False
  if not isinstance(r.get('attempt_id'),str) or not r['attempt_id']:errors.append({'code':'ATTEMPT_MISSING','key':list(key)});ok=False
  ident=key+(r.get('attempt_id'),);fingerprint=canonical(r)
  if ident in ids:
   if ids[ident]!=fingerprint:errors.append({'code':'CONFLICTING_DUPLICATE','key':list(key)});ok=False;groups[key].append({'status':'conflict'})
   else:continue
  ids[ident]=fingerprint
  if ok:groups[key].append(r)
 missing=[];failed=[];passed=[]
 for key in sorted(expected):
  rs=groups[key]
  if not rs:missing.append(list(key))
  elif any(r.get('status')!='passed' for r in rs):failed.append(list(key))
  else:passed.append(list(key))
 errors=[json.loads(x) for x in sorted({canonical(e).decode() for e in errors})]
 return {'ready':False,'status':'not_ready' if missing or failed or errors else 'external_verification_required','structurally_complete':not (missing or failed or errors),'required_assertion_cells':len(expected),'passed_count':len(passed),'pending_count':len(missing)+len(failed),'missing':missing,'failed':failed,'errors':errors,'scope':'informational_preflight_not_authorization','reason':'Requires real executor evidence, artifact digests, approved oracle and independently verified approvals.'}

def readiness(p,ledger):return aggregate(p,ledger)

def main()->int:
 ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);ap.add_argument('--output',type=Path);ap.add_argument('--mode',choices=['plan','release-readiness'],default='plan');a=ap.parse_args()
 try:
  p=json.loads((a.root/'backlog.json').read_text());r=validate(p)
  if r['valid']:r['errors']+=validate_directory(a.root,p);r['valid']=not r['errors']
  if a.mode=='release-readiness' and r['valid']:r=aggregate(p,json.loads((a.root/'evidence-ledger.json').read_text()))
  code=0 if r.get('valid') else 1
 except (OSError,ValueError) as e:r={'valid':False,'errors':[{'code':'INPUT_ERROR','message':str(e)}]};code=2
 if a.output:a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(r,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps(r,ensure_ascii=False,indent=2));return code
if __name__=='__main__':raise SystemExit(main())
