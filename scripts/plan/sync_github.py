#!/usr/bin/env python3
"""One-way, explicit import of the approved Hugit plan. Dry-run by default.
Never closes/reopens issues, merges a PR, or changes product state.
Authentication comes only from GH_TOKEN/GITHUB_TOKEN, never from plan data.
"""
from __future__ import annotations
import argparse, hashlib, json, os, re, sys, time
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError

REPO = 'gmhelmold/hugit'
ROOT = Path(__file__).resolve().parents[2]
PLAN_ROOT = ROOT / 'docs/plan/standalone/v3'
AXES = [('completeness_criteria','Completeness criteria'), ('success_criteria','Success criteria'), ('quality_standards','Quality standards'), ('dod','Definition of Done — DoD'), ('invariants','Invariants')]
START='<!-- hugit-generated:start -->';END='<!-- hugit-generated:end -->'

def canonical(x): return json.dumps(x,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()
def digest(x): return hashlib.sha256(canonical(x)).hexdigest()
def marker(wp): return '<!-- hugit-wp:'+wp+' -->'
def write_json(path, x):
    path.parent.mkdir(parents=True,exist_ok=True)
    tmp=path.with_name('.'+path.name+'.tmp');tmp.write_text(json.dumps(x,ensure_ascii=False,indent=2)+'\n');tmp.replace(path)

def graph(plan):
    tasks={t['id']:t for t in plan['tasks']};anc={}
    for k in plan['topological_order']:
        deps=tasks[k]['depends_on'];anc[k]=set(deps)|set().union(*(anc[d] for d in deps))
    reduced={k:[d for d in t['depends_on'] if not any(d in anc[x] for x in t['depends_on'] if x!=d)] for k,t in tasks.items()}
    # Only redundant native edges are removed; the full DAG stays in backlog.json.
    racc={}
    for k in plan['topological_order']:racc[k]=set(reduced[k])|set().union(*(racc[d] for d in reduced[k]))
    if racc!=anc:raise ValueError('transitive reduction changed the dependency relation')
    return reduced

def document_url(commit,path):return f'https://github.com/{REPO}/blob/{commit}/docs/plan/standalone/v3/{path}'
def body_for(t,plan,commit,parent=None,issue_map=None):
    m=issue_map or {};wid=t['id'];spec=document_url(commit,f'work-packages/{wid}.md')
    lines=[marker(wid),START,f'## {wid} — contrato de execução',f'**Plano:** v{plan["plan_version"]} · **fase:** {t["phase"]} · **lane:** {t["owner_lane"]} · **prioridade:** {t["priority_class"]}.',
        '**Estado inicial: backlog; nenhuma implementação ou qualificação declarada.** A aprovação/merge do PR de planejamento e as pré-condições do WP precedem execução. Fechar por evidência, não por merge automático.',
        f'**Contrato imutável:** [work package completo]({spec}). [Fonte normativa]({document_url(commit,"backlog.json")}).',
        f'**Identidade do plano:** `{digest(plan)}`. **Commit da especificação:** `{commit}`.',
        'Hugit é 100% standalone. Intent = goal capturado por hook; nenhuma dependência de CoreLink, conta, runner remoto ou registro voluntário pelo LLM.']
    if parent:lines.append(f'**Acompanhamento:** #{parent}.')
    deps=[f'#{m[d]["number"]} (`{d}`)' if d in m else '`'+d+'`' for d in t['depends_on']]
    lines+=['## Dependências e admissão',', '.join(deps) if deps else 'Sem predecessor técnico. A admissão de execução continua sujeita ao contrato.',
        '\n'.join('- '+x for x in t['preconditions']),
        '**Protocolos:** '+', '.join(f'[{b["id"]}@{b["version"]}]({document_url(commit,"contratos/"+b["id"]+".md")}) · `{b["digest"]}`' for b in t['protocol_bindings']),
        '## Outputs e coordenação','\n'.join('- `'+x+'`' for x in t['write_set']),
        'Caminhos propostos: HUG-001 resolve existentes/novos contra o checkout. '+t['shared_edits'],
        '## Unidades verificáveis','\n'.join('- **'+s['id']+'** — '+s['change'] for s in t['implementation_slices'])]
    for key,label in AXES:
        lines+=['## '+label]
        for c in t['axioms'][key]:
            lines += ['- **'+c['id']+' — obrigatório:** '+c['statement']+'\n  Assertions: '+', '.join('`'+a+'`' for a in c['assertion_refs'])+'. Famílias: '+', '.join('`'+x+'`' for x in c['test_refs'])+'.']
    tests={x['id']:x for x in plan['tests']}
    lines+=['## Verificação e evidências','\n'.join('- `'+i+'`: '+', '.join(tests[i]['required_cells'])+'; etapa '+tests[i]['stage']+'.' for i in t['acceptance_tests']),
        'Catálogo detalhado, preparo, predicados e artifacts: [contrato completo]('+spec+'). Evidência por sujeito A, assertion, célula e tentativa; pacote B destacado, promoção C separada. Não usar `Closes` em PR intermediário.',
        '## Budget e recovery',t['budget_contract'],'**Rollback/recovery específico:** '+t['rollback']['procedure'],
        '## Política de fechamento','Todos os cinco axiomas e suas assertions devem estar demonstrados no candidato integrado, com recovery ensaiado e revisão distinta da autoria. Suite verde ou issue fechada não substitui a qualificação. HUG-056 só fecha após publicação e pós-verificação.',END]
    result='\n\n'.join(lines)+'\n'
    if len(result)>64000:raise ValueError(wid+' exceeds safe issue body limit')
    return result

def replace_generated(old,new):
    if old.count(START)!=1 or old.count(END)!=1:raise ValueError('generated block missing or ambiguous')
    before,rest=old.split(START,1);_,after=rest.split(END,1)
    block=new.split(START,1)[1].split(END,1)[0]
    return before+START+block+END+after

class Api:
    def __init__(self):
        token=os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN')
        if not token:raise RuntimeError('GH_TOKEN or GITHUB_TOKEN is required for --apply')
        self.token=token;self.last_write=0.0
    def request(self,method,path,payload=None):
        if not path.startswith('/repos/'+REPO+'/'):raise ValueError('API outside approved repository')
        if method!='GET':time.sleep(max(0,1.15-(time.monotonic()-self.last_write)))
        req=Request('https://api.github.com'+path,method=method,data=None if payload is None else json.dumps(payload).encode(),headers={'Authorization':'Bearer '+self.token,'Accept':'application/vnd.github+json','X-GitHub-Api-Version':'2026-03-10','Content-Type':'application/json','User-Agent':'hugit-approved-plan-import'})
        try:
            with urlopen(req,timeout=60) as r:return json.load(r) if r.status!=204 else None
        except HTTPError as e:
            # Never blindly retry a POST with ambiguous outcome. Next invocation
            # reconciles markers and native relations before any new mutation.
            raw=e.read(4096).decode(errors='replace')
            raise RuntimeError(f'{method} {path}: HTTP {e.code}: {raw}') from None
        finally:
            if method!='GET':self.last_write=time.monotonic()
    def pages(self,path):
        result=[];sep='&' if '?' in path else '?'
        for page in range(1,1001):
            rows=self.request('GET',path+sep+f'per_page=100&page={page}')
            if not isinstance(rows,list):raise ValueError('expected paginated list')
            result.extend(rows)
            if len(rows)<100:return result
        raise RuntimeError('pagination ceiling; refusing partial inventory')

def scan(issues):
    found={};parent=None
    for i in issues:
        if 'pull_request' in i:continue
        body=i.get('body') or ''
        if '<!-- hugit-tracking:standalone-v3 -->' in body:
            if parent:raise ValueError('duplicate parent marker')
            parent=i
        for wid in re.findall(r'<!-- hugit-wp:(HUG-\d{3}) -->',body):
            if wid in found:raise ValueError('duplicate WP marker '+wid)
            found[wid]=i
    return found,parent

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--apply',action='store_true');ap.add_argument('--spec-commit',required=True);ap.add_argument('--output',type=Path,default=ROOT/'docs/plan/standalone/github/issue-map.json');ap.add_argument('--dry-run-dir',type=Path,default=ROOT/'docs/plan/standalone/github/preview');args=ap.parse_args()
    if not re.fullmatch('[a-f0-9]{40}',args.spec_commit):ap.error('spec-commit must be immutable SHA')
    p=json.loads((PLAN_ROOT/'backlog.json').read_text());reduced=graph(p)
    if len(p['tasks'])!=60 or any(set(t['axioms'])!={k for k,_ in AXES} for t in p['tasks']):raise ValueError('incomplete plan')
    for t in p['tasks']:
        path=args.dry_run_dir/(t['id']+'.md');path.parent.mkdir(parents=True,exist_ok=True);path.write_text(body_for(t,p,args.spec_commit))
    summary={'repository':REPO,'plan_digest':digest(p),'spec_commit':args.spec_commit,'work_packages':60,'criterion_count':sum(len(v) for t in p['tasks'] for v in t['axioms'].values()),'canonical_edges':sum(len(t['depends_on']) for t in p['tasks']),'native_edges':sum(map(len,reduced.values())),'native_edges_are_transitive_reduction':True,'project':{'status':'not_created','reason':'Current repository-scoped connection does not expose Projects; no account token requested or inferred.'},'dry_run':not args.apply}
    if not args.apply:print(json.dumps(summary,indent=2));return 0
    api=Api();prefix='/repos/'+REPO;all_issues=api.pages(prefix+'/issues?state=all');found,parent=scan(all_issues)
    # Refuse ambiguous title-only predecessors instead of creating duplicates.
    for t in p['tasks']:
        suspects=[i for i in all_issues if 'pull_request' not in i and '['+t['id']+']' in i['title'] and marker(t['id']) not in (i.get('body') or '')]
        if suspects:raise ValueError('unmanaged issue already names '+t['id'])
    labels={x['name'] for x in api.pages(prefix+'/labels')}
    wanted={'plan:standalone-v3':('1D76DB','Approved standalone plan v3 execution tracking'),'type:work-package':('5319E7','One issue per HUG work package'),'status:backlog':('EDEDED','Not admitted to execution; evidence required'),'type:tracking':('0052CC','Parent tracking issue, not product completion')}
    for k,(color,desc) in wanted.items():
        if k not in labels:api.request('POST',prefix+'/labels',{'name':k,'color':color,'description':desc})
    if not parent:
        parent=api.request('POST',prefix+'/issues',{'title':'[TRACKING] Hugit standalone — execução do planejamento v3','body':'<!-- hugit-tracking:standalone-v3 -->\n\n'+START+'\nImportação em andamento. WPs permanecem backlog; nenhum merge ou conclusão automática.\n'+END,'labels':['plan:standalone-v3','type:tracking','status:backlog']})
    journal={**summary,'parent':{'id':parent['id'],'number':parent['number'],'url':parent['html_url']},'issues':{},'relations_verified':False,'import_complete':False}
    for t in p['tasks']:
        wid=t['id']
        if wid not in found:
            found[wid]=api.request('POST',prefix+'/issues',{'title':f'[{wid}] {t["title"]}','body':body_for(t,p,args.spec_commit,parent['number']),'labels':['plan:standalone-v3','type:work-package','status:backlog']})
        i=found[wid];journal['issues'][wid]={'id':i['id'],'node_id':i.get('node_id'),'number':i['number'],'url':i['html_url'],'phase':t['phase'],'lane':t['owner_lane'],'status':i['state'],'depends_on':t['depends_on'],'native_blocked_by':reduced[wid]}
        write_json(args.output,journal)
    children={i['id'] for i in api.pages(prefix+f'/issues/{parent["number"]}/sub_issues')}
    for t in p['tasks']:
        i=found[t['id']]
        if i['id'] not in children:api.request('POST',prefix+f'/issues/{parent["number"]}/sub_issues',{'sub_issue_id':i['id'],'replace_parent':False})
        existing={d['id'] for d in api.pages(prefix+f'/issues/{i["number"]}/dependencies/blocked_by')}
        for dep in reduced[t['id']]:
            if found[dep]['id'] not in existing:api.request('POST',prefix+f'/issues/{i["number"]}/dependencies/blocked_by',{'issue_id':found[dep]['id']})
        # Re-read owned block. Human edits inside it are reported, not clobbered.
        current=api.request('GET',prefix+f'/issues/{i["number"]}');new=body_for(t,p,args.spec_commit,parent['number'],journal['issues'])
        expected_initial=body_for(t,p,args.spec_commit,parent['number'])
        oldblock=(current.get('body') or '').split(START)[-1].split(END)[0]
        if oldblock not in [expected_initial.split(START)[1].split(END)[0],new.split(START)[1].split(END)[0]]:
            raise ValueError('owned block changed; reconcile before update: '+t['id'])
        replaced=replace_generated(current['body'],new)
        if replaced!=current['body']:api.request('PATCH',prefix+f'/issues/{i["number"]}',{'body':replaced})
    current_children={i['id'] for i in api.pages(prefix+f'/issues/{parent["number"]}/sub_issues')}
    if not {i['id'] for i in found.values()}<=current_children:raise ValueError('sub-issue readback incomplete')
    for t in p['tasks']:
        actual={x['id'] for x in api.pages(prefix+f'/issues/{found[t["id"]]["number"]}/dependencies/blocked_by')}
        if not {found[d]['id'] for d in reduced[t['id']]}<=actual:raise ValueError('dependency readback incomplete '+t['id'])
    rows=['<!-- hugit-tracking:standalone-v3 -->',START,'# Hugit — standalone v3',f'[Planejamento versionado]({document_url(args.spec_commit,"PLANO.md")}) · [Contratos de execução]({document_url(args.spec_commit,"EXECUCAO.md")})',
        '60 WPs, 900 critérios, cinco axiomas obrigatórios em cada pacote. Nenhum WP foi executado por esta importação. Aguardar revisão/merge do PR de planejamento antes da admissão.',
        '**Autoridade:** backlog.json. Issues são projeções de execução. Mudanças no contrato entram por PR. PR intermediário usa Refs, não Closes; fechamento exige assertions e qualificação.',
        '**Dependências nativas:** redução transitiva verificada do DAG; o conjunto completo permanece na especificação e no corpo de cada issue. Nenhum bloqueio lógico foi removido.',
        '**GitHub Project:** pendente; o conector atual não expõe a operação e GITHUB_TOKEN não concede acesso a Projects.',
        '| WP | Fase | Lane | Issue |','|---|---|---|---|']
    for t in p['tasks']:rows.append(f'| {t["id"]} | {t["phase"]} | {t["owner_lane"]} | #{found[t["id"]]["number"]} |')
    rows += ['## Conclusão','Esta issue acompanha implementação e qualificação. Importação completa não significa produto pronto. Full termina somente com 60/60 contratos fechados e pós-publicação verificada.',END]
    cur=api.request('GET',prefix+f'/issues/{parent["number"]}')
    api.request('PATCH',prefix+f'/issues/{parent["number"]}',{'body':replace_generated(cur['body'],'\n\n'.join(rows)+'\n')})
    journal.update(relations_verified=True,import_complete=True,dry_run=False);write_json(args.output,journal)
    print(json.dumps({'parent':journal['parent'],'issues':len(journal['issues']),'native_edges':summary['native_edges'],'relations_verified':True,'project':journal['project']},ensure_ascii=False,indent=2));return 0
if __name__=='__main__':
    try:raise SystemExit(main())
    except (ValueError,RuntimeError,OSError) as e:print('IMPORT_STOPPED: '+str(e),file=sys.stderr);raise SystemExit(1)
