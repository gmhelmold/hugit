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

## Vínculo estruturado desta revisão

Versão `3.0`; digest `5d737267087e24bbcca9548488c719f4cca46fa3435a0ccf31b7b98696fde5cc`.

```json
[
  {
    "id": "P00.freeze",
    "decision": "all_flagged_consumers_require_freeze_ancestor",
    "mandatory": true
  },
  {
    "id": "P00.assertions",
    "decision": "criterion_closure_requires_named_assertions_not_suite_boolean",
    "mandatory": true
  },
  {
    "id": "P00.views",
    "decision": "exact_generated_views_required",
    "mandatory": true
  },
  {
    "id": "P00.approval",
    "decision": "plan_validation_is_not_product_qualification",
    "mandatory": true
  }
]
```


---

# P01 — Migração e coexistência de escritores v1/v2

**Owner:** B. **Versão:** 3.0. **Achados:** AR-01, AR-12, AR-15. **Estado:** especificação proposta, não implementação.

## Autoridade e alcance
O v1 não passa a interpretar campos novos por introduzirmos um marker v2. Lock do SO só coordena participantes compatíveis. O relatório AR-01 contém ensaios reais do v1 que invalidam as duas suposições; o mecanismo abaixo precisa dos mesmos binários reais nos testes de implementação. [Entrada: review AR-01; S03]

**Escolha:** cold migration controlada; runtime v2 em diretório novo, separado do formato v1. Não executar dual-write independente. A lista de caminhos admitidos inclui o canonical log, runtime metadata, legado configurado, sidecars e aliases reais. Writes explícitos para um log arbitrário criado pelo próprio usuário não modificam o banco v2 e não são apresentados como history canônica; esse desvio deliberado está fora da garantia de coexistência cooperativa.

**Guarda proposta e obrigatoriamente qualificada:** substituir os caminhos de arquivos v1 graváveis admitidos por diretórios não vazios com manifesto de guard, após preservar os bytes. Isso deve fazer os caminhos legados reais de read/atomic rename falharem. É uma técnica a testar, NÃO uma garantia baseada somente em nome de arquivo. Um writer conhecido que tenha acesso de escrita in-place a descritor já aberto exige quiescência comprovada por seu mecanismo supervisionado; caso contrário a migração recusa. Nenhum marcador ou permissão controlada pelo próprio usuário protege contra esse usuário deliberadamente removê-la.

## Estados e efeito durável
| Estado | Condição de entrada/efeito | Recuperação |
|---|---|---|
| PREPARED | Matriz de versões/paths, espaço e backup bootstrap HUG-058 disponíveis. Nada muda na fonte. | Abortar sem tocar fonte. |
| QUIESCED | Novas sessões/hook entrypoints gerenciados pausados; writers compatíveis fechados; writers v1 admitidos testados. Timeout não equivale a quiescência. | Manter captura pausada; não prosseguir por idade/PID. |
| GUARDED | Cada arquivo v1 preservado e path guardado; operação journal externo ao path guardado; guardas verificadas. | Se criação conflita, preservar qualquer nova escrita e abortar/reconciliar; não escolher a cópia conveniente. |
| SNAPSHOTTED | Fonte final congelada, inventário de receipts, sidecars e hashes confirmado depois das guardas. | Revalidar conteúdo; mudança inesperada bloqueia import. |
| IMPORTED | V2 importa em staging com origin namespace e bytes v1 originais. | Retomar por operation/source IDs, nunca reimport duplicado. |
| VERIFIED | Contagens, hashes, relações e receipt coverage conferidos por verificador independente; old writer probes recusam. | V2 permanece inativo se qualquer desigualdade. |
| ACTIVE | Pointer v2 + generation/operation manifest publicados duravelmente; guardas antigas permanecem. | Depois de write v2, apenas forward repair/restore compatível. |

Durante a pequena janela de instalar guardas, uma escrita legada concorrente torna a instalação conflitante ou a fonte divergente; a implementação deve detectar e bloquear ativação. Não afirmar atomicidade multi-path inexistente. Processos v1 já em voo que usam temp+rename só podem passar o gate se os ensaios mostrarem que o guard impede seus writes após o corte. Matriz não qualificada => `LEGACY_PATH_UNQUALIFIED`, sem override silencioso.

## Journal, rollback e receipts
O migration journal contém operation_id, phase, source manifest/digest, admitted paths, guard hashes, imported head e primeiro write v2. Persistir a fase somente após seu efeito verificável. Receipt confirmada antes do corte pertence ao conjunto preservado; novas tentativas durante pausa têm estado de captura indisponível/pendente, não contagem inventada de perda zero.

Backup mínimo HUG-058 não depende de export final HUG-039. Ele é uma cópia privada congelada com leitor/verificador v1 independente e teste de restore em staging. Antes de ACTIVE/primeiro write v2, rollback pode restaurar v1 se paths e writers continuam quiescentes. Após write v2, downgrade de binário não reconstrói histórico v1: manter guardas, snapshot v2, reparar/abrir com leitor compatível e preservar novos eventos. Nada é “reversível” apenas por reverter commit.

## Ensaios exigidos
V1 publicado por digest: processo pausado antes do rename, iniciado depois do corte, lock antigo com mtime 130 s, aliases, --log e variáveis admitidas; dois migradores; erro em cada write/fsync/rename/phase; backup sem export completo. Resultado deve ser ausência de writer legado modificando história migrada, ou recusa antes da ativação. Demonstração apenas com mock que conhece v2 é insuficiente.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `c1c8435afafdab3706f1a06f7462a6e43ce880c224e1c62dadda8f985b0e16a3`.

```json
[
  {
    "id": "P01.cutover",
    "decision": "legacy_writer_barrier_requires_real_binary_qualification",
    "mandatory": true
  },
  {
    "id": "P01.rollback",
    "decision": "no_destructive_downgrade_after_v2_events",
    "mandatory": true
  }
]
```


---

# P02 — Transação de objetos: SQLite, arquivos, leases e GC

**Owner:** B. **Versão:** 3.0. **Achados:** AR-02, AR-09, AR-16. **Estado:** especificação proposta, não implementação.

## Decisão de storage
SQLite embutido guarda eventos, relações e estados transacionais; blobs ficam fora dele. WAL tem um writer por vez, e leitores longos podem impedir checkpoint. A API de backup serve à cópia consistente do banco. Essas propriedades não criam transação SQL envolvendo rename/unlink externos. [S01, S02]

Ativar FK e verificar PRAGMAs efetivos; `synchronous=FULL` quando um ack significa durabilidade no perfil qualificado. Fixar SQLite/binding com correções vigentes e registrar versões reais. Staging v2 é separado do runtime v1 até P01. Projeções têm versão e watermark reconstruíveis; eventos e a projeção canônica pertinente são commitados juntos. Index derivado pode atrasar, mas sua ausência é reportada e nunca apaga eventos.

## Identidades e retenção
`blob_id` identifica conteúdo; `representation_id` codec/ciphertext; `occurrence_id` o uso/path/produtor. Uma ocorrência histórica não fixa automaticamente bytes para sempre. `retention_roots` (pins, policy retain e bundles fixados) e leases de uso protegem blobs/representações requeridos. Linhagem permite listar derivados e propagar revogação. Recompressão não funde ocorrências nem libera material em uso.

## Tabela de transições
| Transição | Guarda transacional | Efeito fora do DB | Commit/recuperação |
|---|---|---|---|
| none→STAGING | Ticket global + reserva repo + owner instance existem; quota admitida. | Criar tmp no-follow em root privada. | Persistir staging e budget antes de admitir bytes. |
| STAGING→AVAILABLE | Owner válido; tamanho e digests conferidos; policy epoch permitido. | sync bytes; publish sem overwrite de objeto existente diferente; sync diretório onde qualificado. | Só depois marcar AVAILABLE e ligar ocorrência. Crash antes deixa STAGING recuperável. |
| AVAILABLE→read/pin | Epoch permitido, material requerido available; ausência de DELETING. | Abrir descritor bound à representação. | Lease durável e handle de owner antes de responder “disponível”. |
| AVAILABLE→DELETING | Sem retention root, sem lease/produtor vivo; deleter obtém posse. | Unlink somente representação/mapeamento possuído. | Nenhuma nova lease pode entrar; release/retry explicitam unavailable. |
| DELETING→EVICTED | Unlink confirmado ou arquivo já ausente do mesmo ticket. | Sync da remoção onde contrato exige. | Estado histórico continua; budget só é liberado depois de reconciliação. |
| STAGING→ABORTED | Owner morreu/completou e isso foi confirmado por coordenação. | Apagar tmp possuído; blob publicado sem referência é conciliado. | Nada confirmado como AVAILABLE pode sumir como simples temporário. |

Dedupe busca estado na mesma transação que concede a relação/lease; não faz `stat→assume retained`. Se o objeto está DELETING, produtor espera/retry ou recria após EVICTED como nova representação de mesmos bytes, sem reutilizar lease inválida. Delete ticket carrega generation; um cleanup antigo nunca apaga representação nova com mesmo pathname.

## Posse e morte
Lease possui token aleatório, installation/boot/producer instance e handle de coordenação não herdável. TTL pode alertar, mas não prova morte. Reaper só remove lease depois de conseguir exclusividade compatível e confirmar identidade; processo vivo pausado conserva proteção. No restart da instalação, owners de boot anterior são reconciliados antes da liberação. Locks e DB não são mantidos durante stream inteiro; leases são registros protegidos por posse, não transações SQL longas.

Leitor recebe descritor e versão autorizada. Enquanto a lease é válida, GC não remove seu objeto. Revogação de política corta novas entregas/chunks pelo P06; não consegue apagar bytes já entregues. Deadlines encerram leitores lentos por contrato, não roubam suas leases para continuar lendo de arquivo apagado.

## Verificação
Schedules obrigatórios: publish×GC; AVAILABLE read×delete; pin×selection stale; dedupe×delete/recreate; export×GC; processo pausado além de TTL; crash entre sync/publish/DB/ack; quota reduzida. Modelo finito com mutação de guardas serve para refutar falhas de desenho; aceitação do código exige failpoints reais e não basta o modelo passar.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `85eae8e64cbdfe8f597e2811f3a482d40f5096373e9f43a74890ba3f3d19e8f8`.

```json
[
  {
    "id": "P02.gc",
    "decision": "delete_requires_state_transition_and_no_live_roots_or_leases",
    "mandatory": true
  },
  {
    "id": "P02.publication",
    "decision": "available_after_bytes_verified_and_published",
    "mandatory": true
  }
]
```


---

# P03 — Ack de captura, handoff e progresso limitado

**Owner:** B. **Versão:** 3.0. **Achados:** AR-07, AR-08, AR-16. **Estado:** especificação proposta, não implementação.

## Confirmações distintas
`HOOK_RETURNED` não é durabilidade. `ACK_RECEIPT` só depois de bytes autorizados persistidos conforme FS; `EVENT_VISIBLE` depois de evento+projeção commitados; `ARTIFACTS_READY` depois de todas as referências exigidas estarem prontas ou explicitamente incompletas. Resultado inclui IDs e watermark; nenhum estágio é inferido de exit 0 do hook. Receipts não incluem blobs grandes.

## Handoff saudável
Há um coordination handle estável `handoff.lock` e outro `drainer.lock` por repo. Producer prepara bytes/ticket bounded, entra no handoff, publica receipt, marca demanda e verifica/assume responsabilidade por drainer. Se nenhum drainer possui execution lock, inicia worker que concorre pela posse. Wakeups duplicados são baratos e idempotentes. Worker processa em lotes, renovando seu trabalho enquanto a carga finita tem fila; limite de lote não autoriza abandonar o restante.

Para sair normalmente, worker entra no mesmo handoff, reconsulta a fila/demanda, e **libera drainer.lock antes de liberar handoff** somente se não há trabalho. Um producer chegando depois vê posse livre; um producer que chegou antes deixou demanda que a última checagem vê. Não basta olhar queue-empty fora da coordenação.

## Falhas e garantia de liveness
Sem crashes e com storage/escalonamento qualificados, carga finita admitida converge; T-drain registra ack→event→artifact separadamente. Spawn failure mantém receipt e resposta `SPAWN_FAILED`/pending. Um worker que morre depois de ACK não pode garantir retomada autônoma quando não existe outro processo vivo. O perfil standalone sem serviço permanente oferece recuperação na próxima interação instrumentada ou `health --recover`/`wait`; essa limitação é parte do contrato, não uma exceção escondida.

`wait --receipt` usa deadline e assume um drainer enquanto o chamador vive; não chama LLM. Startup e término de sessão dos adapters qualificados invocam recovery/flush automaticamente, sem pedir ao modelo para lembrar. `health` puro mostra estado sem escrever; modo recover é explícito. Não declarar número exato de eventos perdidos antes de receipt se a fonte não fornece contagem verificável.

## Pressão e visibilidade
Receipt ≤64 KiB; campo de texto além do permitido vai para objeto sob política ou é omitido com razão. Se não há budget antes do receipt, recusar admissão; não emitir ACK durável. Se objeto ainda não pronto, evento mostra pending/partial. Controle mínimo de health tem reserva separada para explicar pressão; até essa reserva pode falhar em disco fisicamente cheio, caso em que a resposta viva informa indisponibilidade sem prometer log durável.

Cenários obrigatórios: última chegada entre empty e exit, lote parcial, worker morto, wakeup duplo, spawn failure, renúncia por deadline com passagem de responsabilidade, source IDs retransmitidos. Cron, polling agressivo ou um daemon novo não são usados para ocultar protocolo errado.

## Identidade no replay — vinculante BR-08
ACK_RECEIPT devolve chave de emissão persistida `(origin_namespace_id,source_event_id)` e delivery_id. Worker não substitui o namespace por seu boot. Unique constraint verifica bytes canônicos conflitantes; checkpoint de leitura atrasado pode reentregar sem contar nova execução. Canal after órfão não inventa before. Demonstração obrigatória: cair entre commit e watermark, reiniciar outro collector e observar uma ocorrência, duas entregas.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `5a1fafc04faf82e1edd1a87b2de77eaad0f4e8392d1acf0f48bc3d5244745192`.

```json
[
  {
    "id": "P03.handoff",
    "decision": "producer_and_final_empty_check_coordinate",
    "mandatory": true
  },
  {
    "id": "P03.replay",
    "decision": "dedupe_by_stable_emission_namespace_not_collector",
    "mandatory": true
  }
]
```


---

# P04 — Modelo de dados e encoding de identidades

**Owner:** A. **Versão:** 3.0. **Achados:** AR-06, AR-09, AR-10, AR-12. **Estado:** especificação proposta, não implementação.

## IDs não intercambiáveis
| ID | Identidade | Regra |
|---|---|---|
| history_id | História local preservável/exportável. | Restore-recovery conserva; clone novo cria e aponta origem. |
| installation_id | Instalação ativa da história neste host/root. | Novo em restore em nova instalação; não herdado do bundle. |
| origin_namespace_id | Domínio estável de emissão da fonte qualificada (adapter, source-store, session/generation). | Persiste em replay, import e reinício de coletor. |
| collector_instance_id | Instância efêmera que transportou a observação. | Muda ao reiniciar; nunca entra na chave do evento histórico. |
| delivery_id | Tentativa de entrega de um evento lógico. | Nova por retransmissão; não conta como nova invocação. |
| source_event_id | Evento emitido pela origem, dentro de origin_namespace_id. | UNIQUE(origin_namespace_id,source_event_id); bytes diferentes são conflito; collector não altera a chave. |
| goal_id + revision | Objetivo capturado e revisão explícita. | Não inferir de branch ou de toda mensagem; HUG-057 resolve fonte. |
| invocation_id | Uma tentativa concreta. | Mesmo comando executado de novo recebe outro ID. |
| action_id | Trabalho determinístico sob contrato de reuso. | Não conta invocações; perfil e versões fazem parte da chave. |
| snapshot_id | Manifesto do conteúdo efetivamente materializado/capturado. | Não se resume a HEAD; dirty state e qualidade separados. |
| blob_id | Bytes lógicos autorizados. | Independe de path, nome, codec e run. |
| representation_id | Bytes armazenados e encoding/cipher metadata. | Recompressão/reencriptação muda a representação. |
| artifact_occurrence_id | Uso de blob/árvore num run/path/media/mode. | Duas ocorrências não se fundem pela dedupe do conteúdo. |
| derivation_id | Transformação de inputs por parser/redaction/policy version. | Novo conteúdo/normalização é derivado, não evento observado novo. |
| evidence_binding_id | Uso de resultado/artefato por consumidor e critério/revision. | Independente de existência prévia da action. |

## Encoding canônico proposto
IDs gerados usam nonce de 256 bits e prefixo de tipo. IDs por conteúdo usam SHA-256 com domínio `hugit.v2/<kind>\0` e campos byte-length-prefixed (`u64_be length || bytes`), sem floats. Inteiros possuem representação binária fixa explicitada nos vetores. `blob_id` inclui domínio, byte length e **bytes lógicos após transformação autorizada**. `representation_id` usa manifest canônico do codec/cipher/versões, blob_id e digest dos bytes armazenados. Identificador não transmite raw nem chave; plaintext digest privado não aparece em interface de menor privilégio.

Campos de resultado semanticamente inteiros não aceitam NaN/float de serialização. Maps canônicos ordenados por bytes de chave e vetores explícitos; tempo de origem preservado como fonte, observed_at como chegada, duração monotônica por execução. A cadeia local tem digest/version separados de source IDs; head local não é testemunha independente contra administrador same-user.

## Arquivos e árvores
Path lógico é lista de componentes codificados por plataforma/bytes, não concatenação de strings com `/` arbitrário. No restore, validar separators, dotdot, links, names reservados, case folding e colisões de normalização no target. Não normalizar Unicode silenciosamente e sobrescrever o outro arquivo. Devices, sockets, FIFO e setuid não são materializados por default; manifest pode descrevê-los como não suportados. Modos executáveis admitidos entram na ocorrência/árvore, não alteram blob dos bytes.

## Estados ortogonais
Proveniência do produtor (`declared/harness_observed/process_observed/imported`) é atribuída pelo canal; autenticação/permissions são outra entidade. Completeness (`complete/partial/truncated/unknown`), outcome (`passed/failed/skipped/no_tests/cancelled/timeout/unknown`), availability (`staging/retained/external/evicted/missing/quarantined`), applicability (`same_inputs/stale/incompatible/unknown`) e reuse decision (`denied/policy_eligible/native_tool_decides`) permanecem independentes.

Run terminal pode ter artifacts pending; report coerente pode ter vínculo incerto; aprovação é declaração. Quatro goals criados devem continuar done=0. Retries não apagam falha, e source finish órfão não ganha start inventado.

## Testes obrigatórios
Recompressão, redaction, mesmo bytes em nomes diferentes, duas instalações de mesmo bundle, nonce/produtor reutilizado, clocks regressivos, Unicode/case, dois goals na mesma sessão, revision sem ID explícito. O schema é congelado somente depois do contrato real de goal HUG-057; fixtures do site do harness sozinhas não comprovam o hook do usuário.

## Emissão estável, entrega efêmera — BR-08
Chave lógica = `(origin_namespace_id, source_event_id)`. O namespace identifica source store/session e geração da emissão; não é PID, novo boot do collector ou texto do payload. Antes do primeiro ACK, registrar a identidade de emissão e a receipt de maneira recuperável. Ao reentregar, conservar a chave e os bytes originais, acrescentando delivery_id/collector_instance_id separados. Commit do evento e checkpoint de leitura podem ocorrer separadamente: crash entre eles produz replay, não outro fato.

Fonte com ID nativo estável conserva-o. Um wrapper próprio gera um UUID de emissão antes de confirmar e preserva na receipt. Reader de arquivo sem ID nativo só oferece exactly-once lógico se qualifica um stream_generation_id durável + offset/record-id com prova de continuidade do arquivo; offset, mtime ou hash do payload isolados não bastam. Rotação/truncamento/reset registrados criam nova geração. Continuidade não demonstrável resulta `SOURCE_CONTINUITY_UNKNOWN` e quarentena/declaração de incompletude, nunca dedupe por texto ou reuse arbitrário de namespace. HUG-057/023/024 congelam fixtures reais dessa regra.

Rerodar o mesmo comando gera invocation e emissão novas. Reimportar um evento preserva o namespace histórico; novo trabalho após restore usa nova origem da instalação. Copiar transcript sem lineage suficiente não recebe identidade histórica presumida. Mesmo evento, dois collectors: uma ocorrência lógica e duas entregas; mesmos bytes, duas emissões reais: duas ocorrências.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `e968bb6e71fbb085c3a94f2a706f1573b437f9355dfec1325c7c04c17de43bf9`.

```json
[
  {
    "id": "P04.identity",
    "decision": "origin_namespace_plus_source_event_is_logical_event_key",
    "mandatory": true
  },
  {
    "id": "P04.delivery",
    "decision": "collector_and_delivery_are_not_emission_identity",
    "mandatory": true
  },
  {
    "id": "P04.artifact",
    "decision": "content_representation_occurrence_policy_are_distinct",
    "mandatory": true
  }
]
```


---

# P05 — Canais de origem, permissões e fronteira same-user

**Owner:** D. **Versão:** 3.0. **Achados:** AR-05, AR-06. **Estado:** especificação proposta, não implementação.

## Decisão
O processo/adaptador qualificado estabelece origem; input só oferece fatos. `process_observed` não é aceito como elevação escolhida num arquivo JSON. Import cria `imported` e conserva uma declaração de produtor original separada. O canal possui root aprovada, installation_id, adapter/version, tipos de evento permitidos, policy epoch e limites. Objetos são resolvidos dentro desse escopo, mesmo quando seus IDs são globalmente conhecidos.

Roots/executável/permissão de export/open/network vêm de configuração do usuário, não de parâmetros de uma tool que lê evidência. Defaults de CLI/MCP são locais e read-first. Hugit não inicia URL/viewer/shell por conteúdo de report; opening de viewer é ação explícita separada e com warning de conteúdo não confiável.

## Confiança e isolamento
Core local confia na integridade da conta/host que roda o recorder. Não promete conter teste hostil executado com acesso de escrita aos mesmos arquivos. Permissão user-only e assinatura local não mudam isso. Um perfil futuro hostil requer isolamento qualificado e consentimento; não é requisito de Docker obrigatório nem parte de uma promessa inexistente do core. Até haver essa qualificação, origin attribution é observação cooperativa, não atestação independente.

Um log pode conter instruções maliciosas. Marcar como dado ajuda o consumidor, mas não prova que qualquer LLM sempre resistirá. A mitigação verificável é estrutural: conteúdo não altera capabilities; leitura não aciona efeitos; root e canais não são derivados do log; raw privado não é exposto por uma query de classe tratada. A autorização de ação sensível está fora do texto que a sugere. [S08]

## Goal e harness
HUG-057 encontra o hook real e as versões antes de schema final. Fonte com goal_id/revision explícitos conserva essa relação. Fonte só com prompt/session/call produz observação de mensagem e binding `unassigned` até uma relação legítima; não há classificação LLM obrigatória nem atribuição ao “goal mais recente”. O hook real existente não é substituído por formulário. Duas tarefas simultâneas constituem fixture obrigatória, não só caminho feliz de uma sessão.

Adapters mesclam config por ownership/hash e não modificam seleção de tools ou comandos de trabalho. Source after sem before pode ser preservado como órfão; gap de output é propagado. Eventos de outros repositórios não entram por herança de env apenas: validar cwd/root e evidência de origem, sem extrapolar permissão.

## Verificação
Import alegando `process_observed`, ID de blob de outro repo, log que pede export fora da root, source event de producer diferente, callback after órfão, dois goals mesmo worktree e versão de harness com campo retirado. Esperado: captura limitada/erro/ambiguidade explicável, nunca autoridade a mais.

## Origem e transporte — vinculante BR-08
Canal autorizado resolve origin_namespace_id estável da fonte qualificada; collector_instance_id identifica só quem entregou. Nenhum payload importado escolhe privilégio; import conserva uma origem histórica como dado, com trust=imported. Reset/rotação da fonte possui geração própria. Declaração de idempotência exata é negada quando a continuidade não pode ser demonstrada.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `9683d0acafd218be2078fba9ca22087a56ca933152d1ba62b0d930778f367564`.

```json
[
  {
    "id": "P05.authority",
    "decision": "ingress_channel_assigns_trust_not_payload",
    "mandatory": true
  },
  {
    "id": "P05.roots",
    "decision": "root_permissions_outside_untrusted_evidence",
    "mandatory": true
  }
]
```


---

# P06 — Política de conteúdo, criptografia privada e revogação

**Owner:** D. **Versão:** 3.0. **Achados:** AR-04, AR-05, AR-09. **Estado:** especificação proposta, não implementação.

## Classes e defaults fechados
| Classe | Captura/persistência | Consulta/index | Export |
|---|---|---|---|
| treated_text | Regras de transformação qualificadas antes de persistência; qualidade de tratamento indicada, sem promessa de apagar todo segredo arbitrário. | Apenas interfaces com permissão dessa classe; lineage e epoch obrigatórios. | Perfil sanitizado autorizado, com lista de transformações e omissões. |
| opaque_private | Opt-in para cópia; registro metadata mínimo pode existir sem o blob. Trace/ZIP/binário não se tornam tratados por scrub externo. | Não indexar nem enviar a contexto por default. | Consentimento explícito por seleção; nunca “sanitized” implicitamente. |
| raw_private_opt_in | Autorização separada + armazenamento cifrado em formato de envelope mantido e já revisado; sem chave não persistir em claro. | Somente canal privado explícito; sem FTS/preview tratado. | Backup privado autorizado com política de chaves; chave não é incluída inadvertidamente. |

Escolha de formato privado: age v1 com recipients X25519, usando implementação mantida e versão exata congelada em HUG-020/052, com vetores de interoperabilidade e autenticação. Não implementar primitivas criptográficas próprias. [S10] Uma chave de captura pública permite cifrar; a chave privada de leitura é provisionada pelo usuário em armazenamento local protegido e não acompanha o bundle. Backup e teste de recuperação de chave são obrigatórios para habilitar raw; integração com keychain é conveniência opcional, não requisito cloud. Se chave não pode ser provisionada/recuperada no modo suportado, `KEY_UNAVAILABLE`: guardar metadados permitidos/omissão, não fallback em claro. O gate raw só fecha quando um formato e fluxo reais forem qualificados; nem flag `encrypted=true` nem chmod cumprem o contrato.

## Dados derivados e epoch
Todo derivado referencia inputs, policy version, parser/transform version e conteúdo produzido. Tabela de acesso aplica epoch atual antes de index/materialized view. Revogar obedece ao gate de entrega abaixo; o ACK é emitido somente depois do commit do novo epoch sob esse gate. Rebuild de FTS/context/summary ocorre depois; projections velhas não recuperam autorização. Arquivos raw, índices temporários e diagnostic logs seguem a mesma classificação; quarantine com bytes também exige autorização de retenção.

Consulta usa concessão de entrega coordenada, não check-then-use. Export prepara bytes em staging privado antes da seção crítica; publicação e revoke compartilham o gate estável. Restore aplica a política conhecida localmente, mas diferencia import numa instalação com âncora atual e recuperação a frio sem ela; backup antigo sozinho não comprova atualidade de revogações.

## Ponto de linearização de entrega e revoke — BR-05
Existe um gate nativo estável por domínio de política/repositório. Ordem: gate → transação curta SQL → publicar localmente/enfileirar chunk limitado → registrar resultado → liberar gate. GC não segura locks SQL enquanto espera esse gate; seleção e leases ocorrem antes, evitando ordem invertida.

Exporter grava e verifica staging sem o gate. Depois adquire gate, recupera intents de entrega interrompidas, confere policy/autorizações da seleção atual e registra `delivery_intent` durável. Ainda sob gate, faz rename no mesmo filesystem para destino aprovado, sincroniza diretório conforme perfil e confirma `DELIVERED`. Não segura SQL transaction durante cópia longa; rename/publicação tem journal com digest/destino/operation_id. Se algo falha, não afirmar entrega nem autorizar replay para novo destino. Crash após rename: próximo owner reconcilia existência/digest sob gate antes de responder revoke. Witness insuficiente -> DELIVERY_AMBIGUOUS e diagnóstico, sem repetir exposição.

Revoke adquire o mesmo gate; antes de ACK resolve/cancela intents anteriores que poderiam ainda publicar, comita epoch novo e invalida elegibilidade. Seu ACK `NO_NEW_DELIVERIES` impede novas concessões sob o epoch antigo. Se owner está pausado ou FS impede recuperação, responder REVOKE_PENDING/erro, nunca ACK falso nem roubar lock por TTL.

Para páginas/streams, gate protege apenas enqueue não bloqueante de até 64 KiB no transporte autorizado, não uma espera pelo leitor. SQL registra concessão com epoch e contador; EAGAIN libera sem autorização reutilizável e nova tentativa revalida. Bytes já enfileirados são explicitamente entrega concedida antes do ACK e podem chegar ao leitor depois; não são recolhíveis. Revogação impede novos chunks, não retrocede buffers do SO. Abortando a sessão, não enviar remainder salvo sob novo grant válido. Locks de política e transações têm budget; não prometer ACK com deadline absoluto sob disco/owner travado.

O teste pausa antes de adquirir o gate: revoke comita primeiro e publish antigo deve falhar. Se exporter está dentro do gate, revoke não pode ACK antes de seu publish/cancel/recovery. `checar imediatamente antes` sem exclusão é um controle negativo obrigatório.

## Recuperação a frio sem autoridade sobrevivente — BR-06
Dois modos: `authority_preserved` usa tombstones/epoch do destino que já conhece decisões recentes; merge de bundle nunca os apaga. `cold_restore` sem âncora externa ao backup marca policy_freshness=unknown. Texto tratado histórico é lido só sob política default restritiva; nenhuma classe privada ou derivado anteriormente proibido é automaticamente liberada. Não afirmar que o epoch do backup é o mais recente.

Um kit local independente de políticas/tombstones pode ser fornecido pelo usuário. Validar identidade, integridade e vínculo à história, e declarar limite de freshness; sem fonte que prove que é atual, ainda desconhecido. O proprietário pode emitir nova autorização local de recuperação, com seleção, classes e audit record novos. Isso é uma decisão presente, não reconstrução de revogações perdidas. O core permanece offline e sem serviço obrigatório. Mesmo backup em dois mundos indistinguíveis produz o mesmo estado conservador; não inventar informação ausente.

## Limites explícitos de eliminação
Retirar acesso futuro nas interfaces e índices controlados é requisito. Não prometer apagamento forense de qualquer SSD ou revogação de pacote já compartilhado. `secure_delete` não resolve automaticamente vestígios de FTS; essa limitação precisa estar no runbook. [S04] Metadados canônicos sensíveis devem ser evitados; mudanças de política não reescrevem hashes antigos para fingir que nunca houve conteúdo. Backup privado de uma época anterior continua sensível e seu acesso tem que ser protegido pelo usuário.

## Transformações em streams
Regras operam com lookbehind limitado documentado; secret split que exceda capacidade de tratamento resulta em omit/truncated/unknown, nunca `sanitized_complete` por omissão. Bytes lógicos pós-redaction recebem novo blob_id; representação de mesmo conteúdo pode mudar codec sem mudar blob. Chaves, digests de plaintext privado e nomes sensíveis não vazam via paths/IDs de interfaces menos privilegiadas.

## Ensaios
Plantar tokens em argv, filename, env, texto dividido em chunks, ZIP/trace e report HTML. Depois revogar e consultar todos os índices, ranges, packs, exports e imports de backup antigo. A classe tratada não recupera raw ou derivado proibido. O teste distingue impossibilidade de recuperar bytes já entregues da falha de controle de acesso futuro.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `2e6d36db28e0cd10d7ec732507b96a8428c4c6dcc6ad7e0c0a9b4cdb5cf27fc9`.

```json
[
  {
    "id": "P06.revocation",
    "decision": "shared_delivery_gate_linearizes_publish_and_revoke_ack",
    "mandatory": true
  },
  {
    "id": "P06.cold_restore",
    "decision": "unknown_freshness_denies_private_until_new_local_authorization",
    "mandatory": true
  },
  {
    "id": "P06.buffer",
    "decision": "previously_enqueued_bytes_may_arrive_after_ack",
    "mandatory": true
  }
]
```


---

# P07 — Processos e streams: observação sem transparência fictícia

**Owner:** C. **Versão:** 3.0. **Achados:** AR-08, AR-16. **Estado:** especificação proposta, não implementação.

## Perfil suportado
`observe -- <argv>` executa comando não interativo com cwd autorizado, stdin herdado não copiado e stdout/stderr em pipes independentes. Sem shell implícito: executar `sh -c` é escolha explícita. Se a sessão é TTY/interativa, recusar captura integral por default; `--noninteractive` é consentimento explícito para o perfil pipe e suas diferenças. PTY não faz parte da promessa pipe/full desta expansão; adicioná-lo exige contrato próprio, não é chamado “sem diferença” silenciosamente.

Redirecionar stdout pode alterar isatty, cores e buffering. Isso deve aparecer na documentação e na comparação recorder off/on no mesmo modo não interativo, não ser tratado como transparência absoluta. Não capturar conteúdo de prompt de senha por tee de stdin.

## Ownership de execução
Supervisor prepara grupo/job antes de lançar trabalho. Unix: grupo dedicado, handles de recorder non-inheritable; Windows: job sem breakaway qualificado antes de aceitar lifecycle. Nunca sinalizar grupo do harness. O perfil core trata processos cooperativos e testes locais; escape deliberado same-user não é isolamento provado. Caso não se consiga atribuir processo ao mecanismo nativo, `PROCESS_SETUP_FAILED` antes de lançá-lo, em vez de execução sem supervisão escondida.

Deadline monotônico inclui filho, drains e descendentes admitidos. Child exit não encerra deadline enquanto pipe continua aberto. Timeout gera resultado timeout+partial e cleanup com tolerância de 500 ms no fixture qualificado; não marca saída zero anterior como execução íntegra. Cancelamento repetido é idempotente, mas relançar trabalho não é. Sinais/PIDs/handles são validados como propriedade da operação; sem kill genérico por nome.

## Saídas e pressão
Pumps mantêm offsets de stdout/stderr, segmentos de tamanho limitado, EOF e gaps separados. Sem ordem global perfeita entre streams. Storage/codec rodam fora do pump; fila cheia/disk lento fazem captura drenar e descartar com counters, evitando wait infinito por recorder. Preservar forwarding de stdout pode sofrer a pressão do consumidor original; esse problema é distinguido do armazenamento e continua coberto pelo deadline. Modo preview pode truncar visualização, explicitamente, sem declarar log completo.

Quando limite de bytes ou policy impede armazenar, evento de run pode existir com incomplete; report estruturado é mantido quando permitido como canal próprio, sem converter truncamento em completude total. Capture-only hook permanece silencioso e não devolve logs como instruções do harness.

## Contrato de exit
Child concluído: preservar código 0..255; `exit_origin=child`. Falha antes de launch: 125/setup; timeout: 124/supervisor; signal POSIX: 128+signal com raw signal em JSON; code 124 de child não se chama timeout sem evento supervisor. Windows preserva código nativo em JSON e representação CLI documentada. Erro do recorder não mascara erro do trabalho. Child sucesso + recorder failure mantém código child, salvo `--require-evidence`, que retorna erro do recorder com `exit_origin=recorder`. Hooks Git observacionais continuam success/fail-open por falha do observador; esse contrato não é aplicado a gates shell de check.

## Ensaios
stdin binary, isatty, SIGINT/TERM, child exit/neto vivo, stdout e stderr floods, consumidor lento, disk full, pipe fechado, assignment failure Windows e secret split. Oráculo verifica códigos, efeitos de downstream `&&`, status da captura, bytes/gaps, cleanup e sobrevivência do harness.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `995564c0e30b81dbf5523fc72b8960a812edabcf576a85d551cd17fa86e49694`.

```json
[
  {
    "id": "P07.terminal",
    "decision": "child_and_capture_outcomes_separate",
    "mandatory": true
  },
  {
    "id": "P07.mode",
    "decision": "noninteractive_pipes_not_universal_tty_equivalence",
    "mandatory": true
  }
]
```


---

# P08 — Reports, busca, bundles e restore por origem exata

**Owner:** F. **Versão:** 3.0. **Achados:** AR-10, AR-12, AR-04. **Estado:** especificação proposta, não implementação.

## Report não se associa por mtime
Preferir diretório vazio exclusivo `runs/<invocation_id>/outputs/` criado pelo supervisor/adapter e parâmetro de reporter ligado à execução. Capturar producer/run witness quando a ferramenta fornece. Quando só se encontra arquivo externo, import é permitido como `imported` com vínculo alegado/unknown; nem XML válido nem mtime recente o promove a arquivo produzido pela tentativa atual.

Erro de compilação, setup failure, timeout, cancelamento ou shards faltantes impede declarar suite completa passada. Run exit e report outcome permanecem campos separados: inconsistência é `report_process_conflict`, não “escolher o verde”. Relatório antigo verde + compilação atual falha => test_not_run/failed process, sem casos aprovados atribuídos à nova execução. Mesmo com output directory exclusivo, código de projeto hostil pode mentir; o contrato é observação local cooperativa, não atestação externa.

## Cases, attempts e shards
Identidade do caso inclui produtor/suite/case/parametrização normalizada e config relevante; tentativa/retry possui número e run ID próprios. Shard aggregate tem expected set fixado antes ou origem declared_unknown; só `complete` quando todos os shards requeridos finalizados e sem conflito. IDs duplicados incompatíveis são invalid, não last-wins. Zero casos é no_tests. Dialetos JUnit são enumerados por versão; XML external entities/DTD externas/rede desabilitados, tamanho/profundidade/elementos limitados.

Build, benchmark e Playwright estendem o mesmo envelope com finalization e unidades. Buildx export usa ref exata e finalização qualificada; `.dockerbuild`, imagem e cache são classes diferentes. [S05] Ausência de ferramenta opcional não impede o core. Reports textuais genéricos permanecem úteis sem parser estruturado; não se inventa structured_complete a partir de saída truncada.

## Consulta e bundles
Cursor é ligado a history/installation, query_digest, ordenação total (tie-break ID), query_generation e policy epoch. **Escolha core: invalidar, não fingir snapshot antigo.** Todo commit que altera evento/projeção/join/availability/FTS/estado visível incrementa query_generation na mesma transação; epoch de política também invalida. Em cada página, uma transação curta lê generation e dados juntos; divergência em relação ao cursor retorna CURSOR_INVALIDATED sem uma página parcial ou omissão silenciosa. O output inclui último cursor válido e causa; repetir a consulta é ação explícita do consumidor. Mudanças irrelevantes podem invalidar conservadoramente na v1 deste contrato; custo aceito e medido, sem prometer estabilidade sob escrita contínua.

Não basta `seq <= watermark` sobre rows sobrescritas. FTS atrasado retorna INDEX_NOT_READY (ou busca exata por ID sem FTS), não resultado vazio completo. Index rebuild/GC incrementam generation. Antes da entrega, validar generation/epoch sob gate P06; se mudou durante a leitura, negar o chunk. Ordenação total está no query digest. Para percorrer dataset estável longo, usar bundle/snapshot read-only explícito já qualificado em HUG-039; não segurar leitor WAL indefinidamente.

Lookup por ID continua disponível sem FTS. Read lease antecede resposta; defaults 8 KiB, página 64 KiB, request 1 MiB e deadline P11. Conteúdo escapado em terminal/HTML, preservando bytes lógicos. Teste: página 1 → status/deleção/novo resultado → página 2 recebe CURSOR_INVALIDATED; sem mudança recebe o restante exato sem repetição. Incluir empate, FTS atrasado, policy revoke e compactação.

Backup privado guarda corte consistente, objetos selecionados e versões. Bundle sanitizado é derivado com omissões. Verificador inspeciona paths/hashes/schema/contagens sem executar executável, hook ou URL presente no bundle. Export adquire selection leases duráveis e usa snapshot DB independente para evitar segurar reader WAL durante cópia longa. A publicação adquire o gate de entrega P06; check de epoch isolado nunca concede publish.

## Restore explícito
**Recovery:** continuar history_id em nova installation_id/producer instance; IDs históricos permanecem origem. **Clone:** nova history_id ativa com relação ao snapshot importado; IDs antigos namespaceados. **Viewer import:** leitura offline sem ativar hooks, jobs, executable paths ou markers. Restaurar duas vezes nunca duplica identidade de producers futuros. Import repetido deduplica eventos históricos por namespace/source ID+bytes e rejeita colisão divergente.

Destino existente recusa por default. Expansão em staging privado impede path traversal e colisões; publish só depois de verificação completa. Arquivos temporários do autor não são reinstalados como paths válidos. Git history opcional é bundle real de objetos/ref selection, não repo sintético anunciado como backup da origem. Política local conhecida não é relaxada por bundle antigo. Cold restore sem âncora de política recente segue P06: freshness unknown, privado bloqueado até nova autorização local explícita; não afirmar sobrevivência de tombstones que não foram recebidos.

## Limites de parser e testes
Defaults propostos: report 32 MiB, profundidade XML/JSON 64, no máximo 100 mil casos por report e 1 milhão elementos; archive 100 mil entries, expansão total reservada antes, ratio máximo configurado/limitado, links não seguidos. Exceder retorna partial/refusal explícita, não report vazio verde. Guardar relatório verde antigo, shard ausente, arquivo parcial, duas restores+novos eventos+import repetido, Unicode/case/ZIP Slip e GC×export são negativos obrigatórios.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `0d6a0d18c9b72c52ff3eef81f0b156b20e753a501bbd95027a4da8ba2b0c46e2`.

```json
[
  {
    "id": "P08.cursor",
    "decision": "generation_mismatch_invalidates_no_silent_omissions",
    "mandatory": true
  },
  {
    "id": "P08.report",
    "decision": "syntactic_validity_does_not_prove_current_run_origin",
    "mandatory": true
  },
  {
    "id": "P08.restore",
    "decision": "cold_restore_cannot_know_unreceived_revocations",
    "mandatory": true
  }
]
```


---

# P09 — Reuso permitido somente em perfis executáveis qualificados

**Owner:** C. **Versão:** 3.0. **Achados:** AR-11, AR-09, AR-10. **Estado:** especificação proposta, não implementação.

## Três operações distintas
Consultar observação passada é sempre possível quando dados/permissões permitem. Afirmar que ela pertence a determinados bytes requer qualidade de origem/snapshot. Substituir execução futura exige fechamento de inputs, ambiente e outputs. Um before/after hash igual não impede que um input tenha mudado e voltado; readonly snapshot não impede leitura de rede, relógio, /proc, serviço ou biblioteca dinâmica mutável.

A separação action cache/CAS é usada como referência: metadata de uma ação e disponibilidade de seus arquivos são coisas distintas. Não copiar árvores de cache para fingir validade. [S06]

## Perfil mínimo útil obrigatório
**`outline-v1`:** função em processo que recebe bytes completos+language+parser version+grammar version+options+implementation digest e retorna JSON de outline. Inputs só por valores imutáveis; nenhum shell, IO arbitrário ou efeito externo exigido no caminho cacheável. A action key inclui todos esses campos, schema e policy profile. Miss computa e preserva JSON; hit devolve os mesmos bytes após verificar representação e versão. Modificar grammar/flags/input/producer invalida. Pelo menos um arquivo real e um corpus multilíngue demonstram reuso útil. Isso não é descrito como garantia de qualquer suíte arbitrária.

**`native-tool-decides`:** BuildKit e ferramenta de compilação qualificada decidem suas keys/validade/hit. Hugit preserva oferta/refs e relata decisão observada, não concede policy_eligible genérico. HUG-041/042 incluem trabalho real e formato público, evitando que toda a entrega fique em “deny tudo”.

**`shell-observed`:** testes shell/Cargo genéricos, serviços, relógio, randomness e side effects não fechados permanecem observation-only e são executados novamente. O usuário declarar cacheable=true não eleva esse perfil. Skip manual existe somente como decisão declarada, nunca prova de execução ou resultado aplicável.

## Guardas de elegibilidade
Exigir: source autorizado; inputs completos materializados para o perfil; actual executable/implementation identity quando pertinente; configuração/test selection/policy idênticos; determinismo da classe qualificada; required outputs disponíveis e íntegros; nenhum side effect requerido fora desses outputs; resultado terminal completo sem infra/timeout/cancelled; ausência de flake não comprovada não se presume. Unknown de qualquer campo necessário => deny com reason.

Namespace v1 não serve resultado para v2 elegível sem reexecução/qualificação. Métricas contam invocações distintas; saved_ms é estimativa da duração histórica evitada, enquanto wall time/CPU/I/O são medidos separadamente. TTL influencia retenção, não prova validade.

## Contrapontos e aceitação
Teste real outline MISS/HIT, versão de grammar alterada, output evicted, digest corrompido, ação repetida para outro consumidor. Negativos: shell disfarçado de outline; serviço/banco externo alterado com código igual; fixture fora da captura; mesmo PATH com executable substituído; probe unavailable; report de outro run; resultado flaky só pelo retry verde. Deve haver deny/refusal/native miss conforme classe, sem preencher sucesso.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `46cda356f2a1682342d7470e649ed81c902f659d77b0b67ef8b84cf5f1a251ad`.

```json
[
  {
    "id": "P09.reuse",
    "decision": "only_qualified_outline_or_native_tool_decides",
    "mandatory": true
  },
  {
    "id": "P09.claims",
    "decision": "outline_hit_is_not_ci_savings_proof",
    "mandatory": true
  }
]
```


---

# P10 — Integração real, publicação gerenciada e entrega explícita ao Git do usuário

## Decisão fechada e alcance
O motor cria uma integração Git real, testa a árvore candidata exata e publica o resultado em namespace Hugit reservado. **A automação não move refs/heads/*, HEAD, index ou arquivos dos worktrees do usuário.** A entrega à branch de trabalho é uma ação Git explícita do usuário, por exemplo `git merge --ff-only <candidate_oid>` no worktree escolhido, com observação posterior. Isso mantém integração local real; não anuncia promoção automática concorrente segura de branches humanas que Git externo possa fazer checkout sem respeitar locks Hugit. O perfil full inclui candidato integrado, validação, recuperação e handoff completo; auto-update de main concorrente não é vendido como garantia demonstrada. Não há troca silenciosa da promessa: esta decisão substitui o target+witness v2 e é exposta em CLI/docs/tests.

## Candidato e seleção
Preparar commit candidato em worktree de integração privado/detached. Armazenar base observada, member OIDs/ordem, tree OID, profile e policy. Checks executam a árvore materializada correspondente; mutação dos inputs exige nova validação. Remover um locus de falha não autoriza remainder: revalidar conjunto final, no máximo 64 probes por padrão; esgotamento/infra/unknown mantém held. Estado managed_published não significa integrado na branch do usuário nem remote push accepted. Se base humana avançar, manter candidato histórico e indicar stale_base; handoff não reusa validação de uma árvore alterada.

## Uma ref de autoridade por operação — BR-03
Antes de Git, SQL cria operation_id único com candidate_commit/tree, validation_id, base OID e digests. Produzir um commit de metadados no Git com manifesto canônico contendo esses campos e candidate como parent. Após validar todos os objetos, criar **uma única** ref nova `refs/hugit/operations/<operation_id>` apontando esse commit via create-if-absent/no-deref. Nenhum segundo ref é necessário para inferir o efeito. Readback resolve ref direta, manifesto, parent e tree; mismatch mantém AMBIGUOUS. Não usar symref recebida de input; namespace privado tem dono local cooperativo. Uma ref já existente com bytes diferentes é conflito, não update automático.

A propriedade certificada é somente: **o pacote candidato validado foi publicado nessa ref gerenciada**. Não é testemunho de alteração em refs/heads, e não prova causalidade de outro movimento. Alias opcional de descoberta é projeção descartável e nunca prova. SQL registra MANAGED_PUBLISHED apenas após readback exato; crash entre ref e SQL é reconciliado pelo operation_id e manifesto. Não repetir efeitos no worktree humano para reparar o ledger.

## Estados e recuperação
| Estado | Efeito | Recuperação |
|---|---|---|
| PREPARED | Journal sem ref publicada | Abortar ou validar; nenhum efeito humano. |
| VALIDATED | Candidate/validation congelados | Preparar objetos e ref gerenciada. |
| OBJECTS_READY | Objetos existem; publicação pode não existir | Verificar closure de objetos antes de criar ref. |
| REF_OBSERVED | Uma ref resolve manifesto exato; SQL pode atrasar | Reconciliar efeito gerenciado; não assumir branch humana. |
| MANAGED_PUBLISHED | Manifesto Git e SQL concordam | Oferecer handoff explícito; não mover ref duas vezes. |
| HANDOFF_REQUESTED | Usuário escolheu candidato/branch fora do loop de captura | Observação Git identifica resultado disponível; sem proof de ação, declarar apenas target_observed. |
| USER_INTEGRATION_OBSERVED | Git observado contém candidato/árvore esperada | Registrar fato e fonte, sem inferir autoria exclusiva da ação. |
| AMBIGUOUS | Ref/objeto/journal divergem ou falha de durabilidade | Preservar; nenhum reset/retry de efeitos humanos. |

Ref ausente depois de crash permite retry somente de criação do mesmo nome e mesmo manifesto, com CAS de inexistência e sem side effect humano; race com outro criador resolve igualdade/conflict. Não remover lockfile de Git apenas por idade: ownership/failure recovery qualificados. `process crash` e `machine/power failure` são células diferentes; full exige testes/declaração de durabilidade no filesystem admitido. Sem qualificação de crash de máquina, não conceder aquele grau de durabilidade.

## Estados parciais herdados não são apagados do modelo
Conservar a contraprova do protocolo antigo: destination-only, witness-only, both, neither após cada persistência, para target-first e witness-first. Witness-only jamais prova branch movida. Destination-only pode ser mudança observada, não commit concluído de toda operação. Na v3 nenhum desses estados converte automaticamente em MANAGED_PUBLISHED: somente a ref única do manifesto correto faz isso. Modelos devem incluir objetos escritos, ref renomeada, readback e SQL, e recovery de ausência/corrupção; não condensar dois recursos no mesmo passo.

## Checkout concorrente — BR-04
Teste obrigatório com Git normal: depois da observação de branch livre, criar worktree na branch humana e só então publicar o candidato gerenciado. HEAD/index/arquivo desse worktree devem permanecer idênticos ao snapshot anterior. Namespace por operation é imutável; worktree manual criado apontando um desses commits não tem ref que a automação posteriormente avance. Handoff humano altera seu próprio worktree pelo Git, não por update-ref subterrâneo. O Hugit não promete impedir corridas entre comandos externos concorrentes no mesmo worktree controlado pelo usuário.

## Matriz e oráculos
Pin Git version, ref backend (files/reftable), object format (sha1/sha256), filesystem e falha admitida no qualification lock de A. Não presumir largura de SHA1 em nomes/old-zero. Ensaios do review são históricos; repetir failpoints nas células suportadas. Reftable ou plataforma não ensaiada fica bloqueada para essa capacidade, não marcada tested por compilação. Full testa matriz core e domínios de integração declarados; G5 bloqueia qualquer claim de promoção incompatível.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `47fed390c826e26c6d3201b71d59cb011850b8d355e39bd8de7b38b2998dc197`.

```json
[
  {
    "id": "P10.authority",
    "decision": "single_immutable_managed_ref_certifies_candidate_only",
    "mandatory": true
  },
  {
    "id": "P10.human_ref",
    "decision": "automation_does_not_write_refs_heads_or_user_worktree",
    "mandatory": true
  },
  {
    "id": "P10.handoff",
    "decision": "user_git_action_explicit_no_hidden_index_repair",
    "mandatory": true
  },
  {
    "id": "P10.partial",
    "decision": "legacy_partial_witness_states_are_not_full_commit_proof",
    "mandatory": true
  }
]
```


---

# P11 — Admissão, quotas e orçamentos agregados

**Owner:** B. **Versão:** 3.0. **Achados:** AR-16, AR-02, AR-07, AR-08. **Estado:** especificação proposta, não implementação.

## Objetivo e natureza dos limites
Os valores abaixo são **defaults propostos e alvos de qualificação**, não resultados medidos do Hugit atual. Limites de admissão/buffers devem ser aplicados em código; RSS total é medido na qualificação e não anunciado como limite inviolável de qualquer OS/processo hostil. Workload observado (test/build) não se confunde com recorder: custo Hugit inclui todos os seus processos/workers/codecs/índices, sem excluir quem estourou.

| Recurso | Default proposto | Ação ao atingir |
|---|---:|---|
| Dados Hugit por installation/user | 20 GiB, inclui DB/WAL/tmp/receipts/blobs/cache copiado | Recusar novas reservas; preservar pins e dados admitidos. |
| Envelope máximo por repo | 5 GiB dentro do global; alocação incremental em blocos de 128 MiB | Novo repo/expansão precisa ticket global, sem dedupe global. |
| Controle reservado por repo | 32 MiB já dentro da quota | Health/failure metadata priorizados; não promessa contra disco fisicamente cheio. |
| Transitórios de repo | até 512 MiB, já dentro de seu envelope, salvo export explícito com reserva de destino | Ingest/export sem reserva é refused/partial. |
| SQLite WAL | soft 128 MiB; hard admission 256 MiB + no máximo um lote bounded 16 MiB | Checkpoint/encerrar leitores com deadline; pausar writes novos se não progride. |
| Receipts | 64 KiB cada; 100 mil ou 64 MiB pendentes, o primeiro limite | Recusar nova admissão; fonte recebe ausência de ack. |
| Coletores de stream ativos | 4 por instalação; outros slots admission bounded | Pipe continua drenado em discard/capture_refused, não fila ilimitada. |
| Memória recorder | alvo 128 MiB por collector e 1 GiB agregado com ingress/worker/control | Buffers reservados; qualificação falha se agregado excede. Não contar só heap explícito. |
| Buffer de pump | 1 MiB por stream, dois streams por run | Drenar/descartar com gaps; sem fsync no pump. |
| Métodos de leitura | 8 KiB default, página máx. 64 KiB, request máx. 1 MiB | Paginar/recusar input antes de alocar. |
| SQL read txn | 100 ms por página como budget; operação total 2 s default | Cursor continuation/deadline; export usa snapshot separado. |

## Coordenação global sem serviço obrigatório
Manter budget journal privado da instalação sob arquivo de coordenação estável do SO. Reservar envelope/ticket global ANTES da reserva repo. Não manter lock global durante stream/DB transaction. Commit de arquivos/banco não é distribuído: crash entre ticket e repo pode vazar reserva conservadoramente. Recuperar libera só após cruzar journal, manifest, owner e estado repo; não liberar por relógio. Falha segura é reduzir admissão, não duplicar crédito. Release de espaço global acontece depois de dados realmente removidos e reserva repo conciliada. Sem quota inter-user ou proteção contra outro software encher o disco.

Para memory/process slots, semáforos/tickets têm owner instance e handle; reduzir limites não rouba slot vivo. 32 produtores podem existir como requests, mas não iniciar 32 coletores pesados além dos slots. Budget de metadados e índice entra no mesmo total. Sem recurso para representar novo evento, não emitir durable ack. Se todos os pins excedem quota recém-reduzida, `over_quota`: preservar pins, bloquear novos dados, permitir usuário exportar/desafixar explicitamente.

## Medição de latência e progresso
Medir `t_hook_return`, `t_receipt_ack`, `t_event_visible`, `t_artifacts_ready` individualmente. Meta quente payload≤16KiB: overhead hook p95≤50ms/p99≤200ms; event visibility p95≤1s na célula sem backlog/falha. Para blobs grandes, reportar throughput/latência total e partial readiness, sem prometer ready em 1s. Consulta 100k eventos p95≤250ms, 1M no cenário definido ≤1s; 1GiB de output testa streaming e quota, não residência em memória inteira. CPU idle alvo <1% de um core, sem polling agressivo.

Registrar 1/8/32 producers, cold/warm FS/cache, backlog 10k/100k, leitores/export longo, múltiplos repos e quota reduzida. P50/p95/p99 de samples completos, pelo menos 30 repetições/célula e 1k eventos de timing nas células de hooks. Hardware/FS/software e testes lentos permanecem no pacote. Target não cumprido reabre otimização/claim, não muda de máquina silenciosamente.

## Garantias sob falha
Carga finita saudável progride conforme P03; após crash sem processo vivo só há recovery na próxima interação. ENOSPC externo pode impedir até log de erro; responder ao chamador quando possível sem fingir registro durável. Nenhuma meta de desempenho autoriza falso verde, perda de receipt confirmada ou deleção de pin.

## Cursors e gate de entrega
Não manter WAL reader entre páginas; P08 usa query_generation e erro CURSOR_INVALIDATED. Revocation/publish seguem o gate curto de P06; timeout sem obter exclusão não equivale a ACK de revoke. Incluir delivery journal, snapshots de consulta explícitos e evidências de qualificação no orçamento correto; estes últimos vivem fora de A. Histórias anteriores ao corte não recebem nova identidade de emissão ao coletor reiniciar.

## Vínculo estruturado desta revisão

Versão `3.0`; digest `d0d06231bc06a332774e65589b6e696aed4811e4cb1d610b71f84315409d9156`.

```json
[
  {
    "id": "P11.budget",
    "decision": "global_repo_and_transient_resources_accounted",
    "mandatory": true
  },
  {
    "id": "P11.read",
    "decision": "short_transactions_generation_invalidation",
    "mandatory": true
  }
]
```


---

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

## Vínculo estruturado desta revisão

Versão `3.0`; digest `3f6367a500989e2a102abda97032a8ce25c1bd0edcbe660dd77a96fe4bb37a64`.

```json
[
  {
    "id": "P12.matrix",
    "decision": "all_required_cells_and_assertions_order_independent",
    "mandatory": true
  },
  {
    "id": "P12.identity",
    "decision": "evidence_key_includes_subject_test_assertion_cell_attempt",
    "mandatory": true
  },
  {
    "id": "P12.release",
    "decision": "immutable_A_detached_B_post_publication_C",
    "mandatory": true
  },
  {
    "id": "P12.closure",
    "decision": "HUG056_closed_only_after_post_publication_not_precondition_for_itself",
    "mandatory": true
  }
]
```
