# Execução e admissão

# P00 — Autoridade normativa, cinco axiomas e fechamento sem circularidade

## Fonte de verdade e limites
`backlog.json` é a fonte normativa única desta revisão. Contém o corpo de cada protocolo, regras estruturadas, sua versão/digest, contratos dos WPs, assertions e matrizes. Markdown/HTML/sidecars são projeções geradas determinísticas. `validate_directory` compara o conteúdo completo de cada vista normativa e falha se falta ou diverge. Um hash prova igualdade; não torna uma decisão correta ou uma revisão independente. Mudança autorizada exige nova versão, revisão de impacto e atualização dos consumidores; atualizar só o hash não aprova uma política.

## Axiomas obrigatórios
Cada WP conserva Completeness criteria, Success criteria, Quality standards, DoD e Invariants. Não há waiver, N/A, grupo vazio nem substituição por herança genérica. Não existe quota obrigatória de frases: acrescentar propriedades quando houver fronteira real. Cada critério tem assertion_refs explícitas; cada assertion define preparo, estímulo, observáveis, predicado, evidência requerida e critérios cobertos. Uma assertion pode servir vários critérios se demonstrar todos. Passar a família N/R não aprova automaticamente suas assertions nem invariantes.

Um predicado que exige julgamento de artefato é `structured_review`: o reviewer identifica os bytes e preenche cada afirmação observada com rationale e evidência. Não fingir que uma comparação de strings provou comportamento. Assertions de runtime são implementadas pelo WP, inspecionadas antes de habilitar a capacidade e executadas sobre A. O catálogo presente é especificação desses testes, não resultados de produto.

## Ready e congelamento
Estado comum: planned → ready → executing → implementation_review → integrated_verified → closed; blocked/reopened interrompem; waived não existe. Todos os consumidores com requires_runtime_protocol_freeze=true têm HUG-059 entre seus ancestrais e bindings exatos de versão/digest. HUG-001 produz inventário; HUG-057 produz fixtures do hook real; HUG-015 produz schema; HUG-059 produz freeze. Esses produtores não exigem seu próprio output para começar. Contenções anteriores ao freeze são explicitamente classificadas baseline_containment; não habilitam runtime v3 implicitamente.

Write-sets são propostas a resolver contra checkout por HUG-001. Mudanças compartilhadas passam pelo integrador; tarefas prontas com mesmo path continuam serializadas. Path traversal, symlink de vista normativa ou protocolo ausente invalidam pacote. A mudança de origem/ref do código requer identidade verificável e análise de alcance, não substituição pelo nome da branch.

## Identidade e evidência
Tests têm owner único correto; artifacts são resultados por sujeito/teste/assertion/célula/attempt, não um caminho sobrescrito. Capabilities e perfis possuem denominador explícito: full mantém os 60 WPs e as capacidades acordadas, inclusive HUG-041. Testes compartilhados usam referência ao mesmo resultado imutável; nenhum artefato diferente compartilha pathname sem agregador tipado. Não é necessário duplicar bytes para múltiplos critérios.

## A, B e C
A é sujeito imutável de código/build; B é pacote de qualificação que aponta A; C registra promoção e verificação posterior de A e B. Evidence não precisa ser commitada em A. Se for versionada no Git, usar ref/commit separado e nunca reconstruir A sob a identidade antiga.

HUG-056 é dispatchável após predecessores integrados e qualificados. Sua implementação, rehearsal, negative tests e revisão precisam estar aprovados antes de ready_for_promotion. Seu DoD completo depende de publicação e verificação posterior: ready_for_promotion → publishing → published_unverified → post_publish_verified → closed. Falha depois de publicar deixa published_unverified/recovery_required, não closed. Todos os axiomas continuam obrigatórios; a etapa de verificação é ordenada, não dispensada. O gate pré-promoção não exige HUG-056 já fechado; o final exige 60/60 fechados. A closure de HUG-055 é o pacote B destacado e revisado, não um commit de autoatestação dentro de A.

## Validação estrutural versus decisão de produto
O validador testa relações e igualdade das vistas, não interpreta toda prosa nem certifica suficiência de cada assertion. Preflight informa pendências e nunca publica. O verificador autoritativo de HUG-052/055/056 exige resultado real, artifact digest, trust policy e aprovações externas verificadas. Fixtures sintéticas são controles do algoritmo; jamais contam no ledger de produto. Um achado novo reabre WPs/claims atingidos.


# P12 — Qualificação por células, sujeito imutável e publicação finita

## Três decisões
Plan validation verifica schema, protocolos/digests/consumidores, assertions, ownership, matriz finita e vistas. Preflight relata pendências, é não autoritativo e nunca publica. Product qualification verifica execução real sobre A, artefatos/saídas, assertions e aprovação independente segundo trust policy externa ao ledger. Scripts/models distribuídos nesta revisão são controles do planejamento, não substituem esse terceiro sistema.

## Sujeito A, qualificação B e promoção C — BR-10
A = manifesto canônico de código/tree/commit, build recipe, toolchain, dependências, qualification-lock e digests dos assets finais. Seu digest identifica os bytes testados; A não contém B nem seu próprio digest embutido. Assinatura de manifesto usa envelope destacado. Qualificação B aponta A, plan digest, versões de protocolos/assertions, resultados e aprovações verificadas. B pode ser armazenado em arquivos ou ref de evidence separada. Publicação C aponta A e B, registra destino/asset IDs/readback/digests e incidente/rollback; não altera A.

Gate pré-promoção exige todos os WPs em integrated_verified/closed, implementação da automação 056 testada e revisada, assertions pre_promotion completas e B aprovado. **Não exige 056 closed antes de publicar.** Gate final exige C, assertions post_publication e todos os 60 closed. Falha após publicação não permite substituir bytes silenciosamente na mesma versão. Pre-ready, published_unverified, post_publish_verified e closed são estados distintos. Testar finitamente A→B→C, incluindo A alterado, B adulterado e pós-verificação falha. Anotações em C não invalidam bytes A; novo código/asset em A exige nova qualificação B.

## Obrigação de matriz — BR-09
Uma família test_id é instanciada nas required_cells declaradas. Cada célula fixa target, filesystem, adapter/configuration e tipo de avaliação; seu qualification lock final contém versões/ambiente/tool digests resolvidos e revisados antes de A. Não usar latest ou environment vazio. Nem todo teste precisa do cartesiano completo: documentos têm célula artifact-review; core executável tem quatro células nativas; adapters especializados têm células finitas adicionais explicitadas. Runtime opcional não dispensa teste da integração anunciada.

Chave de resultado = (subject_digest, plan_digest, protocol_set_digest, test_id, assertion_id, cell_id, qualification_lock_digest, attempt_id). Obrigação = mesma chave sem attempt. Resultados são append-only; duplicata byte-idêntica é retransmissão; conflito na mesma identidade é invalid. Tentativas divergentes não usam last-wins: falha no mesmo candidato/célula permanece blocker, salvo decisão de infra invalidation assinada pelo revisor com justificativa e nova execução; falha semântica não é dispensada. Na referência conservadora desta revisão nenhuma falha é invalidada automaticamente. Ordem dos registros não altera pendências ou readiness.

Cada assertion requerida precisa de output estruturado e referência aos bytes; um passed de suite não substitui assertions individuais. Não contar teste de Linux como Windows. Evidência de outro A ou cell/lock é rejeitada. Artefato compartilhado só por resultado imutável endereçado, não overwrite de path. Catálogo contém selectors legíveis por máquina; A fixa o ambiente efetivo, pois o plano não inventa versões não observadas do hook.

## Perfis
Full mantém 60 WPs, os 16 AR e os 10 BR, 23 capacidades e gates G0…G9. Early-slice continua milestone interno sobre HUG-060; não declara full nem antecipa publicação. Adapters e caches permanecem opcionais ao instalar/usar o core. Linux x86_64/ext4, macOS arm64/APFS, macOS x86_64/APFS e Windows x86_64/NTFS são as células nativas core a qualificar; WSL não substitui Windows. FS/versão adicional é nova célula revisada.

HUG-056 tem famílias P/N/Q/R de pré-qualificação e família POST de confirmação real. Todos os seus cinco grupos de axiomas permanecem obrigatórios; a assertion de pós-publicação só pode fechar depois de C. Nenhum hash de commit precisa conter a si próprio. CI que versiona evidência usa commit distinto de A e não recompila assets silenciosamente.

## Autenticidade e controle positivo
O verifier real pinna identidades de executor/reviewer e valida assinatura/provenance e digest dos arquivos em B, contra trust policy admitida fora dos dados. Assinar bytes de um relatório não prova sozinho que o teste aconteceu; o executor autenticado atesta execução e o reviewer qualifica o oráculo e os observáveis. Perfil same-user não oferece autoridade independente contra conta comprometida. O controle positivo local demonstra algoritmo de A/B/C com subprocesso real e assinaturas de teste, explicitamente não confiáveis para produto. Sem executor aprovado real, preflight sempre not_ready/external_verification_required.

## Testes negativos obrigatórios
Reordenar Windows failed/Linux passed; omitir uma célula; trocar target/FS/env/adapter; colidir attempt ID; duplicar assertion com resultado conflitante; alterar A mantendo B; faltar pós-verificação; commitar B dentro de A; excluir HUG-041/capability; protocolo/consumer drift; retirar assertion específica e manter suite green. Controles positivos cobrem todas as células de uma fixture limitada, e A→B→C termina sem autorreferência.

## Implementação da automação antes do freeze final
HUG-052/053 criam receitas/verificadores e ensaiam candidatos provisórios; esse ensaio não torna um subject final imutável antes de terminar os writers de código. HUG-055/056 também entregam código/políticas/templates antes do freeze final. Sua evidência destacada B/C é produzida depois, sem modificar esses arquivos.

Depois de todas as unidades que alteram fonte estarem integradas, congelar A e seu qualification-lock, recipe e catálogo de oráculos. Executar/verificar todas as obligations do perfil sobre A; evidência de candidato provisório anterior não satisfaz automaticamente essa fase. Uma correção de código cria novo A e requer nova qualificação, mas gravar B/C não muda A nem cria série infinita de commits. WPs podem fornecer versão de interface integrated_verified para consumidores antes da qualificação final de release; isso não declara sua matriz full concluída.

Resultado de fechamento BR de integração pertence a HUG-055, depois dos componentes. O consumidor individual implementa e verifica sua parcela por suas assertions próprias. Não introduzir dependência oculta de um componente em sua própria integração downstream. Early-slice verifica uma célula interna explicitada e não fecha WPs full cuja matriz restante ainda está pendente.


## Comandos

```sh
python scripts/validate_plan.py --root .
python scripts/test_plan_validator.py --root .
python scripts/check_protocol_models.py --root .
python scripts/probe_git_managed.py --root .
python scripts/test_qualification.py --root .
python scripts/validate_plan.py --root . --mode release-readiness
```

O último comando deve retornar not_ready neste pacote. Fechar produção exige execução, review e A/B/C reais; o algoritmo de demonstração usa credenciais locais de teste.
