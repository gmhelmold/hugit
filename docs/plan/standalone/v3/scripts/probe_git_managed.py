#!/usr/bin/env python3
"""Real Git probe in private temporary repos. No Hugit binary, no network.
Creates a concurrent human worktree and an immutable managed ref. Simulates a
PROCESS interruption using os._exit after a completed native step, not power loss.
"""
from __future__ import annotations
import argparse,hashlib,json,os,sqlite3,subprocess,sys,tempfile
from pathlib import Path

def h(x):return hashlib.sha256(x).hexdigest()
def env(root):
 e=os.environ.copy();e.update(HOME=str(root/'home'),GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=os.devnull,GIT_AUTHOR_NAME='hugit-fixture',GIT_AUTHOR_EMAIL='fixture@example.invalid',GIT_COMMITTER_NAME='hugit-fixture',GIT_COMMITTER_EMAIL='fixture@example.invalid');return e

def git(root,*args,input=None):
 r=subprocess.run(['git','-c','core.hooksPath='+os.devnull,*args],cwd=root,env=env(root),input=input,capture_output=True,timeout=20)
 if r.returncode:raise RuntimeError('git '+repr(args)+': '+r.stderr.decode(errors='replace'))
 return r.stdout.decode().strip()

def worker(repo,stop):
 info=json.loads((repo/'fixture.json').read_text());db=sqlite3.connect(repo/'journal.sqlite');db.execute('create table if not exists ops(id text primary key,status text,manifest text)');db.execute('insert or ignore into ops values(?,?,?)',('op-001','VALIDATED',info['manifest']));db.commit()
 if stop=='journal':os._exit(86)
 assert git(repo,'cat-file','-t',info['manifest'])=='commit'
 if stop=='objects':os._exit(86)
 git(repo,'update-ref','--no-deref','refs/hugit/operations/op-001',info['manifest'],'0'*len(info['manifest']))
 if stop=='ref':os._exit(86)
 assert git(repo,'rev-parse','refs/hugit/operations/op-001')==info['manifest']
 if stop=='readback':os._exit(86)
 db.execute('update ops set status=? where id=?',('MANAGED_PUBLISHED','op-001'));db.commit()
 if stop=='sql':os._exit(86)
 db.close()

def state(repo,wt):
 # status refresh before digest on both sides; no recorder write to the human worktree.
 status=git(wt,'status','--porcelain');index=Path(git(wt,'rev-parse','--path-format=absolute','--git-path','index'))
 return {'head':git(wt,'rev-parse','HEAD'),'file':(wt/'state.txt').read_text(),'index_sha256':h(index.read_bytes()),'status':status}

def probe(stop):
 with tempfile.TemporaryDirectory(prefix='hugit-managed-ref-') as td:
  root=Path(td);repo=root/'repo';repo.mkdir();(repo/'home').mkdir();git(repo,'init','-q','-b','main');(repo/'state.txt').write_text('old\n');git(repo,'add','state.txt');git(repo,'commit','-qm','base');base=git(repo,'rev-parse','HEAD')
  git(repo,'branch','integration',base);before_check=git(repo,'worktree','list','--porcelain')
  # Compute candidate without touching the human checkout.
  blob=git(repo,'hash-object','-w','--stdin',input=b'new\n');tree=git(repo,'mktree',input=f'100644 blob {blob}\tstate.txt\n'.encode());candidate=git(repo,'commit-tree',tree,'-p',base,input=b'candidate\n')
  metadata={'operation_id':'op-001','candidate':candidate,'tree':tree,'base':base,'validation':'fixture-only-not-Hugit'}
  mb=git(repo,'hash-object','-w','--stdin',input=json.dumps(metadata,sort_keys=True).encode());mt=git(repo,'mktree',input=f'100644 blob {mb}\toperation.json\n'.encode());manifest=git(repo,'commit-tree',mt,'-p',candidate,input=b'operation\n');(repo/'fixture.json').write_text(json.dumps({'manifest':manifest}))
  wt=root/'human-worktree';git(repo,'worktree','add','-q',str(wt),'integration');before=state(repo,wt)
  r=subprocess.run([sys.executable,__file__,'--worker',str(repo),'--stop',stop],capture_output=True,timeout=25)
  assert r.returncode==(0 if stop=='none' else 86),(r.returncode,r.stderr)
  after=state(repo,wt);assert before==after,(stop,before,after)
  rr=subprocess.run(['git','show-ref','--verify','--hash','refs/hugit/operations/op-001'],cwd=repo,env=env(repo),capture_output=True)
  present=rr.returncode==0
  if present:
   assert rr.stdout.decode().strip()==manifest;assert json.loads(git(repo,'show',manifest+':operation.json'))==metadata
   recovered='MANAGED_PUBLISHED'
  else:recovered='PREPARED_RETRYABLE'
  db=sqlite3.connect(repo/'journal.sqlite');prior=db.execute('select status from ops').fetchone()[0]
  if present:db.execute('update ops set status=?',('MANAGED_PUBLISHED',));db.commit()
  db.close()
  # Explicit user handoff exercised only in successful control, outside automation.
  handoff=None
  if stop=='none':
   git(wt,'merge','--ff-only',candidate);handoff={'head':git(wt,'rev-parse','HEAD'),'file':(wt/'state.txt').read_text()};assert handoff['head']==candidate and handoff['file']=='new\n'
  return {'stop_after':stop,'worker_exit':r.returncode,'managed_ref_present':present,'sql_before_recovery':prior,'recovery':recovered,'human_checkout_unchanged':True,'before':before,'after':after,'explicit_user_git_handoff':handoff,'precheck_had_human_branch_checked_out':'branch refs/heads/integration' in before_check}

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);ap.add_argument('--worker',type=Path);ap.add_argument('--stop',default='none');a=ap.parse_args()
 if a.worker:worker(a.worker,a.stop);return
 version=subprocess.check_output(['git','--version']).decode().strip();cases=[probe(s) for s in ['journal','objects','ref','readback','sql','none']]
 report={'scope':'native_git_and_sqlite_reference_not_Hugit_v3','git_version':version,'platform':sys.platform,'cases':cases,'all_passed':True,'limits':['process exit after completed steps, not injected inside rename','not power loss or native Windows/macOS','private temp repo; no user repository changes']}
 (a.root/'validation/native-managed-ref-probe.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(json.dumps({'scope':report['scope'],'git_version':version,'cases':len(cases),'all_passed':True},indent=2))
if __name__=='__main__':main()
