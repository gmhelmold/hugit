#!/usr/bin/env python3
"""Update structural digests after AUTHORIZED canonical edit. Not an approval."""
import argparse,json,hashlib
from pathlib import Path
def h(x):return hashlib.sha256(json.dumps(x,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()).hexdigest()
def main():
 a=argparse.ArgumentParser();a.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);a.add_argument('--ack-semantic-review-required',action='store_true');n=a.parse_args()
 if not n.ack_semantic_review_required:a.error('explicit --ack-semantic-review-required required; this does not approve the changes')
 f=n.root/'backlog.json';p=json.loads(f.read_text());pr={r['id']:r for r in p['protocol_registry']}
 for r in pr.values():r['digest']=h({k:v for k,v in r.items() if k!='digest'})
 p['protocol_set_digest']=h({k:r['digest'] for k,r in pr.items()})
 for t in p['tasks']:t['protocol_bindings']=[{'id':k,'version':pr[k]['version'],'digest':pr[k]['digest']} for k in t['protocols']]
 cs={c['id']:c for t in p['tasks'] for v in t['axioms'].values() for c in v}
 for a in p['assertions']:
  if len(a['criterion_ids'])==1:
   c=cs[a['criterion_ids'][0]];a['criterion_digest']=h({'id':c['id'],'statement':c['statement']})
  a['oracle_digest']=h({k:v for k,v in a.items() if k not in ['oracle_digest','status']})
 f.write_text(json.dumps(p,ensure_ascii=False,indent=2)+'\n');print('Digests updated. Review semantics, regenerate views and run tests. No approval granted.')
if __name__=='__main__':main()
