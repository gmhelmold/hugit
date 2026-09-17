# Hugit — execução standalone v3

Esta pasta importa o planejamento aprovado na conversa do proprietário, sem alterar código do produto. A especificação normativa está em [`v3/backlog.json`](v3/backlog.json); os 60 WPs mantêm os cinco axiomas obrigatórios, assertions e recuperação específica.

## Começar

Leia [`v3/PLANO.md`](v3/PLANO.md), [`v3/EXECUCAO.md`](v3/EXECUCAO.md) e os [work packages](v3/work-packages/). As issues são projeções para coordenar execução, não outra fonte normativa.

Todos os WPs permanecem backlog. Revisão/merge deste PR aprova a importação, **não** fecha HUG-001 ou qualquer outro WP. Nenhum produto foi qualificado pela importação. Mudanças em contratos passam por PR e regeneração; comentários de issue não alteram os critérios. PRs intermediários usam `Refs`, sem fechamento automático. Fechar WP somente depois de evidência, revisão e qualificação no candidato correto.

## Validação desta importação

Na raiz do repositório:

```sh
python3 scripts/plan/check_import.py
python3 scripts/plan/test_sync.py
python3 scripts/plan/sync_github.py --spec-commit <SHA-completo-da-especificacao> --dry-run-dir /tmp/hugit-issue-preview
```

O primeiro comando valida a fonte normativa e todas as vistas geradas, além da identidade do JSON e scripts importados. Não declara testes do produto executados. O verificador original de **arquivo completo** em `v3/scripts/validate_plan.py` também exige os ZIPs históricos: esses archives não foram vendorizados no Git. Sua ausência não é ocultada por um relatório de arquivo completo aprovado. Os digests e origem histórica estão em [`IMPORT-MANIFEST.json`](IMPORT-MANIFEST.json); o pacote v3 original permanece na Library/conversa do proprietário.

## Importação idempotente de issues

`scripts/plan/sync_github.py` é dry-run por padrão; `--apply` exige autenticação local via GH_TOKEN/GITHUB_TOKEN. Não incluir tokens em issues/arquivos/chat. A execução escreve somente neste repositório: cria issues ausentes, preserva texto fora do bloco gerado, recusa identidade duplicada e adição a outro parent. Não fecha/reabre issues, não faz merge e não inicia implementação.

O mapa operacional fica em `github/issue-map.json` quando a importação termina. Cada issue inclui seus cinco axiomas e link fixado no commit da especificação. Dependências nativas usam redução transitiva verificada: 121 arestas preservam o alcance das 295 do DAG completo, que não foi alterado.

## Project

A conexão desta sessão não expõe Projects e o token de repositório do GitHub Actions não acessa Projects pessoais. O painel permanece pendente, explicitamente. Não foi substituído por alegação de painel criado. Configuração desejada: **Hugit — Standalone Delivery**, campos WP ID, Status, Fase, Lane, Prioridade e responsável; vistas Prontos, Em execução por lane e Qualificação/publicação.

**Limites:** Hugit continua standalone; intent = goal capturado por hook; nenhum CoreLink, conta, cloud, runner remoto ou Git AI obrigatório. A importação não é auditoria nova nem aprovação independente.
