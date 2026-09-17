# Hugit — planejamento standalone v3

**Revisão 3.0 • 16 de setembro de 2026 • baseline `626fbaac6deac84c29b9c6686f013c1fbfc7ad4c`.**

## Entrega e estado
Esta revisão endereça BR-01…BR-10 do segundo review. Mantém os 60 work packages e os cinco axiomas em cada um, preserva AR-01…AR-16 e não modifica o código do Hugit. Os WPs continuam planned e as obrigações de produto not_run. Alterações de especificação, validação do plano, implementação, observação e revisão independente são decisões distintas.

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
Todos os WPs contêm Completeness criteria, Success criteria, Quality standards, Definition of Done e Invariants. São 900 critérios obrigatórios, ligados a 927 assertions de aceitação. Há 267 famílias de testes e 857 obrigações família×célula. Esses números são denominadores do plano; **não são quantidade de testes do produto executados nem prova automática de suficiência**. As assertions que exigem inspeção de artefato têm revisão estruturada explícita; não se disfarçam de predicado automatizado já implementado.

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
