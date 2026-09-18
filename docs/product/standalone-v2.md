# Hugit standalone — contrato de produto v2

Documento de requisitos do HUG-002 (#381), sob o planejamento v3.0.
Não é anúncio de versão publicada nem certificado de que o runtime v2 já existe.
O par `standalone-v2.md` / `support-contract.json` define limites de promessa;
o plano aprovado continua sendo a fonte de dependências, testes e cinco axiomas.

## Produto e problema

Hugit preserva fatos de trabalho, objetivos de fontes qualificadas, execuções,
logs, relatórios e artefatos para que uma investigação possa ser retomada e sua
conclusão demonstrada. A unidade de valor é recuperar a evidência certa, com
origem e limites, sem repetir trabalho só para reconstruir o que ocorreu.
Não é outro provedor de hospedagem, servidor de runners ou substituto do Git.

O core usa arquivos, Git e computação locais. Depois de instalar o binário,
registrar/consultar o corpus básico não exige conta, API key, rede, servidor,
CoreLink ou chamada de modelo. Instalar um binário ou um adapter é uma operação
separada; não se promete que baixar ferramentas seja offline. Um comando de
usuário pode usar rede: isso não transforma rede em requisito do recorder.

## O que automático significa

Após configuração e consentimento iniciais, hooks/adapters fazem a captura
suportada sem o usuário repetir o objetivo num formulário e sem depender de o
LLM lembrar de chamar uma ferramenta. Isso não implica cobertura de todo
processo, transparência perfeita de terminal, zero custo ou ausência de setup.

Goal significa objetivo. Um commit não é um goal. Uma mensagem também não cria
um goal automaticamente: pode conter dois objetivos, uma revisão ou apenas
uma pergunta. A origem qualificada fornece identidade/revisão, ou o registro
permanece `unassigned`. Não usar o objetivo mais recente como fallback.

HUG-057 localiza o produtor, fixa sua versão e observa as amostras reais.
Na base deste contrato, essa descoberta está incompleta: não há produtor de
goal qualificado nem fixtures reais dos três casos. Isto não prova inexistência
do hook e não autoriza inventar seu payload. HUG-015/HUG-022 só podem consumir
os fatos conforme suas próprias dependências. O contrato de produto pode ser
especificado sem fingir que esses fatos já existem.

## Quatro classes distintas

**Observação:** o que um canal efetivamente registrou, com escopo e origem.
**Declaração:** o que foi afirmado pelo caller; preservar não autentica.
**Aprovação:** decisão sobre candidato/assertions, não deduzida de uma receipt.
**Reuso:** resultado anterior cuja compatibilidade foi demonstrada; não execução nova.

Um pre-push demonstra tentativa, não aceite remoto. Uma aprovação de modelo
não certifica ambiente hostil. Receipt publicada não significa que todos os
artefatos foram materializados. `exit 0` não substitui observação de comportamento.

## Matriz de promessas e ausência

Cada linha abaixo corresponde à mesma capability em `support-contract.json`.
O JSON contém fonte, WPs responsáveis, famílias de testes, perfil e testemunho.
As 23 capabilities do plano estão preservadas. Nenhuma é promovida a suporte
v2 demonstrado pela existência destes documentos.

| Capability | Promessa sob condição | Ausência / limite |
|---|---|---|
| `standalone_core` | Registrar e consultar evidência local sem serviço obrigatório. | Informar fonte indisponível ou capacidade ausente; não retornar captura completa. |
| `git_capture` | Preservar os fatos Git observáveis por hooks instalados. | Operação Git não deve ser falsamente confirmada como capturada; expor lacuna/cobertura parcial. |
| `goal_capture` | Preservar objetivo e revisões recebidos automaticamente de fonte qualificada. | Não atribuir ao goal mais recente; manter observação sem atribuição ou lacuna e bloquear qualificação da capacidade. |
| `opencode_adapter` | Receber eventos do OpenCode por adapter autorizado, sem ação voluntária do LLM. | Sem adapter: ausência explícita dessa fonte; core local continua disponível. |
| `claude_adapter` | Receber eventos do Claude Code por adapter autorizado, sem cadastro recorrente. | Ausência/versão incompatível não equivale a captura; não impor esse harness ao core. |
| `noninteractive_run_capture` | Associar comando, saídas e término de execução não interativa observada. | Preservar erro, timeout, truncamento ou captura parcial sem converter ausência em sucesso. |
| `junit_provenance` | Vincular relatório JUnit à execução que o produziu. | Relatório órfão/antigo não certifica a tentativa atual; registrar ausência ou ambiguidade. |
| `rust_pytest_reports` | Ingerir relatórios Rust/Pytest com origem e versão delimitadas. | Sem ferramenta/relatório compatível: capacidade ausente, não falha do core nem aprovação de teste. |
| `playwright_artifacts` | Preservar relatórios, traces e artefatos Playwright associados à tentativa. | Trace ausente não é teste aprovado; lacuna permanece mesmo com outro relatório presente. |
| `build_reports` | Registrar comandos, logs e resultados de build com identidade da execução. | Captura não observada ou output removido deve ser indicado; não reconstruir um build imaginário. |
| `evidence_read` | Consultar evidências e projeções com integridade, política e limites explícitos. | Recusar corrupção/revogação; sinalizar ausente ou CURSOR_INVALIDATED em vez de omitir silenciosamente. |
| `resume` | Retomar investigação a partir da evidência preservada sem rerodar só para descobrir a falha. | Informar lacunas/retenção expirada e limites; não inventar contexto removido. |
| `privacy_revocation` | Aplicar política, acesso e revogação a originais e derivados controlados. | Não expor dados quando autoridade/atualidade exigida não puder ser demonstrada. |
| `migration_v1` | Migrar armazenamento legado com backup e barreira contra escritores antigos. | Recusar cutover com escritor/backup/identidade não qualificados; preservar legado e estado de recuperação. |
| `private_export_restore` | Exportar e restaurar corpus autorizado em corte consistente. | Ausência/corrupção/atualidade de política desconhecida não ganha autorização silenciosa; preservar estado explicável. |
| `native_buildx_records` | Preservar registros nativos do Buildx sem torná-lo dependência do core. | Sem Buildx: integração ausente; nenhum falso sucesso nem instalação automática. |
| `native_buildkit_cache` | Preservar/reusar cache BuildKit e estrutura OCI sob contrato nativo. | Cache faltante/corrompido/incompatível produz miss/recusa explícita; não derruba o core independente. |
| `native_compiler_cache` | Integrar caches de compilação pelos mecanismos nativos qualificados. | Miss ou erro delimitado; executar novamente só pelo comando autorizado, nunca fabricar resultado reutilizado. |
| `qualified_outline_reuse` | Reutilizar outline qualificado para conteúdo/configuração idênticos. | Recusar entrada inválida ou recomputar conforme operação autorizada; não servir outline stale como atual. |
| `local_git_promotion` | Produzir candidato Git verificado em ref gerenciada com handoff explícito. | Ref divergente, evidência ausente ou queda exigem recusa/reconciliação, não merge presumido. |
| `native_platforms` | Anunciar suporte somente nas células nativas qualificadas. | Célula ausente permanece não qualificada; não promover a suporte testado por analogia. |
| `verified_distribution` | Distribuir os mesmos bytes qualificados com evidência e pós-verificação. | Bloquear promoção por obrigação ausente/falha/conflito; não usar último verde para apagar falha. |
| `basic_evidence_read` | Ler a evidência básica autorizada sem depender de todos os viewers/adapters. | Ausência parcial e origem desconhecida são informadas; não inventar campo, teste ou aprovação. |

## Cache e efeitos externos

Capturar logs de um build e preservar um cache são capacidades diferentes.
BuildKit/OCI e caches de compilador usam formatos e chaves nativos qualificados;
não existe promessa de cache universal nem cópia cega de `target/`. Conteúdo
igual pode compartilhar bytes sem compartilhar política ou identidade de execução.
Mudança de toolchain/plataforma/configuração exige miss ou recusa apropriada.

HUG-041 continua obrigatório no perfil completo. Docker/Buildx/compiladores
podem ser opcionais na instalação de um usuário, mas a integração anunciada
precisa ter sido qualificada nas células exigidas. Ausência do executável
opcional não derruba o core nem gera um teste verde fictício.

Export/restore preserva o corpus autorizado, não o mundo externo. Não desfaz
e-mails enviados, pushes aceitos, ações de serviços ou bytes já entregues.
Backup antigo sem autoridade mais recente não revela revogações posteriores.
Não há promessa de undo universal ou `main` que nunca quebra.

## Confiança local e privacidade

O domínio é cooperativo, com confiança na conta/host local. Um processo hostil
com o mesmo acesso de escrita pode modificar os mesmos arquivos; permissões
user-only e hash chain não criam isolamento independente. Origem declarada não
é autenticação. Pelo P05, autoridade vem do canal configurado, não do payload.
Root, export, execução e acesso à rede não podem ser ampliados pelo texto de
um relatório. Ler evidência não executa seus comandos nem abre URLs/viewers
sem ação explícita autorizada. Tampouco se promete resistência universal de
qualquer LLM a prompt injection.

No runtime alvo, política/revogação e entregas concorrentes seguem os protocolos
aprovados. Não atribuir essas garantias a todos os comandos legados atuais:
o inventário documenta consultas com efeitos de migração/status. Isso precisa
ser reconciliado pelos WPs responsáveis antes de anunciar conformidade v2.

## Jornada core sem chaves nem rede

Em repositório descartável e configuração/home isolados: observar `health`
inativo; executar `attach`; fazer um `git commit` comum; observar o `ref.update`
com o OID exato; consultar estado e confirmar ausência de goal inventado e de
arquivos de runtime rastreados. Não chamar `capture` manualmente para simular
um hook automático. Aguardar a drenagem deve ter limite e registrar timeout.

A evidência exige binário identificado, negação de rede verificada por controle,
comandos/saídas e o evento retido. Uma fixture construída pelo teste é um ensaio
real com dados sintéticos, não sessão real de goal do usuário. Resultado de
binário arquivado não é build atual nem qualificação de todas as plataformas.
A jornada ampliada goal → falha → evidência → retomada → restore é a obrigação
de HUG-060/HUG-051, não algo que esta jornada básica alegue ter entregue.

## Escopo encerrado e precedência

Não reabrir CoreLink, remote AC, provisioning/workspaces remotos, orquestração
de runners, Clerk, tenancy, forge/hosting, mirror deployment ou atestação de
runner externo. Execução local autorizada e cache nativo opcional permanecem
no escopo; são coisas diferentes desses serviços encerrados.

`docs/product/product.md` e `CLAUDE.md` conservam narrativas históricas com
promessas incompatíveis (plataforma distribuída, undo universal e outras).
O cabeçalho de encerramento e o plano atual prevalecem; o histórico não vira
roadmap obrigatório. HUG-054 possui os write-sets para reconciliar esses pontos
de entrada. Estes dois arquivos não significam que README/CLAUDE já foram editados.

## Aceitação, versões e recuperação

O perfil `early-slice` é interno, não release completo. `full` exige os 60 WPs
aprovados e suas 23 capabilities. Um verde Linux não substitui uma célula
Windows/macOS nem um target cross-compilado prova execução nativa.

O testemunho liga candidato, assertion, célula, ambiente qualificado e tentativa
a expected/observed/artifact_refs/outcome/verifier_identity. Conflitos e faltas
bloqueiam; não há agregação last-wins. A é o sujeito imutável, B reúne evidências
e revisão fora dele, C registra publicação/pós-verificação dos mesmos bytes.
Revisão desta entrega é responsabilidade do executor por orientação do
proprietário; não é apresentada como revisão independente.

Reverter este contrato afeta somente a versão da especificação: sem alterar
runtime/dados, bloquear novos dispatches até reconciliar os claims. Se a versão
anterior não existe (primeira instalação), ausência bloqueia em vez de inventar
uma versão aprovada. Trabalho já admitido conserva sua referência; não é morto
ou reconfigurado silenciosamente. O ensaio desta regra não implementa o gate
futuro de dispatch, que pertence aos seus WPs.

## Cinco axiomas do HUG-002

Os IDs, afirmações e vínculos completos estão em `support-contract.json.axioms`;
nenhum grupo é opcional. Fechamento depende de evidência e integração, não de
contar documentos ou testes do checker.

### Completeness criteria

**HUG-002-CC01** — Escopo ativo e encerrado têm fronteira explícita.

**HUG-002-CC02** — Localização do hook e payload são entrada de HUG-057, nunca formulário imposto.

**HUG-002-CC03** — Produto não anuncia cache universal nem restauro de todo efeito externo.

### Success criteria

**HUG-002-SC01** — Matriz relaciona cada promessa a perfil, fonte, teste e comportamento de ausência; existe jornada core sem chaves nem rede.

**HUG-002-SC02** — Inserir dependência obrigatória de serviço ou chamar ausência de captura de sucesso falha no contrato e bloqueia perfil.

### Quality standards

**HUG-002-QS01** — Nenhum claim sem condição de validade e testemunho definido.

**HUG-002-QS02** — Redação diferencia dado observado, aprovação e reuso.

**HUG-002-QS03** — Manifestos/versionamento e leitura offline; sem efeito no runtime; documentos sem placeholder obrigatório. Validação linear no número de registros e saída de erro limitada.

### Definition of Done

**HUG-002-DOD01** — Fechar HUG-002 somente quando a implementação/artefato de 'Contrato standalone e fronteira pública de produto' satisfaz suas assertions e observáveis obrigatórios; uma suite verde sem resultado individual não fecha os axiomas.

**HUG-002-DOD02** — Todos os outputs docs/product/standalone-v2.md, docs/product/support-contract.json entregues nos write sets resolvidos; erros de P00, P05 e mudanças de interface revisados pelo owner do contrato.

**HUG-002-DOD03** — Recovery deste pacote ensaiado: Reverter somente a versão da especificação e bloquear novos dispatches até reconciliação dos claims; dados e código não mudam neste pacote.

**HUG-002-DOD04** — Qualificar HUG-002 sobre o sujeito A identificado por digest; anexar em B assertions verificadas e aprovação de revisor diferente do autor. Resultados não alteram A; revalidar no candidato final as obrigações atingidas. Nenhum axioma obrigatório dispensado.

### Invariants

**HUG-002-INV01** — CoreLink e serviços externos não são dependências do core.

**HUG-002-INV02** — Registrar não depende de decisão voluntária do LLM.

**HUG-002-INV03** — Confiança declarada não equivale a autenticação.

## Fontes fixadas

Base de inspeção: `10a3c8b3cdd780554d78db750975f8e5a2b253b0`. Requisitos: `docs/plan/standalone/v3/backlog.json`
(SHA-256 `5234276523649da9b35abe578ed6cbf2ce25c43651bfc0f6ab2d549b37e01409`), capabilities, perfis e P00/P05 da mesma versão.
Observações de fonte: README, `docs/quickstart-hooks.md`, capture/intent e os
mapas de `docs/audit/`. Descoberta de goal: os três arquivos de
`docs/contracts/goal-source-*` integrados pelo PR #451. Não há dado de sessão
privada nem segredo necessário para ler este contrato offline.
