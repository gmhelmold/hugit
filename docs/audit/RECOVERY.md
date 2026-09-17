# HUG-001: recuperação dos derivados integrados

Este procedimento atende ao ensaio técnico de DOD03 e acrescenta controles de SC02.
Não altera os 15 critérios do WP, não concede aprovação independente e não libera dependentes.

## Entradas que precisam sobreviver

- Fonte: `96ddcdade3d56d7c2cf4a01c294a56c3aaa4e58c`, tree `0e74dd8f693b0d660c3ce87def679e8a23b1d1dc`.
- Enumerações `git ls-tree -r -z` e índice temporário `git ls-files --stage -z` desse sujeito.
- Observações Cargo/basis: `5dcd4355c8ed896e168f7181e1d5add806ef6aaf:docs/audit/reachability.json`.
- Curadoria: `d6f311e2a3521013f0f2cc2477fa716f7ca0f925:docs/audit/reachability.json`.
- O plano e Cargo.lock são lidos da própria fonte fixada.

Git OID e SHA-256 das entradas retidas são exigidos pelo reconstrutor. Clone sem esse histórico é erro de preparação: não há fallback de rede, conjunto vazio ou inferência de conteúdo perdido.

As descrições, escolhas de testes, locators e limitações são trabalho de curadoria. Não podem ser reinventadas a partir de código. A curadoria usa uma lista explícita de campos; seus contadores e flags de aprovação são ignorados. A base Cargo é observação histórica preservada, não uma nova execução de cargo metadata nem auditoria das bibliotecas externas.

## O que é reconstruído

`recover_inventory.py` não recebe os JSONs finais de source-inventory ou path-bindings.
Ele reenumera os arquivos presentes, confere os bytes contra os objetos Git, recalcula SHA-256, classes, responsáveis e registros de leitura mecânica. Reconstrói os 181 bindings do plano e calcula conflitos/compartilhamentos novamente.

A visão composta preserva apenas as seleções e afirmações da curadoria fixada, recompõe metadados/contagens e é serializada novamente no formato publicado. O catálogo de dependências é confrontado com Cargo.lock, os perfis default/all-features e a consistência das arestas. Não é uma cópia opaca dos três arquivos nem uma segunda revisão semântica.

## Ensaio na CI

1. Extrair a fonte histórica e resolver os objetos retidos usando Git, sem executar essa fonte.
2. Criar derivados descartáveis; removê-los exclusivamente num diretório temporário próprio.
3. Reconstruir duas vezes com as mesmas entradas, sem ler os derivados descartados.
4. Exigir igualdade byte a byte com os três outputs integrados e entre reconstruções.
5. Reutilizar o verificador de superfícies sobre os dados reconstruídos, inclusive a identidade do catálogo.
6. Demonstrar recusa de dependência transitiva não classificada, dependência omitida, lock divergente, perfil ausente, falha de preparação e entrada corrompida.
7. Conferir novamente a fonte completa, o estado rastreado do checkout e as referências Git antes/depois.

A CLI só escreve num diretório novo, fora da fonte; não sobrescreve diretórios existentes. A suite remove apenas diretórios que ela própria criou. O relatório destacado identifica os hashes reconstruídos e conserva `whole_wp_ready=false`, `consumer_admission=false` e `independent_review=false`.

## Uso

Com `SOURCE`, `TREE_FILE`, `INDEX_FILE`, `BASIS` e `CURATION` apontando para as entradas acima:

```sh
python3 scripts/plan/recover_inventory.py --source "$SOURCE" \
  --tree "$TREE_FILE" --index "$INDEX_FILE" --basis "$BASIS" \
  --curation "$CURATION" --out /caminho/novo/derivados
```

O workflow `standalone-plan-validation.yml` contém a preparação completa e executa a suite, incluindo a comparação com os outputs reais do candidato.

## Limites e recursos

Python 3.11+ (`tomllib`) e POSIX. Fonte, entradas e destino devem estar quiescentes. O leitor de conteúdo usa descritores relativos/no-follow; a enumeração exige essa mesma precondição de ausência de escritor concorrente. Não há garantia contra processo hostil com o mesmo usuário nem ensaio de perda de energia.

Limites: 16 MiB por entrada, 8 MiB por arquivo-fonte, 256 MiB de fonte, 50.000 registros, profundidade 64 e 4.096 bytes por caminho de leitura. São limites de processamento, não teto de RSS. Verificação de bytes é linear nos bytes; grafos usam conjuntos em passagens por nós/arestas; bindings usam índice por caminho. A serialização ordenada tem custo de ordenação de chaves e não deve ser apresentada como linear genérica. Este incremento não certifica a complexidade de todo coletor histórico.

Falha durante publicação pode deixar um diretório novo parcial. Só um relatório de sucesso, seguido da conferência dos hashes, confirma uma recuperação completa. Nunca restaurar sobre a fonte ou ignorar erro para produzir aprovação. A recuperação não gera uma assinatura de revisor e não substitui SC01/CC02 ou DOD04.
