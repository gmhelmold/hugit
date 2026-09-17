"""Lossless reconstruction of approved bytes from a verified partial transport."""
import copy,hashlib,json

def h(x):
 return hashlib.sha256(json.dumps(x,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()).hexdigest()

def restore(seed,orders):
 p=copy.deepcopy(seed);template=p.pop('_assertion_template')
 pr={r['id']:r for r in p['protocol_registry']}
 for r in pr.values():r['digest']=h({k:v for k,v in r.items() if k!='digest'})
 p['protocol_set_digest']=h({k:r['digest'] for k,r in pr.items()})
 ts={t['id']:t for t in p['tasks']}
 for t in ts.values():t['protocol_bindings']=[{'id':k,'version':pr[k]['version'],'digest':pr[k]['digest']} for k in t['protocols']]
 cs={c['id']:(t,c) for t in ts.values() for v in t['axioms'].values() for c in v}
 tests={x['id']:x for x in p['tests']};out=[]
 for s,order in zip(p['assertions'],orders):
  if 'raw' in s:a=copy.deepcopy(s['raw'])
  else:
   t,c=cs[s['criterion']];test=tests[s['test']];a=copy.deepcopy(template)
   a.update(id=s['id'],criterion_ids=[s['criterion']],owner_package=t['id'],test_id=s['test'],stage=s['stage'])
   a['given'].update(protocols=[k+'@'+pr[k]['version'] for k in t['protocols']],fixture_inputs=test['preconditions'],fixture_scope=t['write_set'])
   a['when']=test['procedure']
   a['then'].update(predicate=c['statement'],observable_paths=t['write_set'])
   a.update(copy.deepcopy(s['override']))
   for k in s['delete']:a.pop(k,None)
  if len(a['criterion_ids'])==1:
   c=cs[a['criterion_ids'][0]][1];a['criterion_digest']=h({'id':c['id'],'statement':c['statement']})
  a['oracle_digest']=h({k:v for k,v in a.items() if k not in ['oracle_digest','status']})
  out.append({k:a[k] for k in order})
 p['assertions']=out;return p
