# Resposta aos dez achados — v3

Status: addressed_in_plan; implementação do Hugit e revisão independente pendentes. Nenhum caso anterior foi apresentado como novo teste de runtime.

## BR-01 — O validador aceita violações dos contratos que deveria coordenar

**Decisão:** Fonte normativa única, digest/versão por protocolo, freeze para todos os consumidores e vistas comparadas integralmente.

**Protocolos:** P00. **WPs:** HUG-007, HUG-059, HUG-006, HUG-056.

**Aceitação original preservada:** Cada mutação de `scripts/audit_validator.py` tem resultado esperado explícito. A versão corrigida deve recusar remoção do freeze de qualquer consumidor que o exige; protocolo ausente; consumer ligado à versão errada; owner divergente; baseline inválido; capability obrigatória omitida. Artefato compartilhado só passa com um agregador definido e verificável. Controles positivos continuam passando. A aprovação desse gate não se converte em aprovação de implementação.

**Família de fechamento:** `T-BR-01-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `3f016d3d2bf55eba9fa95c1b2907c0902dc28f73e80a03daa69af4bab2c7e073`. A seção integral está no JSON e no arquivo original preservado.

## BR-02 — Os cinco axiomas existem, mas a rastreabilidade ainda é por etiqueta de teste

**Decisão:** Assertions por critério; ledger create-goal ligado à assertion P correta; dedupe entre duas políticas tem predicado próprio.

**Protocolos:** P00. **WPs:** HUG-007, HUG-017, HUG-036, HUG-055.

**Aceitação original preservada:** Para INV01 do ledger, executar create-goal sem implementação/testes e provar `done=0`. Para INV03 de blobs, usar os mesmos bytes em duas invocações e duas políticas distintas, verificando dedupe físico sem fusão de ocorrências ou permissões. Mutar somente a assertion específica deve invalidar o critério correspondente, mesmo quando N/R e o restante da suíte passam. Demonstrar essas relações em JSON e na vista humana.

**Família de fechamento:** `T-BR-02-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `7e6c4be4a37c606104ece3a6eb0d7ee54667bf0231d79a14e14cb64f98d5633f`. A seção integral está no JSON e no arquivo original preservado.

## BR-03 — O witness Git depende de uma fronteira de commit que o modelo abstraiu

**Decisão:** Uma ref imutável de manifesto prova publicação de candidato gerenciado, não uma transação em branch humana; manter negativos de states parciais.

**Protocolos:** P10. **WPs:** HUG-043, HUG-045, HUG-047, HUG-059.

**Aceitação original preservada:** Substituir o passo único do modelo por estados de aquisição, persistência do destino, persistência do witness e confirmação. O modelo deve incluir os estados nativos observados. Rodar fault injection por fronteira no backend admitido; ordem não admitida deve ser rejeitada ou continuar ambígua. Uma observação parcial nunca produz RECORDED com autoridade maior que a sustentada. Preservar avanço posterior de terceiros e distinguir queda de processo de falha de máquina.

**Família de fechamento:** `T-BR-03-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `65a35e71a14e8673bbcd20c2952839f361dd12268238ce8b87469bf48d6e3123`. A seção integral está no JSON e no arquivo original preservado.

## BR-04 — Verificar branch livre e depois promovê-la admite checkout concorrente

**Decisão:** Automação só publica refs gerenciadas imutáveis; handoff humano explícito via Git, sem update-ref em branch livre presumida.

**Protocolos:** P10. **WPs:** HUG-028, HUG-043, HUG-045, HUG-047.

**Aceitação original preservada:** Forçar checkout/worktree-add entre a checagem e a promoção, também antes/depois de prepare. Ou a operação permanece no namespace gerenciado sem afetar o checkout alheio, ou a concorrência é excluída/recusada pelo contrato demonstrado. Nenhum worktree do usuário recebe HEAD novo com estado físico antigo como efeito oculto da automação. Rodar com comandos Git reais, não um mock que respeita artificialmente o lock do Hugit.

**Família de fechamento:** `T-BR-04-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `0ed0d9b09566a8765c4e557ea253761b4c87c6fcb762d7c790b4c93c99c428ee`. A seção integral está no JSON e no arquivo original preservado.

## BR-05 — Revalidar policy epoch antes de publicar ainda deixa uma janela de autorização

**Decisão:** Gate de entrega compartilhado com revoke; ACK impede novos grants antigos, in-flight já enfileirado identificado.

**Protocolos:** P06. **WPs:** HUG-020, HUG-037, HUG-039, HUG-045.

**Aceitação original preservada:** Pausar o exporter depois da decisão de acesso e antes da publicação; confirmar revoke; retomar. Não pode surgir uma entrega nova proibida segundo a semântica do ACK. Testar também queda durante o gate, revogação durante export longo e página parcialmente encaminhada. O evento registra quando autorização/entrega foi concedida, sem afirmar apagamento retroativo de bytes já fornecidos.

**Família de fechamento:** `T-BR-05-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `977a0c07d7e0b4f8487f197f475b4d1ec0ce853ed0e879fb40440a7cf1411c57`. A seção integral está no JSON e no arquivo original preservado.

## BR-06 — Um restore offline antigo não conhece revogações que não recebeu

**Decisão:** Cold restore sem âncora é freshness unknown com privado bloqueado até nova autorização local; import preserva autoridade atual conhecida.

**Protocolos:** P06. **WPs:** HUG-018, HUG-020, HUG-039, HUG-058.

**Aceitação original preservada:** Restaurar backup antigo em ambiente que conhece revoke posterior: a revogação prevalece. Repetir numa máquina vazia sem a âncora: estado de freshness desconhecido, nenhum acesso privado implicitamente reautorizado. Com recuperação autorizada, registrar a nova decisão e não afirmar conhecimento retrospectivo da revogação perdida. Nenhuma dependência cloud é introduzida para completar o teste.

**Família de fechamento:** `T-BR-06-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `9843a155278e211afe416d31116847dd3bd5dff0e7092c634b6c405c860550d8`. A seção integral está no JSON e no arquivo original preservado.

## BR-07 — Watermark no cursor não preserva sozinho uma projeção que foi sobrescrita

**Decisão:** Paginação usa generation e invalidação explícita, não falsa reconstrução as-of de projeção sobrescrita.

**Protocolos:** P08. **WPs:** HUG-016, HUG-034, HUG-035, HUG-038.

**Aceitação original preservada:** Página 1, mudança de estado/deleção/novos resultados, página 2: devolver exatamente a visão contratada ou CURSOR_INVALIDATED/erro explícito, sem omissão ou duplicação silenciosa. Repetir com FTS atrasado, empate de ordenação, redaction/policy mudança e compactação de versões. O teste compara IDs e conteúdo esperado, não apenas status HTTP/JSON.

**Família de fechamento:** `T-BR-07-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `831064e7b21d5c86ad9211a17154a4bd07234018172868948e85d2e603a51d40`. A seção integral está no JSON e no arquivo original preservado.

## BR-08 — Instância efêmera de coletor não pode ser o namespace de retransmissão histórica

**Decisão:** Separar namespace estável da emissão de collector/delivery efêmeros; crash de watermark reentrega, não duplica.

**Protocolos:** P04. **WPs:** HUG-011, HUG-019, HUG-023, HUG-024, HUG-027, HUG-028.

**Aceitação original preservada:** Processar um evento, cair depois do commit e antes de avançar watermark, reiniciar adapter e reler: uma ocorrência lógica, duas entregas rastreáveis. Executar de fato o mesmo comando novamente: duas invocações. Testar rotação/reset de contador, dois produtores concorrentes e import duplicado. Fixtures devem vir do adapter real ou de seu contrato explicitamente congelado.

**Família de fechamento:** `T-BR-08-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `610273388aacf53c258df831ba832f503e907729f07dd87ca4e58a109041cbcd`. A seção integral está no JSON e no arquivo original preservado.

## BR-09 — A evidência por test_id pode apagar uma falha de plataforma pela ordem de chegada

**Decisão:** Obrigações por sujeito/assertion/célula/lock/attempt; agregação order-independent e conflitos bloqueantes.

**Protocolos:** P12. **WPs:** HUG-005, HUG-007, HUG-049, HUG-052, HUG-055, HUG-056.

**Aceitação original preservada:** O fixture Windows failed/Linux passed resulta na mesma pendência nas duas ordens. Remover uma célula requerida, trocar target ou candidate, omitir ambiente ou duplicar resultado conflitante bloqueia qualificação. Com todas as instâncias reais verificadas e aprovadas, o controle positivo produz readiness qualificada no pipeline próprio; o preflight plan-only continua não autorizando por conta própria.

**Família de fechamento:** `T-BR-09-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `562c4a3e54cc8247aba04615c30be9b25927770bd1739b655ad150672ecc3240`. A seção integral está no JSON e no arquivo original preservado.

## BR-10 — Candidato imutável, evidência e publicação ainda formam uma referência circular

**Decisão:** A imutável, B evidência destacada, C promoção/post-verificação; 056 fecha depois, não antes, da própria publicação.

**Protocolos:** P12. **WPs:** HUG-052, HUG-054, HUG-055, HUG-056.

**Aceitação original preservada:** Demonstrar um percurso finito com candidate A, evidence B e promotion C; nenhum objeto precisa conter seu próprio hash final. O teste positivo chega a pré-promoção, publica somente A e fecha após pós-verificação. Alterar A invalida B. Alterar apenas anotações de C não muda os bytes A. Falha pós-publicação não produz closed nem autoriza substituição silenciosa de asset sob a mesma identidade.

**Família de fechamento:** `T-BR-10-C`. Resultados reais por assertions/células, sem waiver.

**Fingerprint da seção original:** `5b9eaf48e817e3a4352473d12e28d68dc8c66560cca3e0393d4f905c61f3274a`. A seção integral está no JSON e no arquivo original preservado.
