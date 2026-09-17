#!/usr/bin/env python3
"""Deterministic offline renderer. backlog.json is the only normative source.
No network, no user code execution, no third-party dependencies.
"""
from __future__ import annotations
import argparse,html,json,re
from pathlib import Path
AXES=[('completeness_criteria','Completeness criteria'),('success_criteria','Success criteria'),('quality_standards','Quality standards'),('dod','Definition of Done — DoD'),('invariants','Invariants')]
def js(x):return json.dumps(x,ensure_ascii=False,indent=2)+'\n'
def wp_md(t,p):
 aidx={x['id']:x for x in p['assertions']};tidx={x['id']:x for x in p['tests']}
 out=[f"# {t['id']} — {t['title']}",f"**Estado:** {t['status']}; **owner:** {t['owner_lane']}; **fase:** {t['phase']}; **prioridade:** {t['priority_class']}.",
 '**Dependências:** '+(', '.join(t['depends_on']) or 'nenhuma'),
 '**Congelamento de runtime requerido:** '+str(t['requires_runtime_protocol_freeze']).lower(),
 '**Revisões:** '+', '.join(t['review_refs']+t['review2_refs']),
 '**Contratos vinculados:**\n'+ '\n'.join(f"- {x['id']}@{x['version']} — sha256 `{x['digest']}`" for x in t['protocol_bindings']),
 '## Prontidão',*t['preconditions'],
 '## Outputs e ownership','Caminhos propostos: HUG-001 precisa vincular sua presença/criação ao checkout real antes da implementação. Evidências B/C ficam fora do sujeito A.',
 '\n'.join('- `'+s+'`' for s in t['write_set']),
 '**Leituras:** '+', '.join('`'+s+'`' for s in t.get('source_paths_to_reinspect',t.get('read_set',[]))),
 '**Recursos exclusivos:** '+', '.join(t.get('exclusive_resources',[])),
 '## Interfaces e erros','```json\n'+js({'interfaces':t['interfaces'],'errors':t['error_contract']}).strip()+'\n```',
 '## Unidades verificáveis','\n'.join(f"{i+1}. **{s['id']}** — {s['change']}" for i,s in enumerate(t['implementation_slices']))]
 if t.get('contract_updates'):out+=['## Decisões atualizadas',*t['contract_updates']]
 if t.get('detached_evidence_outputs'):out+=['## Outputs de evidência destacados de A', '\n'.join('- `'+x+'`' for x in t['detached_evidence_outputs'])]
 if t.get('review2_responsibility'):out+=['## Responsabilidade nos achados de integração', '\n'.join('- '+r['finding']+': '+r['local_acceptance'] for r in t['review2_responsibility'])]
 for axis,title in AXES:
  out+=['## '+title]
  for c in t['axioms'][axis]:
   out += [f"**{c['id']} — obrigatório.** {c['statement']}", 'Assertions: '+', '.join('`'+x+'`' for x in c['assertion_refs'])+'. Famílias: '+', '.join(c['test_refs'])+'.']
 out+=['## Budget e recuperação',t['budget_contract'],'**Rollback/recovery:** '+t['rollback']['procedure'],
 '**Rollout:** habilitar só depois das assertions, qualificação de célula e revisão. Flag não anuncia recurso não qualificado.','**Plano de evidências:** '+t['evidence_location'],
 '## Assertions deste pacote']
 for a in p['assertions']:
  if a['owner_package']!=t['id']:continue
  out += [f"### {a['id']}",f"Família `{a['test_id']}`; etapa `{a['stage']}`; estado `{a['status']}`.",
 '**Dado:** '+js(a['given']).strip(),
 '**Estímulo:** '+ ' '.join(a['when']),
 '**Predicado e observáveis:**\n```json\n'+js(a['then']).strip()+'\n```',
 '**Evidência:** resultado imutável por sujeito/assertion/célula/attempt, bytes referenciados e identidade do verificador. Uma suite verde sem esta assertion não fecha o critério.',
 '**Oráculo SHA-256:** `'+a['oracle_digest']+'`']
 out+=['## Famílias e células de aceitação']
 for id in t['acceptance_tests']:
  x=tidx[id];out += [f"### {id}",'Owner da família: '+x['owner_package']+'. Células: '+', '.join(x['required_cells'])+'. Etapa: '+x['stage']+'.',
 '**Preparo:** '+' '.join(x['preconditions']), '**Execução:** '+' '.join(x['procedure']), '**Esperado:** '+x['expected'], '**Artifact template:** `'+x['artifact']+'`']
 out+=['**Fechamento:** cinco grupos de axiomas demonstrados por assertions verificadas no candidato integrado, sem waiver. HUG-056 fecha somente após C e pós-publicação; demais estados seguem P00/P12.']
 return '\n\n'.join(out)+'\n'
def overview(p):
 counts={'wp':len(p['tasks']),'criteria':sum(len(v) for t in p['tasks'] for v in t['axioms'].values()),'assertions':len(p['assertions']),'tests':len(p['tests']),'obligations':len(p['obligations'])}
 return f'''# Hugit — planejamento standalone v3

**Revisão 3.0 • 16 de setembro de 2026 • baseline `{p['source_commit']}`.**

## Entrega e estado
Esta revisão endereça BR-01…BR-10 do segundo review. Mantém os {counts['wp']} work packages e os cinco axiomas em cada um, preserva AR-01…AR-16 e não modifica o código do Hugit. Os WPs continuam planned e as obrigações de produto not_run. Alterações de especificação, validação do plano, implementação, observação e revisão independente são decisões distintas.

**Direção:** memória operacional local — goal → tentativa → execução → resultado → evidência → recuperação. Hugit é 100% standalone, sem CoreLink, conta, backend, gateway, runner remoto ou Git AI obrigatório. Goal continua vindo do hook real qualificado em HUG-057, sem exigir disciplina do LLM. Ferramentas de build/cache/harness são integrações opcionais em runtime; testes da integração anunciada continuam obrigatórios.

## O que foi fechado no planejamento
1. Fonte normativa única e protocolos com versão/digest vinculados por consumidor; vistas geradas integralmente verificáveis.
2. Critérios ligados a assertions identificadas, com estímulo, predicado, observáveis e evidência. Etiqueta N/R ou suite green não demonstra automaticamente um invariante.
3. Promoção automatizada em namespace gerenciado imutável; handoff à branch humana é explícito. Witness legado não prova efeito de transação multi-ref inteira.
4. Entrega e revogação compartilham um gate; ACK tem semântica para bytes já em voo e para novas concessões.
5. Cold restore sem autoridade recente marca freshness unknown e bloqueia privado até nova decisão local; não inventa revogação não recebida.
6. Cursor invalida diante de query_generation divergente. Não se promete snapshot antigo a partir de projeção sobrescrita.
7. Namespace da emissão é estável; restart do collector muda entrega, não identidade do fato histórico.
8. Evidência e resultados são por sujeito, assertion, célula de ambiente e tentativa; agregação independe da ordem.
9. Sujeito A imutável, pacote B de qualificação e registro C de publicação eliminam autorreferência de commits e de fechamento.

Essas decisões têm custo explícito: consulta pode exigir reinício quando há escrita concorrente; revoke pode ficar pending se owner/FS não permite completar o gate; operação gerenciada não promete auto-update concorrente seguro de main. São contratos concretos, não garantias escondidas em adjetivos.

## Cinco axiomas sem exceção
Todos os WPs contêm Completeness criteria, Success criteria, Quality standards, Definition of Done e Invariants. São {counts['criteria']} critérios obrigatórios, ligados a {counts['assertions']} assertions de aceitação. Há {counts['tests']} famílias de testes e {counts['obligations']} obrigações família×célula. Esses números são denominadores do plano; **não são quantidade de testes do produto executados nem prova automática de suficiência**. As assertions que exigem inspeção de artefato têm revisão estruturada explícita; não se disfarçam de predicado automatizado já implementado.

Os casos concretos do review foram mantidos: criar quatro goals exige asked=4/done=0/proven=0; conteúdo igual em duas invocações e políticas exige ocorrências e permissões separadas. Remover a assertion específica invalida o critério mesmo se a família passar.

## Arquitetura e protocolos
SQLite embutido armazena eventos, relações, estado e projeções; grandes blobs são arquivos separados sob reserva/leases/GC. Não há transação mágica SQL+filesystem+Git. P01–P03 mantêm migração, publicação de objetos e drenagem; P04–P08 fecham emissão, autoridade, privacidade, processo, reports e leitura; P09 limita reuso; P10 qualifica integração real; P11 controla recursos; P00/P12 governam evidência e publicação.

**P10:** construir e testar candidato real num worktree privado/detached; publicar manifesto Git numa única `refs/hugit/operations/<operation_id>`; readback qualifica somente essa publicação. Hugit não atualiza automaticamente refs/heads, HEAD, index ou arquivos do usuário. Um handoff explícito com Git integra o commit escolhido; o recorder observa o resultado e não inventa autoria. Nenhum WP foi removido, mas a promessa perigosa de auto-update concorrente de branch humana foi substituída por esta superfície verificável. A interface/documentação deve tornar isso inequívoco.

**P06:** o gate é compartilhado por entrega e revoke. Staging longo fica fora; concessão/rename ou enqueue limitado e ACK são ordenados. Se o processo morre, journal é reconciliado antes de emitir ACK de revogação. Bytes já enfileirados são in-flight anteriores, não novos grants depois do ACK.

**P08:** cada mudança visível altera generation transacional. Uma nova página usa a generation esperada dentro da mesma transação de leitura e nega CURSOR_INVALIDATED se ela mudou. FTS atrasado retorna INDEX_NOT_READY. Bundle/snapshot explícito atende leitura longa estável, sem WAL reader infinito.

**P04:** dedupe lógico usa origin_namespace_id + source_event_id. collector_instance_id e delivery_id pertencem ao transporte; copiar/rotacionar fonte sem continuidade demonstrável não permite forjar idempotência exata.

## Ordem de execução
A sequência 0–100 é de marcos, não prazo ou porcentagem de linhas. A: inventário, regressões, goal observado e contenções; B: schema/freeze, política, store/receipts/blobs; C: primeira fatia útil; D: demais adapters/relatórios; E: caches nativos e integração; F: qualificação; G: publicação.

HUG-057 antecede schema; HUG-058 antecede cutover; HUG-059 antecede cada consumidor que exige freeze. HUG-060 permanece independente de Docker, Playwright, compiler cache, migração de instalações antigas ou landing real. A primeira fatia usa runtime novo isolado, mas não recebe waiver de privacidade/durabilidade/identidade.

Fatia: hook real → uma falha de teste → logs/report associados → nova sessão recupera perguntas predeterminadas → export/restore do mesmo corpus. Comparar com Git e logs/transcript efetivamente disponíveis, não com concorrente privado de seus arquivos. Outline hit não é evidência de economia de CI ou build.

## Qualificação A/B/C
A fixa código, receitas, assets e qualification-lock resolvido. B contém resultados por assertion/célula, artefatos e aprovações verificadas, fora de A. C contém destino, confirmação de publicação e pós-verificação, sem reconstruir A. Nenhum objeto precisa conter seu próprio hash final.

HUG-056 implementa/testa a automação antes do gate pré-promoção; fica ready_for_promotion com predecessores e assertions pre aprovados. Só fecha depois de publicar e verificar C. Full continua exigindo 60/60 e todos os cinco axiomas ao término; não exige conclusão posterior antes da própria ação.

Plan validation e preflight são ferramentas auxiliares; não concedem produção. O ledger de produto permanece vazio. O verifier real é trabalho de HUG-052/055/056, com trust roots fora do input e oráculo revisado. Controles locais desta entrega têm chaves e subjects de teste, explicitamente não aceitos para publicar Hugit.

## Fonte, integridade e limites
O pacote anterior e o review são preservados em source-inputs, sem alterações, como histórico não normativo. Protocolos, WPs e sidecars são gerados de backlog.json; editar uma vista manualmente faz o validador falhar. Uma atualização consistente ainda exige revisão semântica: digest não prova correção da política.

Resultados executados nesta rodada ficam em validation/, com distinção entre análise estrutural, mutação, referência executável e sonda nativa. Não houve nova execução do Hugit nem auditoria integral do Rust. Nenhum subagente independente foi iniciado. Baseline e caminhos do produto continuam fixados/qualificados pelas tarefas de descoberta; não foram inventados para preencher o plano.

## Uso
Leia EXECUCAO.md, RESPOSTA-AO-REVIEW-2.md e os contratos. Valide com `python scripts/validate_plan.py --root .`; rode a bateria em VALIDACAO.md. Comece pelos pacotes liberados no DAG e pela posse de arquivos, não pela ordem numérica. Toda mudança de protocolo versiona bindings e reabre consumers afetados.
'''
def response_md(p):
 out=['# Resposta aos dez achados — v3','Status: addressed_in_plan; implementação do Hugit e revisão independente pendentes. Nenhum caso anterior foi apresentado como novo teste de runtime.']
 for r in p['second_review_resolutions']:
  out += ['## '+r['id']+' — '+r['title'],'**Decisão:** '+r['resolution'],'**Protocolos:** '+', '.join(r['protocols'])+'. **WPs:** '+', '.join(r['packages'])+'.',
 '**Aceitação original preservada:** '+r['original_acceptance'],'**Família de fechamento:** `'+r['acceptance_test']+'`. Resultados reais por assertions/células, sem waiver.',
 '**Fingerprint da seção original:** `'+r['original_section_sha256']+'`. A seção integral está no JSON e no arquivo original preservado.']
 return '\n\n'.join(out)+'\n'
def views(p):
 v={'PLANO.md':overview(p),'RESPOSTA-AO-REVIEW-2.md':response_md(p)}
 for t in p['tasks']:v['work-packages/'+t['id']+'.md']=wp_md(t,p)
 for pro in p['protocol_registry']:
  v['contratos/'+pro['id']+'.md']=pro['normative_body'].rstrip()+'\n\n## Vínculo estruturado desta revisão\n\nVersão `'+pro['version']+'`; digest `'+pro['digest']+'`.\n\n```json\n'+js(pro['rules']).strip()+'\n```\n'
 for name,key in [('protocol-registry.json','protocol_registry'),('assertion-catalog.json','assertions'),('test-catalog.json','tests'),('capabilities.json','capabilities'),('gates.json','gates'),('release-profiles.json','profiles'),('matrix-cells.json','matrix_cells'),('obligations.json','obligations'),('review-resolution.json','review_resolutions'),('review2-resolution.json','second_review_resolutions')]:v[name]=js({'schema':'hugit-plan-view/3',key:p[key]})
 v['EXECUCAO.md']='# Execução e admissão\n\n'+p['protocol_registry'][0]['normative_body']+'\n\n'+p['protocol_registry'][12]['normative_body']+'\n\n## Comandos\n\n```sh\npython scripts/validate_plan.py --root .\npython scripts/test_plan_validator.py --root .\npython scripts/check_protocol_models.py --root .\npython scripts/probe_git_managed.py --root .\npython scripts/test_qualification.py --root .\npython scripts/validate_plan.py --root . --mode release-readiness\n```\n\nO último comando deve retornar not_ready neste pacote. Fechar produção exige execução, review e A/B/C reais; o algoritmo de demonstração usa credenciais locais de teste.\n'
 v['VALIDACAO.md']='''# Repetição e limites

Executar os comandos de EXECUCAO.md num diretório privado. Validadores, modelos e sonda Git usam somente fixtures; não modificam o repositório do usuário. Leia os scripts antes de executar arquivos vindos de terceiros. Scripts de render/validação/modelos/sonda usam Python stdlib e Git local; demonstração criptográfica exige cryptography instalado, sem instalação/rede automática. Ausência de dependência é setup_error e não passa.

`render_plan.py --root .` regenera vistas. `validate_plan.py` compara arquivos completos, regras/bindings, aliases e assertions. Mudanças no documento canônico exigem recalcular digests via `refresh_contracts.py`, rever consumidores e executar a bateria; o script de atualização não aprova semanticamente a revisão.

`test_plan_validator.py` contém controles positivos, ataques históricos relevantes e mutações novas; assertion removida, ambiente faltante, protocolo ausente, freeze/owner/capability errados precisam falhar. `check_protocol_models.py` é exploração limitada de referência, não modelo completo de SO/Git/SQLite. `probe_git_managed.py` é sonda nativa do candidato gerenciado com checkout concorrente e SQLite; não executa Hugit. `test_qualification.py` executa subprocesso real de fixture, assina evidencia com chaves efêmeras e verifica A/B/C — não conta como produto qualificado.

O verifier de hashes confere arquivos distribuídos, não suficiência de requisitos nem autenticidade externa. `validation/` armazena ensaios desta entrega; `evidence-ledger.json` fica sem testes de produto. Os modelos preservam controles inseguros: all-green sem contraexemplo não é evidência de que o ataque estava representado.
'''
 v['README.md']='# Hugit — planejamento v3\n\nComece em index.html ou PLANO.md. Correção do planejamento, não do código.\n\n'+response_md(p).split('## BR-01')[0]+'\nVer EXECUCAO.md/VALIDACAO.md. Fonte normativa única: backlog.json. Inputs anteriores preservados por hash.\n'
 v['FONTES.md']='# Fontes técnicas e materiais\n\nO pacote v2 e seu review são entradas primárias preservadas em source-inputs. Referências novas consultadas em 16/09/2026:\n\n'+ '\n'.join('- '+x['id']+': '+x['url']+' — '+x['used_for'] for x in p['new_primary_sources'])+'\n\nAs fontes técnicas justificam limites de APIs, não comprovam implementação do Hugit. Fontes anteriores permanecem no campo sources do documento canônico.\n'
 v['CONTRATOS.md']='\n\n---\n\n'.join(v['contratos/'+x['id']+'.md'] for x in p['protocol_registry'])
 v['WORK-PACKAGES.md']='\n\n---\n\n'.join(v['work-packages/'+x['id']+'.md'] for x in p['tasks'])
 return v
# Intentionally small safe renderer for our own generated markdown: no raw HTML or scripts.
def inline(s):
 s=html.escape(s);s=re.sub(r'`([^`]+)`',r'<code>\1</code>',s);s=re.sub(r'\*\*([^*]+)\*\*',r'<strong>\1</strong>',s);return s

def md(s):
 out=[];code=[];in_code=False;table=False
 for line in s.splitlines():
  if line.startswith('```'):
   if in_code:out.append('<pre><code>'+html.escape('\n'.join(code))+'</code></pre>');code=[]
   in_code=not in_code;continue
  if in_code:code.append(line);continue
  if line.startswith('|'):
   if not table:out.append('<div class="table-wrap"><table>');table=True
   if re.match(r'^\|[\s|:\-]+\|$',line):continue
   cells=line.strip().strip('|').split('|');out.append('<tr>'+''.join('<td>'+inline(x.strip())+'</td>' for x in cells)+'</tr>');continue
  if table:out.append('</table></div>');table=False
  if not line.strip():continue
  m=re.match(r'^(#{1,6}) (.*)',line)
  if m:level=min(5,len(m.group(1))+1);out.append(f'<h{level}>'+inline(m.group(2))+f'</h{level}>')
  elif line.startswith('- '):out.append('<p class="listrow">• '+inline(line[2:])+'</p>')
  elif line=='---':out.append('<hr>')
  else:out.append('<p>'+inline(line)+'</p>')
 if table:out.append('</table></div>')
 return '\n'.join(out)

def html_view(p,v):
 cards=[]
 for pro in p['protocol_registry']:
  cards.append(f'<details class="entry protocol" id="{pro["id"]}"><summary><span class="tag">{pro["id"]}</span>{html.escape(pro["title"])}</summary><div class="inside">{md(v["contratos/"+pro["id"]+".md"])}</div></details>')
 for t in p['tasks']:
  cards.append(f'<details class="entry wp" id="{t["id"]}"><summary><span class="tag">{t["id"]}</span><span>{html.escape(t["title"])}</span><small>lane {t["owner_lane"]} · {len(t["depends_on"])} deps</small></summary><div class="inside">{md(v["work-packages/"+t["id"]+".md"])}</div></details>')
 rows=''.join(f'<a href="#{t["id"]}">{t["id"]}</a>' for t in p['tasks'])
 css='''*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;font:16px/1.6 system-ui,sans-serif;background:#eef2f6;color:#162b3d}a{color:#0c6079}header{background:#122b40;color:white;padding:46px max(24px,calc((100vw - 1180px)/2)) 38px}header h1{font-size:clamp(30px,4vw,49px);line-height:1.15;margin:10px 0 20px}header p{max-width:850px;color:#dce5ed}.kicker{letter-spacing:.15em;text-transform:uppercase;font-size:12px;color:#9fd8d6}.badges{display:flex;flex-wrap:wrap;gap:9px}.badges span{border:1px solid #658198;padding:5px 11px;border-radius:5px;font-size:13px}main{max-width:1180px;margin:0 auto;padding:25px 20px 75px}.notice{background:#fff6df;border-left:4px solid #c38b20;padding:14px 20px;margin:0 0 22px}.toolbar{position:sticky;top:0;z-index:5;display:flex;gap:10px;align-items:center;flex-wrap:wrap;background:#eef2f6f8;padding:12px 0}input{font:inherit;padding:11px 15px;border:1px solid #b6c6d4;border-radius:5px;flex:1;min-width:170px}button{font:inherit;background:#fff;border:1px solid #aabfce;padding:10px 13px;border-radius:5px;color:#162b3d;cursor:pointer}.panel{background:white;padding:24px;border:1px solid #d3dfe7;border-radius:8px;margin-bottom:22px;min-width:0}.panel h2{font-size:27px;margin-top:0}.entry{background:white;border:1px solid #ccdbe5;border-radius:6px;margin:10px 0;scroll-margin-top:100px}.entry[open]{border-color:#4c889e}summary{padding:16px 18px;cursor:pointer;font-weight:650;display:flex;gap:12px;align-items:center}summary:before{content:'+';font-size:20px;color:#2b7988}.entry[open]>summary:before{content:'−'}summary small{margin-left:auto;white-space:nowrap;font-weight:400;font-size:12px;color:#617789}.tag{font-family:ui-monospace,monospace;font-size:13px;background:#e7f0f3;padding:3px 7px;border-radius:4px;white-space:nowrap}.inside{padding:0 24px 25px;border-top:1px solid #dbe5ec}.inside h2{font-size:24px}.inside h3{font-size:20px;border-top:1px solid #dbe5ec;padding-top:22px}.inside h4{font-size:17px;color:#145e70}p,li,td{overflow-wrap:anywhere}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f2f6f8;border:1px solid #d5e1e8;padding:14px;font-size:12px;max-width:100%;line-height:1.6}code{font-size:.88em;background:#eef3f6;padding:1px 4px;border-radius:3px}.table-wrap{max-width:100%;overflow:auto}table{border-collapse:collapse;width:100%;font-size:14px}td{padding:10px 12px;border:1px solid #d5e2e9;vertical-align:top}tr:first-child td{background:#edf3f7;font-weight:650}.index{display:flex;gap:8px;flex-wrap:wrap}.index a{font-size:12px;font-family:ui-monospace,monospace;text-decoration:none;background:#edf3f7;padding:4px 7px}.hidden{display:none!important}.count{font-size:13px;color:#51697c}footer{border-top:1px solid #cfdae3;padding-top:20px;color:#526b7c;font-size:13px}button:focus-visible,input:focus-visible,summary:focus-visible,a:focus-visible{outline:3px solid #ce8b26;outline-offset:3px}@media(max-width:600px){header{padding:30px 20px}main{padding:18px 12px}.panel{padding:17px}summary{align-items:flex-start;flex-wrap:wrap;font-size:14px;padding:13px}summary small{margin-left:0;width:100%}.inside{padding:0 15px 18px}.toolbar{gap:6px}button{padding:9px;font-size:13px}input{min-width:180px}td{min-width:150px}}@media print{.toolbar,.index{display:none}body{background:white}header{padding:20px;color:#132d43;background:white}header p{color:#233}main{max-width:none}details{break-inside:auto}summary{break-after:avoid}.inside{display:block}}'''
 script='''const entries=[...document.querySelectorAll('.entry')]; const q=document.getElementById('q');const count=document.getElementById('count');const cached=entries.map(e=>e.textContent.normalize('NFD').replace(/[\\u0300-\\u036f]/g,'').toLowerCase());function filter(){const s=q.value.normalize('NFD').replace(/[\\u0300-\\u036f]/g,'').toLowerCase().trim();let n=0;entries.forEach((e,i)=>{const ok=!s||cached[i].includes(s);e.classList.toggle('hidden',!ok);if(ok)n++});count.textContent=n+' contratos/pacotes visíveis';}q.addEventListener('input',filter);document.getElementById('open').onclick=()=>entries.filter(e=>!e.classList.contains('hidden')).forEach(e=>e.open=true);document.getElementById('close').onclick=()=>entries.forEach(e=>e.open=false);function jump(){const id=decodeURIComponent(location.hash.slice(1));const el=document.getElementById(id);if(el&&el.matches('details')){q.value='';filter();el.open=true;setTimeout(()=>el.scrollIntoView(),10)}}window.addEventListener('hashchange',jump);filter();jump();'''
 return '<!doctype html><html lang="pt-BR"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Hugit • Planejamento standalone v3</title><style>'+css+'</style></head><body><header><div class="kicker">Engenharia • contratos • evidência</div><h1>Hugit<br>Planejamento standalone v3</h1><p>Dez achados endereçados na especificação. Sessenta work packages, cinco axiomas obrigatórios e evidência por assertion e célula — sem transformar plano validado em produto validado.</p><div class="badges"><span>100% standalone</span><span>60 WPs preservados</span><span>BR-01…BR-10</span><span>Implementação pendente</span></div></header><main><div class="notice"><strong>Estado:</strong> correção do planejamento. Nenhuma alteração de código ou GitHub; WPs planned, qualificação de produto não executada. Resultados desta rodada: validation/ no pacote.</div><section class="panel" id="overview">'+md(v['PLANO.md'])+'</section><section class="panel" id="review">'+md(v['RESPOSTA-AO-REVIEW-2.md'])+'</section><section class="panel"><h2>Índice dos work packages</h2><div class="index">'+rows+'</div></section><div class="toolbar"><input type="search" id="q" aria-label="Buscar em contratos e pacotes" placeholder="Buscar WP, BR, assertion, assunto…"><button id="open">Abrir visíveis</button><button id="close">Fechar todos</button><span id="count" class="count" aria-live="polite"></span></div>'+''.join(cards)+'<footer>Fonte normativa: backlog.json. Vistas geradas; inputs anteriores preservados. Cinco axiomas não equivalem a cinco checkboxes: cada afirmação requer seu resultado verificável.</footer></main><script>'+script+'</script></body></html>'

def all_views(p):
 v=views(p);v['index.html']=html_view(p,v);return v

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);args=ap.parse_args();p=json.loads((args.root/'backlog.json').read_text())
 for n,s in all_views(p).items():f=args.root/n;f.parent.mkdir(parents=True,exist_ok=True);f.write_text(s)
 print(json.dumps({'rendered':len(all_views(p)),'source':'backlog.json','scope':'plan_views'},ensure_ascii=False))
if __name__=='__main__':main()
