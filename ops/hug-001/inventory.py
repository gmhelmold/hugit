#!/usr/bin/env python3
"""HUG-001 mechanical inventory, NOT semantic/runtime qualification.
Run against a clean pinned checkout. Writes only output/evidence folders.
Cargo metadata is supplied separately with its original logs retained.
"""
import argparse, base64, collections, copy, hashlib, json, os, re
import subprocess, sys, tomllib
from pathlib import Path, PurePosixPath
OUTPUTS = ['source-inventory.json', 'reachability.json', 'path-bindings.json']
def encoded(v): return (json.dumps(v,ensure_ascii=True,sort_keys=True,indent=2)+'\n').encode()
def sha(b): return hashlib.sha256(b).hexdigest()
def write(p,v):
    p.parent.mkdir(parents=True,exist_ok=True); p.write_bytes(encoded(v))
def git(root,*args):
    r=subprocess.run(['git','-c','core.hooksPath=/dev/null',*args],cwd=root,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=90)
    if r.returncode: raise RuntimeError('git setup_error: '+r.stderr[:2000].decode(errors='replace'))
    return r.stdout
def classify(p,mode,data):
    if mode=='160000': return 'gitlink_not_expanded'
    if mode=='120000': return 'symlink_not_followed'
    if p.startswith('docs/plan/standalone/v3/'): return 'plan_tooling' if '/scripts/' in p else 'plan_normative_or_generated'
    if p.startswith('engine-snapshots/'): return 'historical_engine_snapshot'
    if p.startswith('.github/workflows/'): return 'ci_workflow'
    if p.endswith('Cargo.toml') or p=='Cargo.lock': return 'cargo_manifest_or_lock'
    if p.startswith('conformance/') or '/fixtures/' in p: return 'fixture_or_conformance'
    if '/tests/' in p or p.startswith('tests/'): return 'test_source_or_data'
    if p.endswith('.rs'): return 'rust_source'
    if p.endswith(('.py','.sh','.js','.ts')) or data.startswith(b'#!'): return 'script_or_adapter'
    if p.startswith(('.claude/','skills/')): return 'agent_instruction_or_hook'
    if p.endswith(('.md','.html')): return 'documentation_or_presentation'
    if b'\0' in data: return 'binary_data'
    return 'configuration_or_other_text'
def lane(p):
    for prefix,owner in [('crates/hugit-refstore/','B'),('crates/hugit-checks/','C'),('crates/hugit-queue/','C'),('crates/hugit-policy/','D'),('crates/hugit-mcp/','G'),('crates/hugit-ledger/','G'),('.github/','H'),('tests/','I')]:
        if p.startswith(prefix): return owner
    return 'A'
def rows_from_git(root,ref):
    tree_raw=git(root,'ls-tree','-r','-z','--full-tree',ref); index_raw=git(root,'ls-files','--stage','-z'); tree={};index={}
    for entry in tree_raw.split(b'\0'):
        if not entry: continue
        attrs,path=entry.split(b'\t',1);mode,kind,oid=attrs.decode().split();tree[path]=(mode,kind,oid)
    for entry in index_raw.split(b'\0'):
        if not entry: continue
        attrs,path=entry.split(b'\t',1);mode,oid,stage=attrs.decode().split()
        if stage!='0': raise ValueError('unmerged index')
        index[path]=(mode,oid)
    if {p:(m,o) for p,(m,k,o) in tree.items()}!=index: raise ValueError('TREE_INDEX_MISMATCH')
    rows=[];contents={}
    for rawpath,(mode,kind,oid) in sorted(tree.items()):
        p=os.fsdecode(rawpath);parts=PurePosixPath(p).parts
        if PurePosixPath(p).is_absolute() or '..' in parts: raise ValueError('unsafe source path')
        actual=root/p
        if mode=='160000': data=b''
        elif mode=='120000': data=os.fsencode(os.readlink(actual))
        else:
            if actual.is_symlink() or not actual.is_file(): raise ValueError('unexpected filesystem kind: '+p)
            data=actual.read_bytes()
        if kind=='blob':
            alg=hashlib.sha1 if len(oid)==40 else hashlib.sha256
            if alg(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()!=oid: raise ValueError('WORKTREE_BLOB_MISMATCH: '+p)
        contents[p]=data
        rows.append({'path':p,'path_bytes_base64':base64.b64encode(rawpath).decode(),'git_mode':mode,'git_kind':kind,'git_oid':oid,'sha256':sha(data) if kind=='blob' else None,'size_bytes':len(data) if kind=='blob' else None,'classification':classify(p,mode,data),'owner_lane':lane(p),'byte_inspection':'verified_git_object' if kind=='blob' else 'gitlink_identity_only','semantic_review':{'status':'not_reviewed','reviewer':None,'ranges':[]}})
    return rows,contents,tree_raw,index_raw
def normalize_metadata(obj,root):
    s=json.dumps(obj,ensure_ascii=True)
    for old,new in [(str(root),'$REPO'),(os.environ.get('CARGO_HOME','/home/runner/.cargo'),'$CARGO_HOME')]: s=s.replace(old,new)
    return json.loads(s)
def validate(inv,reach,bindings,baseline,metas,plan):
    errors=[];actual={x['path']:x for x in inv['files']};expected={x['path']:x for x in baseline}
    if len(actual)!=len(inv['files']) or set(actual)!=set(expected): errors.append('FILE_SET_MISMATCH')
    for path in set(actual)&set(expected):
        for field in ['git_oid','git_mode','sha256','path_bytes_base64','classification']:
            if actual[path].get(field)!=expected[path].get(field): errors.append('FILE_RECORD_MISMATCH');break
        if actual[path]['semantic_review']['status']!='not_reviewed': errors.append('UNSUPPORTED_REVIEW_CLAIM')
    ids={p['id'] for m in metas.values() for p in m['packages']};listed=[p['id'] for p in reach['cargo_packages']]
    if set(listed)!=ids or len(listed)!=len(set(listed)): errors.append('DEPENDENCY_SET_MISMATCH')
    wanted={(t['id'],p) for t in plan['tasks'] for p in t['write_set']};bound=[(b['wp_id'],b['path']) for b in bindings['bindings']]
    if set(bound)!=wanted or len(bound)!=len(wanted): errors.append('BINDING_SET_MISMATCH')
    for b in bindings['bindings']:
        if b['exists_at_subject']!=(b['path'] in expected): errors.append('BINDING_EXISTENCE_MISMATCH')
        if b['exists_at_subject'] and b['path'] in expected and b['existing_git_oid']!=expected[b['path']]['git_oid']: errors.append('BINDING_OID_MISMATCH')
    return sorted(set(errors))
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--root',type=Path,required=True);ap.add_argument('--ref',required=True);ap.add_argument('--evidence',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);args=ap.parse_args()
    root=args.root.resolve();ev=args.evidence.resolve();out=args.out.resolve();ev.mkdir(parents=True,exist_ok=True);out.mkdir(parents=True,exist_ok=True)
    if not re.fullmatch('[0-9a-f]{40}|[0-9a-f]{64}',args.ref): raise ValueError('INVALID_SOURCE_REF')
    if git(root,'rev-parse','HEAD').decode().strip()!=args.ref: raise ValueError('SOURCE_DRIFT')
    before=git(root,'status','--porcelain=v1','--untracked-files=all')
    if before: raise ValueError('DIRTY_SOURCE')
    planpath='docs/plan/standalone/v3/backlog.json';plan=json.loads((root/planpath).read_text());files,texts,tr,idx=rows_from_git(root,args.ref)
    (ev/'git-ls-tree.z').write_bytes(tr);(ev/'git-ls-files-stage.z').write_bytes(idx)
    origin={'source_commit':args.ref,'source_tree':git(root,'rev-parse',args.ref+'^{tree}').decode().strip(),'plan_file_sha256':sha(texts[planpath]),'plan_version':plan['plan_version'],'tool_version':'hug-001-inventory-1','runtime_execution':'not_run','qualification':'partial_mechanical_inventory_not_wp_acceptance'}
    inv={**origin,'files':files,'file_count':len(files),'classes':dict(sorted(collections.Counter(f['classification'] for f in files).items())),'semantic_reviewed_files':0,'scope_exclusions':[],'limits':['All tracked paths remain in the denominator. Byte verification is not semantic review.','Gitlinks, if present, identify external trees; their contents are not silently counted.']}
    metas={k:normalize_metadata(json.loads((ev/('cargo-metadata-'+k+'.json')).read_text()),root) for k in ['default','all-features']}
    packages={p['id']:p for m in metas.values() for p in m['packages']};members=set(metas['default']['workspace_members']);cargo=[]
    for pid,p in sorted(packages.items()):
        cargo.append({'id':pid,'name':p['name'],'version':p['version'],'source':p.get('source'),'role':'workspace_member' if pid in members else 'resolved_dependency','manifest_path':p['manifest_path'],'features':p['features'],'targets':p['targets'],'dependencies':p['dependencies'],'semantic_review':'not_reviewed'})
    manifests=[]
    for path,data in texts.items():
        if path.endswith('Cargo.toml'):
            obj=tomllib.loads(data.decode());manifests.append({'path':path,'package':obj.get('package',{}).get('name'),'in_workspace':any(p['manifest_path']=='$REPO/'+path for p in cargo if p['id'] in members),'features':obj.get('features',{}),'review_scope':'TOML structurally parsed, not proof of runtime reachability'})
    surfaces=[];patterns={'subcommand_enum':r'\b(?:pub\s+)?enum\s+(\w*Command\w*)\b','dispatch_reference':r'\b(?:Command|\w+Command)::([A-Za-z_][A-Za-z_0-9]*)','process_candidate':r'(?:Command::new|\.spawn\(|\.exec\()','network_candidate':r'(?:TcpStream|TcpListener|reqwest|ureq|HttpAcClient|LeaseClient|https?://)','write_candidate':r'(?:fs::write|File::create|OpenOptions|\.append\(|\.rename\(|atomic_write|update-ref)','optional_feature':r'#\[cfg\([^\n]*feature','mcp_tool_name':r'"name"\s*:\s*"([a-z][a-z0-9-]+)"'}
    bypath={f['path']:f for f in files}
    for path,data in texts.items():
        if not path.endswith(('.rs','.py','.sh','.ts','.js','.yml','.yaml')): continue
        kind=bypath[path]['classification'];tests=sorted(p for p in texts if p.startswith('/'.join(path.split('/')[:2])+'/tests/') and p.endswith('.rs')) if path.startswith('crates/') else [];tags=[]
        if kind=='historical_engine_snapshot': tags.append('historical_not_reopened')
        if kind in ['test_source_or_data','fixture_or_conformance']: tags.append('test_or_fixture')
        if '/land/' in path: tags.append('landing_simulation_requires_semantic_review')
        if path.startswith('crates/hugit-mcp/'): tags.append('mcp_optional_binary_not_default_cli')
        if not tags: tags=['source_candidate_not_runtime_proof']
        for no,line in enumerate(data.decode('utf-8',errors='replace').splitlines(),1):
            for category,pat in patterns.items():
                if category=='mcp_tool_name' and not path.startswith('crates/hugit-mcp/'): continue
                matches=list(re.finditer(pat,line))
                if matches: surfaces.append({'path':path,'line':no,'kind':category,'symbols':sorted(set(m.group(1) if m.lastindex else m.group(0) for m in matches)),'owner_lane':lane(path),'classification':tags,'test_candidates':tests,'test_binding':'unverified','semantic_status':'needs_review'})
    workspace_targets=[{'package_id':p['id'],'target':t} for p in cargo if p['id'] in members for t in p['targets']]
    reach={**origin,'workspace_member_count':len(members),'cargo_packages':cargo,'cargo_manifests':manifests,'workspace_targets':workspace_targets,'resolved_graphs':{k:m['resolve'] for k,m in metas.items()},'locked_packages':tomllib.loads(texts['Cargo.lock'].decode())['package'],'surface_candidates':surfaces,'semantic_reachability':'not_qualified','unresolved':['Lexical matches include comments, tests and dead code; no claim of actual calls or exhaustive semantic reachability.','Tests listed by crate are candidates, not independently verified coverage.','Goal hook location/contract belongs to HUG-057; no branch-name inference.','Default/all-features metadata is package-level resolution, not a native platform qualification.'],'whole_wp_ready':False}
    bind=[]
    for t in plan['tasks']:
        for path in t['write_set']:
            if '..' in PurePosixPath(path).parts or PurePosixPath(path).is_absolute() or any(x in path for x in '*?['): raise ValueError('NON_EXACT_PLAN_PATH: '+path)
            f=bypath.get(path);ancestors=[str(a) for a in PurePosixPath(path).parents if str(a)!='.'];conflicts=[p for p in ancestors if p in bypath];others=[x['id'] for x in plan['tasks'] if x['id']!=t['id'] and path in x['write_set']]
            bind.append({'wp_id':t['id'],'path':path,'owner_lane':t['owner_lane'],'exists_at_subject':f is not None,'existing_git_oid':f['git_oid'] if f else None,'operation':'edit_existing' if f else 'create_new','ancestor_file_conflicts':conflicts,'shared_with':others,'integrator_coordination_required':bool(others) or path in ['Cargo.toml','Cargo.lock'],'admission':'blocked_by_path_conflict' if conflicts else 'path_resolved_not_execution_authorized'})
    bindings={**origin,'bindings':bind,'binding_count':len(bind),'unique_paths':len({b['path'] for b in bind}),'unknown_existing_paths':0,'rule':'Bindings pin existence only. Ownership of shared registries and semantic destination must be reviewed before dispatch.'}
    docs=[inv,reach,bindings];assert not validate(*docs,files,metas,plan);checks=[]
    def attack(name,mutate):
        a,b,c=copy.deepcopy(docs);mutate(a,b,c);errors=validate(a,b,c,files,metas,plan);checks.append({'name':name,'expected':'rejected','observed_errors':errors,'passed':bool(errors)})
    attack('remove_tracked_file',lambda a,b,c:a['files'].pop());attack('add_unclassified_executable',lambda a,b,c:a['files'].append({**a['files'][0],'path':'synthetic-new-executable.sh'}));attack('corrupt_content_digest',lambda a,b,c:a['files'][0].update(sha256='0'*64));attack('false_semantic_review',lambda a,b,c:a['files'][0]['semantic_review'].update(status='fully_reviewed'));attack('omit_dependency',lambda a,b,c:b['cargo_packages'].pop());attack('unclassified_extra_dependency',lambda a,b,c:b['cargo_packages'].append({'id':'synthetic-unclassified-dependency'}));attack('omit_path_binding',lambda a,b,c:c['bindings'].pop());attack('false_existing_path',lambda a,b,c:c['bindings'][0].update(exists_at_subject=not c['bindings'][0]['exists_at_subject']))
    if not all(x['passed'] for x in checks): raise AssertionError('negative control survived')
    for name,obj in zip(OUTPUTS,docs): write(out/name,obj)
    first={n:sha((out/n).read_bytes()) for n in OUTPUTS}
    for name,obj in zip(OUTPUTS,docs): (out/name).unlink();write(out/name,obj)
    assert first=={n:sha((out/n).read_bytes()) for n in OUTPUTS};assert git(root,'status','--porcelain=v1','--untracked-files=all')==before
    report={'scope':'HUG-001 mechanical discovery, not acceptance of all axioms','source_commit':args.ref,'positive':'passed','negative_controls':checks,'recovery_regeneration_byte_identical':True,'source_tracked_and_untracked_unchanged':True,'file_count':len(files),'workspace_members':len(members),'resolved_packages':len(cargo),'bindings':len(bind),'outputs_sha256':first,'semantic_review':'not_done','independent_approval':'not_done','work_package_closed':False};write(ev/'verification.json',report);print(json.dumps(report,ensure_ascii=False))
if __name__=='__main__':
    try: main()
    except Exception as e:
        print(json.dumps({'status':'setup_or_validation_error','error':str(e)[:4000]}),file=sys.stderr);raise SystemExit(1)
