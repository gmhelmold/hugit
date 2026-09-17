# HUG-001 — entrega para aceitação por critério

Não é um novo plano, uma exceção aos cinco axiomas ou aprovação independente.
O inventário descreve o sujeito `96ddcdade3d56d7c2cf4a01c294a56c3aaa4e58c`.
O candidato dos verificadores/documentação e suas evidências têm identidade
separada, registrada no PR/CI. Nenhum resultado de aceitação é inferido deste
arquivo existir ou ser integrado. O contrato permanece em
`docs/plan/standalone/v3/work-packages/HUG-001.md`.

## O que SC01 verifica, e o que não pode ser inferido

São dois vínculos diferentes:

1. O **teste do inventário** verifica identidade, inclusão, classificação e
   associação da superfície. Os verificadores não executam os arquivos inspecionados.
2. O **teste do comportamento** chama código, subprocesso ou produto. Seu alcance
   depende da assertion real. Mocks, fixtures, oráculos legados e nomes de testes
   não demonstram uma jornada real por si sós.

As 57 entradas e 24 variantes têm handlers e testes de fonte em
`reachability.json`. `source_tests[].supports/does_not_support`, os helpers e
`remaining_coverage` continuam obrigatórios. Não elevar `ST-033` (fixture apenas)
a prova de comando; não usar o oráculo legado do ledger para aceitar o v3.
Aliases ligados a `record` mantêm explícita a falta de prova do wiring CLI.

A tabela abaixo resolve a lacuna de **localização dos testes** das automações
por partição dos 28 perfis de `automation-contracts.json`; cada uma das 81
entradas pertence exatamente a um perfil. `A.*` significa método de
`scripts/plan/test_automation.py:AutomationTests`. Para todos os perfis, A.test_positive
confere os registros selecionados, e os casos de omissão/corrupção recusam dados
inconsistentes. Isso não é cobertura de execução de cada script.

| Perfil | Teste de inventário / referência de comportamento | Limite obrigatório |
|---|---|---|
| cargo_config | A.test_positive; arquivo `.cargo/config.toml` | Configuração jobs=4; não é execução de Cargo. |
| agent_config | A.test_positive; `.claude/hooks/forbid-sibling-paths.py:_selftest` | Registro de guard, não produtor de goal. |
| agent_guard | A.test_positive; `.claude/hooks/forbid-sibling-paths.py:_selftest` | Selftest localizado; não executado nesta inspeção. |
| changelog_gate | A.test_positive; `.github/workflows/ci.yml` passo changelog gate | Consumidor localizado; não há teste unitário dedicado selecionado para o shell. |
| ci | A.test_positive; `.github/workflows/ci.yml` jobs gates/windows-check | Comandos de teste declarados não provam execução nesta rodada. |
| dco | A.test_positive; `.github/workflows/dco.yml` | Trailer não autentica autor; sem oráculo de identidade humana. |
| evidence_ci | A.test_positive; `scripts/test-evidence-report.sh`, `scripts/test-verify-evidence-report.sh` | Fresh-host verifica pacote retido; não reexecuta todo o produto. |
| release_ci | A.test_release_write_not_erased; `.github/workflows/release.yml` | Matriz/saída por tag; nenhum release disparado pela inspeção. |
| plan_ci | A.test_positive; `scripts/plan/test_sync.py:ImportTests` | Workflow histórico testa plano/importador, não o inventário posterior. |
| plan_models | A.test_positive; `docs/plan/standalone/v3/scripts/check_protocol_models.py` | Modelos limitados, não implementação do runtime. |
| plan_probe | A.test_positive; `docs/plan/standalone/v3/scripts/probe_git_managed.py` | Sonda de referência; não equivale a testes do Hugit. |
| plan_qualification | A.test_positive; `docs/plan/standalone/v3/scripts/test_qualification.py` | A/B/C de demonstração, não autorização de release. |
| plan_generate | A.test_positive; `docs/plan/standalone/v3/scripts/test_plan_validator.py` | Comparação de vistas, não aval de suas afirmações. |
| plan_validate | A.test_positive; `docs/plan/standalone/v3/scripts/test_plan_validator.py` | Validação do plano, não execução dos WPs. |
| plan_tests | A.test_positive; os próprios `test_plan_validator.py` / `test_qualification.py` | São harnesses; não renomear a existência deles como aprovação. |
| import_check | A.test_positive; `scripts/plan/check_import.py`, `scripts/plan/test_sync.py` | Checagem de importação e controles do importador, sem API. |
| import_restore | A.test_positive; `scripts/plan/check_import.py` | Reconstrução/identidade consumida pelo checker; sem teste isolado adicional alegado. |
| import_sync | A.test_sync_write_not_erased; `scripts/plan/test_sync.py:ImportTests` | Testes offline não qualificam todos os writes de --apply. |
| import_tests | A.test_positive; `scripts/plan/test_sync.py:ImportTests` | Harness de dados, sem prova de API real. |
| evidence_wrapper | A.test_positive; `scripts/test-evidence-report.sh` | Wrapper delega ao produtor; nome benchmark não prova medição. |
| evidence_producer | A.test_output_replace_not_erased; `scripts/test-evidence-report.sh` | Fixtures e substituição de saída delimitadas; não dados arbitrários de usuários. |
| evidence_verifier | A.test_positive; `scripts/test-verify-evidence-report.sh` | Prova sobre observáveis retidos, não nova execução do binário. |
| evidence_tests | A.test_positive; os dois scripts de teste de evidências | Harness executável, distinguido dos scripts que testa. |
| local_journey | A.test_positive; `scripts/validate-go-live.sh` | É a jornada legada; simulação não vira aceite v3. |
| acceptance_helper | A.test_positive; `tests/acceptance/lib.sh` e seus wrappers `run.sh` | Library shell chama argv do caller; não é pura nem teste independente de si. |
| legacy_wrapper | A.test_false_external_classification; o `tests/acceptance/*/run.sh` da própria entrada | Driver de testes histórico; targets Rust ausentes/obsoletos não são prova atual. |
| legacy_external | A.test_external_wrapper_downgraded; o `run.sh` da própria entrada | Não executar/reabilitar runner/CoreLink para satisfazer o standalone. |
| legacy_document | A.test_positive; `tests/acceptance/wp-c1/run.sh` | Grep documental, não execução de runner. |

Os destinos/owners vêm do plano fixado, não são inferidos de números de issue.
Perfis de automação levam a HUG-001/002/003/004/028/044/051/053/055/059, conforme
o registro. Destino existente é uma propriedade estrutural: a revisão ainda
precisa confirmar que ele é semanticamente apropriado. Não adicionar um WP
fictício para preencher a coluna.

## Gerador de fixtures: vínculo concreto sem elevar autoridade

Superfície `crates/hugit-contracts/src/bin/gen_fixtures.rs`; destino HUG-015,
owner A. O programa escreve schemas e goldens via `CARGO_MANIFEST_DIR`.
Referências verificáveis:

- `crates/hugit-contracts/tests/integration_tests.rs:read_golden/roundtrip`
  (linhas 43–66): leitura falha explicitamente, desserializa e exige igualdade de
  bytes reserializados. Casos `golden_*` cobrem os objetos e altitudes listados.
- `integration_tests.rs:schema_drift` (linhas 439–460): compara schemas dos tipos
  com os arquivos. `UPDATE_SCHEMAS=1` muda o modo para atualização; não usar esse
  modo como prova de ausência de drift.
- `crates/hugit-contracts/tests/golden_pins.rs`: pins/tripwires do serializer.
- `tests/acceptance/wp-00/run.sh`: chama testes do crate e procura arquivos/casos.

Esses testes protegem contratos/bytes dos outputs. Não demonstram por si só a
invocação de `gen_fixtures`, seus diretórios de escrita ou recuperação após falha.
A ausência dessa prova permanece no destino de implementação; não afirmar que o
gerador foi executado nesta qualificação do inventário.

## Roteiro da revisão dos 15 critérios

A tabela é um índice de evidência e decisões exigidas, NÃO checklist aprovado.
O resultado por assertion deve registrar expected, observed, artifact_refs,
outcome e verifier_identity sobre o candidato identificado, fora dele.

| Critério | Observável a inspecionar / decisão necessária |
|---|---|
| CC01 | `source-inventory.json`, relatório acquisition e registros de leitura parcial do catálogo. Binários/gerados não recebem leitura semântica inventada. |
| CC02 | `surface-contracts.json`, `automation-contracts.json`, variantes e limites desta tabela contra a fonte. Não apagar simulação/legado. |
| CC03 | `path-bindings.json` e testes de existência/operação/owner no verificador de aquisição. |
| SC01 | Igualdade Git/manifesto; cada entrada resolve handler/destino/owner e referência de teste com seu alcance explícito. Reprovar qualquer vínculo que alegue comportamento não demonstrado. |
| SC02 | Testes de pacote/aresta transitiva, falta de fonte, arquivo extra e identidade na suite de recuperação/acquisição. |
| QS01 | Enumerações Git de tree e índice temporário concordam byte a byte. |
| QS02 | Camada mecânica não aumenta contagem de revisão semântica; limites das fontes inspecionadas continuam visíveis. |
| QS03 | `QUALIFICATION-BOUNDS.md`, código dos verificadores e resultados dos controles. Não substituir análise por amostra de tempo/RSS. |
| DOD01 | Decisão derivada das demais assertions, sem fechar por CI verde ou número de PRs. |
| DOD02 | Três outputs originais integrados e escopos auxiliares identificados nos PRs 441–447; revisão de interfaces. |
| DOD03 | `RECOVERY.md`, manifests reconstruídos e relatório/state check de recuperação. Não confundir dois checks com reconstrução. |
| DOD04 | Parecer efetivamente recebido de revisor distinto, sobre o sujeito/candidato identificado. Ausência, COMMENT do autor ou cota esgotada não satisfazem. |
| INV01 | Arquivos/testes não lidos/executados não são relabelados como lidos/executados. |
| INV02 | Adição/omissão altera o conjunto; fonte histórica não é silenciosamente atualizada para incluir arquivos novos. |
| INV03 | Inspeção somente leitura do produto; outputs temporários separados, fonte quiescente, CI de produto distinguida da ferramenta. |

## Critério de parada operacional

Esta é uma entrega para revisão de suficiência, não autorização para expandir
indefinidamente o inventário. Não corrigir todos os defeitos do runtime dentro de
HUG-001; registrá-los no destino apropriado. Se uma referência for insuficiente,
o reviewer deve apontar a assertion e o contraexemplo, em vez de exigir uma
nova auditoria irrestrita. Isso não permite dispensar nenhuma obrigação.

A recusa de quota recebida da integração Codex continua sendo bloqueio de
revisão, não aprovação. Não gerar identidade fictícia, alterar billing nem
substituir revisão distinta por esta autorrevisão. Sem qualificação e parecer,
HUG-001 permanece aberto e HUG-057 não é admitido automaticamente.
