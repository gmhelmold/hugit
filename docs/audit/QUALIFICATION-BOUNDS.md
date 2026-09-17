# HUG-001 — limites do conjunto de verificação

Este documento descreve o perfil de execução dos verificadores do inventário.
Não altera QS03, não é benchmark do Hugit e não concede aceite independente.
O sujeito do código inventariado é `96ddcdade3d56d7c2cf4a01c294a56c3aaa4e58c`.
Os verificadores e este documento têm a identidade do candidato de integração,
registrada separadamente na evidência de CI/PR.

## Domínio e pré-condições

A qualificação usa Python 3.11+ em POSIX, snapshot quiescente, objetos Git
históricos disponíveis e inputs com identidades conferidas. Os diretórios de
relatórios e reconstrução são temporários próprios, separados da fonte. Não
executa a fonte Rust, hooks, wrappers antigos, clientes HTTP ou publicadores.
A CI de produto/downloads é uma atividade distinta destes verificadores.

Não se promete proteção contra um escritor hostil com o mesmo usuário. Os
readers relativos/no-follow recusam links e arquivos especiais no percurso
qualificado; não constituem um sandbox geral. Os caminhos explicitamente
fornecidos pelo invocador, permissões de saída e disponibilidade do filesystem
continuam sendo pré-condições. Não usar `--report` apontando para a fonte.

O sujeito e o plano são fixados antes da comparação. Atualizar a identidade
junto com dados inconsistentes não transforma hashes em revisão semântica.

## Trabalho e memória

Notação: B = bytes lidos; N = registros; L = bytes de caminhos; E = arestas;
D = profundidade de caminho; A = âncoras; P = perfis. Operações em tabelas hash
são de custo esperado constante, não de pior caso formal universal. O custo de
JSON inclui seus bytes/objetos. Contar apenas arquivos omite B e L.

| Componente | Trabalho inspecionado | Condição/limite relevante |
|---|---|---|
| `verify_inventory.py` | Passagens por enumerações, registros, bytes e bindings. Tree folding pode revisitar bytes de subárvores pelos ancestrais: O(D(B_enum + L + N)), com D limitado; hashing de fonte O(B). | JSON/enumeração até 16 MiB por entrada; 50.000 registros; 8 MiB por arquivo; 256 MiB de fonte; profundidade 64; caminhos até 4.096 bytes; chunks de 64 KiB; visitas de diretório limitadas. |
| `verify_surface_contracts.py` | Indexa arquivos/tasks, compara conjuntos de entradas/variantes/targets; lê cada arquivo-âncora uma vez. Construir/digerir trechos custa os bytes efetivamente revisitados pelas âncoras. | 128 perfis e 256 âncoras no máximo; cache de fonte contabilizado, incluindo o plano; readers de 16 MiB/8 MiB/256 MiB. Não inferir custo de expansão de macros ou call graph: não são executados. |
| `verify_automation.py` | Uma passagem pelos arquivos; seleção por extensão/modo/shebang; uma passagem pelas entradas. Resumo por contagem O(N + P); paths externos seguem a ordem do inventário fixado, sem sort comparativo. Âncoras podem dividir novamente o arquivo, com A limitado. | 64 perfis e 128 âncoras; mesmos limites de fonte/readers. Não interpreta shell/Python arbitrário. A regex externa identifica configuração declarada, não tráfego observado. |
| `recover_inventory.py:check_dependencies` | Comparações do lock, multisets de pacotes e passagens por nós/arestas de dois perfis obrigatórios. | `default` e `all-features`, limites de registros/bytes e identidades históricas obrigatórios. Não é nova resolução Cargo. |
| Reconstrução/serialização | Reenumera e recompõe os três derivados; bindings compartilhados e ordenação de chaves têm custos próprios. | `encoded(..., sort_keys=True)` não é apresentado como algoritmo linear genérico. Reconstrução é separada da validação; o perfil fixado tem 60 WPs. |

Com D, A e P limitados pelo perfil, a validação faz passagens limitadas nos bytes
e registros, em vez de busca transitiva irrestrita ou execução de ferramentas.
Isso não fornece um teto universal de RSS/latência. JSONs, strings, linhas,
sets, buffers de serialização e relatórios ocupam memória além dos bytes de
fonte. Limite de input não é limite de RSS. O processo/CI também tem seu timeout.

O relatório JSON ainda ordena suas chaves de metadados: esse é custo de
serialização de campos/perfis/destinos limitados, não ordenação da lista variável
de paths. O histórico anterior do resumo fazia P varreduras das N entradas e
ordenava os paths externos. A alteração remove ambos, preservando os valores do
relatório para o inventário fixado. Uma ordem de inventário diferente é outra
entrada: não se promete equivalência byte a byte entre fontes diferentes.

## Diagnósticos e efeitos de saída

As falhas de dados tratadas por `verify_inventory` devolvem código de erro
limitado; as demais interfaces usam códigos internos constantes ou
`SCHEMA_INVALID`/`SETUP_OR_SCHEMA_INVALID`. Não imprimem o payload que falhou.
Isso descreve a interface de validação com argumentos válidos e saídas
preparadas, não a ajuda do argparse, traceback de bug não capturado ou falha do
ambiente ao gravar um relatório. Esses casos não podem produzir aceite.

`verify_*` não escreve no snapshot sob a invocação qualificada. Alguns aceitam
`--report`; a separação desse caminho deve ser conferida pelo invocador. O
reconstrutor só publica em diretório novo, externo à fonte, e pode deixar saída
parcial se a publicação falhar. Só sucesso mais conferência de hashes confirma
uma recuperação; nunca recuperar sobre o produto.

## Evidências e testes que sustentam a análise

- Aquisição: `scripts/plan/test_inventory.py`; igualdade Git/manifesto, corrupção,
  modos, path bindings, limites, links e arquivos extras. Não autentica revisor.
- Superfícies: `scripts/plan/test_surface_contracts.py`; conjuntos/variantes,
  efeitos críticos, destinos reais, owner e identidade do plano.
- Automações: `scripts/plan/test_automation.py`; denominador, perfis, âncoras,
  classificação externa e efeitos críticos. Os testes novos medem acessos ao
  campo de perfil e recusam o sort de paths no percurso testado. Não são prova
  completa de complexidade, benchmark de tempo ou instrumento de todos os custos.
- Recuperação: `scripts/plan/test_recover_inventory.py`; descarte/reconstrução,
  igualdade dos três derivados, dependências e falhas de preparação. Ver
  `RECOVERY.md`. A CI compara fonte, refs e status antes/depois.

Resultados, SHAs do candidato, logs e disposição de QS03 pertencem ao pacote de
evidência B e à revisão. Não gravar neste documento um aceite que altera o
próprio sujeito aprovado. Os cinco axiomas e o gate de revisão distinta continuam
inalterados. Um revisor pode reprovar esta análise sem alterar o contrato.
