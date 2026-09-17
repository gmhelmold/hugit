# Importação standalone v3 — verificação e passagem para revisão

Data: 17 de setembro de 2026. Este documento registra a importação do planejamento, não a execução dos work packages.

## Entradas e objetos publicados

- PR de importação: https://github.com/gmhelmold/hugit/pull/400
- Issue-mãe: https://github.com/gmhelmold/hugit/issues/379
- Mapa operacional completo: [issue-map.json](issue-map.json).
- Fonte normativa: [../v3/backlog.json](../v3/backlog.json).
- Especificação fixada nas issues: commit `0b7f629dd0259515608517bcd692e5ec4d2b6ee3`.
- Commit com mapa operacional verificado: `4075e91c6cf102356ed7be44794681bd110ec19b`.
- Workflow de importação concluído: https://github.com/gmhelmold/hugit/actions/runs/35208382273

## Verificações observadas

O workflow concluiu recuperação da fonte aprovada, validação das vistas, criação/reconciliação das issues, leitura das relações nativas e gravação do mapa operacional. Seu artifact `hugit-plan-import-report` registra `import_complete=true` e `relations_verified=true`.

A conferência adicional do artifact contra o ZIP original aprovado confirmou:

| Verificação | Resultado |
|---|---|
| WPs presentes no mapa | 60/60 |
| Números de issues distintos | 60/60 |
| Critérios obrigatórios no plano | 900; cinco axiomas em cada WP |
| Fonte normativa SHA-256 | `5234276523649da9b35abe578ed6cbf2ce25c43651bfc0f6ab2d549b37e01409` |
| Blob Git da fonte aprovado e importado | `b1c636ed0c6b6349431b963ca70a875034436cb8` |
| Dependências completas no plano | 295 |
| Dependências nativas reduzidas | 121 |
| Alcance transitivo original versus relações reduzidas | Idêntico para os 60 WPs |
| Implementações ou WPs fechados por esta importação | Nenhum |

As 121 relações são uma redução transitiva, não uma retirada de requisitos. O DAG completo permanece no JSON e no corpo das issues. O importador verifica parentesco e bloqueios lendo as APIs nativas depois das gravações. A inspeção adicional de uma issue de armazenamento confirmou os cinco axiomas, assertions, dependências e recuperação no corpo publicado.

## Uso após aprovação do PR

Começar por [HUG-001 / #380](https://github.com/gmhelmold/hugit/issues/380), sujeito a sua admissão. HUG-057 / #437 qualifica o hook de goal após o inventário. Todos os WPs permanecem backlog; encerramento requer os critérios individuais e evidência, não apenas merge de um PR de código.

Não usar a issue-mãe como prova de prontidão de release. Não fechar issues por sincronização de texto. PRs intermediários de implementação usam `Refs`.

## O que não foi realizado

- Nenhuma alteração no Rust ou implementação de feature.
- Nenhum merge em `main` ou publicação de release.
- Nenhuma qualificação de runtime ou revisão independente produzida pela importação.
- GitHub Project não foi criado: a conexão disponível não expõe essa operação. A issue-mãe, as sub-issues, labels e o mapa operacional já permitem acompanhamento; não são apresentados como um Project nativo.

A verificação do workflow de importação é separada dos checks do PR final. O resultado atualizado desses checks deve ser consultado no PR, nunca inferido do sucesso do importador.

## Preservação e manutenção

Os commits originais da importação permanecem alcançáveis na branch `archive/standalone-v3-import-before-signoff`, para preservar os links imutáveis usados nas issues. A branch de revisão foi consolidada com sign-off, sem alterar os bytes normativos aprovados. A branch temporária de transporte não é parte do PR.

As vistas continuam derivadas do backlog. Alterações de contrato exigem PR e regeneração. Comentários de execução não substituem a especificação.
