#!/usr/bin/env python3
"""Order-independent matrix attacks plus a REAL subprocess/signed A→B→C fixture.
No Hugit execution, remote release, production key or independent human reviewer.
"""
from __future__ import annotations
import argparse,base64,copy,hashlib,json,os,platform,shutil,subprocess,sys,tempfile
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import validate_plan as v
try:
 from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
 from cryptography.hazmat.primitives import serialization
 import qualification_reference as ref
except ImportError as e:
 print(json.dumps({'status':'setup_error','detail':str(e),'note':'No auto installation; this does not pass'}));raise SystemExit(2)

def matrix_fixture(p):
 q=copy.deepcopy(p);family='T-HUG-049-P';cells=['linux-x64-ext4','windows-x64-ntfs']
 q['profiles']['full']['required_obligations']=[family+'@'+c for c in cells]
 required=[o for o in q['obligations'] if o['id'] in q['profiles']['full']['required_obligations']]
 cs={c['id']:c for c in q['matrix_cells']}
 lock={}
 for cid in cells:
  c=cs[cid];identity={k:c[k] for k in ['target','filesystem','adapter','configuration']}
  identity.update(os_version='SYNTHETIC-MATRIX-FIXTURE-NOT-OS-EXECUTION',tool_versions={'fixture':'1'},executable_digests={'fixture':'a'*64});identity['environment_digest']=v.sha(v.canonical(identity));lock[cid]=identity
 ld=v.sha(v.canonical(lock));pd=v.sha(v.canonical(q));subject='b'*64
 ledger={'subject_digest':subject,'plan_digest':pd,'protocol_set_digest':q['protocol_set_digest'],'qualification_lock_digest':ld,'qualification_lock':lock,'assertion_results':[]}
 for o in required:
  for aid in o['required_assertions']:
   ledger['assertion_results'].append({'subject_digest':subject,'plan_digest':pd,'protocol_set_digest':q['protocol_set_digest'],'test_id':o['test_id'],'assertion_id':aid,'cell_id':o['cell_id'],'qualification_lock_digest':ld,'attempt_id':'fixture-attempt','environment':lock[o['cell_id']],'status':'passed'})
 return q,ledger

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);a=ap.parse_args();p=json.loads((a.root/'backlog.json').read_text());q,l=matrix_fixture(p);cases=[]
 def ck(name,ok,detail=None):cases.append({'case':name,'passed':bool(ok),'detail':detail});assert ok,(name,detail)
 positive=v.aggregate(q,l);ck('all_synthetic_cells_complete_but_not_release_authorized',positive['structurally_complete'] and positive['ready'] is False)
 failed=copy.deepcopy(l)
 for r in failed['assertion_results']:
  if r['cell_id']=='windows-x64-ntfs':r['status']='failed'
 first=v.aggregate(q,failed);failed['assertion_results'].reverse();second=v.aggregate(q,failed)
 ck('Windows_failed_Linux_passed_order_independent',first==second and first['pending_count']>0,{'order1_pending':first['pending_count'],'order2_pending':second['pending_count'],'ready':first['ready']})
 for name,change in [
  ('missing_cell',lambda x:x.__setitem__('assertion_results',[r for r in x['assertion_results'] if r['cell_id']!='windows-x64-ntfs'])),
  ('wrong_target',lambda x:x['assertion_results'][0]['environment'].__setitem__('target','aarch64-apple-darwin')),
  ('missing_environment',lambda x:x['assertion_results'][0].__setitem__('environment',{})),
  ('wrong_subject',lambda x:x['assertion_results'][0].__setitem__('subject_digest','c'*64)),
  ('wrong_lock',lambda x:x['assertion_results'][0].__setitem__('qualification_lock_digest','d'*64)),
  ('assertion_omitted_suite_no_substitute',lambda x:x['assertion_results'].pop(0)),
 ]:
  x=copy.deepcopy(l);change(x);r=v.aggregate(q,x);ck(name,not r['structurally_complete'])
 x=copy.deepcopy(l);bad=copy.deepcopy(x['assertion_results'][0]);bad['status']='failed';x['assertion_results'].append(bad);r1=v.aggregate(q,x);x['assertion_results'].reverse();r2=v.aggregate(q,x);ck('conflicting_duplicate_order_independent',r1==r2 and any(e['code']=='CONFLICTING_DUPLICATE' for e in r1['errors']))
 x=copy.deepcopy(l);x['assertion_results'].append(copy.deepcopy(x['assertion_results'][0]));ck('identical_retransmission_no_false_conflict',v.aggregate(q,x)['structurally_complete'])
 # REAL local subprocess and detached signed evidence for a bounded fixture.
 with tempfile.TemporaryDirectory(prefix='hugit-abc-demo-') as td:
  root=Path(td);(root/'subject').mkdir();(root/'evidence').mkdir();code=b'import json\nprint(json.dumps({"proof_value": 42}, sort_keys=True))\n';(root/'subject/program.py').write_bytes(code)
  env=ref.digest(ref.canonical({'python':sys.version,'platform':platform.platform()}))
  subject={'schema':'A-demo/1','purpose':'demonstration_not_product','artifacts':[{'path':'subject/program.py','sha256':ref.digest(code)}],'environment_digest':env,'required_output':{'proof_value':42}}
  A=ref.digest(ref.canonical(subject));run=subprocess.run([sys.executable,str(root/'subject/program.py')],capture_output=True,timeout=10);(root/'evidence/stdout.json').write_bytes(run.stdout);(root/'evidence/stderr.txt').write_bytes(run.stderr)
  payload={'schema':'B-demo/1','subject_digest':A,'result':{'assertion_id':'A-demo-output42','exit':run.returncode,'environment_digest':env,'stdout_path':'evidence/stdout.json','stdout_sha256':ref.digest(run.stdout)}}
  executor=Ed25519PrivateKey.generate();reviewer=Ed25519PrivateKey.generate();pub=lambda key:key.public_key().public_bytes(serialization.Encoding.Raw,serialization.PublicFormat.Raw)
  bundle={'payload':payload,'executor_signature':base64.b64encode(executor.sign(ref.payload_bytes(payload))).decode(),'reviewer_signature':base64.b64encode(reviewer.sign(ref.payload_bytes(payload))).decode()}
  B=ref.verify_fixture(subject,root,bundle,pub(executor),pub(reviewer));ck('actual_subprocess_and_detached_signatures_qualify_fixture',B['fixture_qualified'] and run.returncode==0)
  publish=root/'published';(publish/'subject').mkdir(parents=True);shutil.copy2(root/'subject/program.py',publish/'subject/program.py')
  C={'schema':'C-demo/1','subject_digest':A,'qualification_digest':B['qualification_digest'],'destination':'local-fixture','note':'no actual product release'}
  closed=ref.verify_publication_fixture(subject,B,publish,C);ck('finite_A_B_C_closes_only_demo_after_readback',closed['state']=='closed_demo_fixture' and not closed['product_release_authorized'])
  C['note']='new annotation outside subject';ck('annotations_do_not_change_A',ref.verify_publication_fixture(subject,B,publish,C)['subject_digest']==A)
  def rejected(name,fn):
   try:fn()
   except Exception as e:ck(name,True,{'exception':type(e).__name__});return
   ck(name,False)
  tamper=copy.deepcopy(bundle);tamper['payload']['result']['exit']=9
  rejected('tampered_B_signature_rejected',lambda:ref.verify_fixture(subject,root,tamper,pub(executor),pub(reviewer)))
  wrong=Ed25519PrivateKey.generate();rejected('untrusted_reviewer_rejected',lambda:ref.verify_fixture(subject,root,bundle,pub(executor),pub(wrong)))
  (root/'subject/program.py').write_bytes(code+b'#changed\n');rejected('A_changed_B_unchanged_rejected',lambda:ref.verify_fixture(subject,root,bundle,pub(executor),pub(reviewer)))
  (publish/'subject/program.py').write_bytes(b'wrong asset');rejected('postpublication_failure_does_not_close',lambda:ref.verify_publication_fixture(subject,B,publish,C))
  demo={'subject_digest_A':A,'qualification_digest_B':B['qualification_digest'],'observed_stdout':run.stdout.decode(),'exit':run.returncode,'signatures':'two ephemeral DEMO keys, not independent product approval','private_keys_retained':False,'product_release_authorized':False}
 report={'scope':'matrix_algorithm_and_cryptographic_fixture_not_Hugit_qualification','cases':cases,'passed':sum(c['passed'] for c in cases),'total':len(cases),'all_passed':all(c['passed'] for c in cases),'actual_execution_demo':demo,'matrix_inputs':'synthetic records; no native Windows execution claimed'}
 (a.root/'validation/qualification-reference-tests.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['scope','passed','total','all_passed']},indent=2))
if __name__=='__main__':main()
