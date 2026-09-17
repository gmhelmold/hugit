# HUG-001 — targets dos wrappers históricos

Complemento de `ACCEPTANCE-HANDOFF.md` para SC01/CC02. Não muda o plano,
não executa wrappers nem constitui aprovação independente.

## Fonte e reprodução

Sujeito inspecionado: `96ddcdade3d56d7c2cf4a01c294a56c3aaa4e58c`.
O resolutor exige os digests do inventário publicado e da observação Cargo
preservada em `5dcd4355c8ed896e168f7181e1d5add806ef6aaf:docs/audit/reachability.json`.
Não resolve novas dependências, não avalia shell arbitrário e não acessa serviços.

```sh
python3 scripts/plan/resolve_wrapper_tests.py --source "$SOURCE" \
  --inventory docs/audit/source-inventory.json --basis "$BASIS" \
  --out "$OUTPUT_DIR/wrapper-test-bindings.json"
python3 scripts/plan/test_wrapper_resolution.py --source "$SOURCE" \
  --inventory docs/audit/source-inventory.json --basis "$BASIS" -v
```

O workflow do plano prepara as mesmas entradas históricas, executa ambos,
retém o relatório completo e a saída dos testes. O relatório fica fora do
checkout; a CLI recusa sobrescrita e saída dentro da fonte. Inputs precisam
ser quiescentes e confiáveis. Python 3.11+ e POSIX são requisitos do diagnóstico,
não uma declaração de suporte multiplataforma do produto.

## Disposição das 52 entradas

40 invocações nomeadas resolvem para targets Cargo existentes; sete pedem
pacotes ausentes e duas pedem targets ausentes em pacotes existentes. Os três
wrappers restantes chamam uma suíte de crate, uma suíte do workspace e uma
verificação documental, respectivamente. Todos permanecem no denominador.

| Wrapper (`tests/acceptance/.../run.sh`) | Pacote / target pedido | Disposição |
|---|---|---|
| wp-b6 | hugit-app-sidecar / acceptance_wp-b6 | Pacote ausente |
| wp-b7 | hugit-app-ui / acceptance_wp-b7 | Pacote ausente |
| wp-c2a | hugit-runner / acceptance_c2a | Pacote ausente |
| wp-c2b | hugit-runner / acceptance_c2b | Pacote ausente |
| wp-c3 | hugit-runner / acceptance_c3 | Pacote ausente |
| wp-c5a | hugit-fence / acceptance_c5a | Target ausente |
| wp-c9 | hugit-runner / acceptance_c9 | Pacote ausente |
| wp-e4 | hugit-runner / acceptance_e4 | Pacote ausente |
| wp-x4 | hugit-invariants / acceptance_x4 | Target ausente |

Triagem: HUG-004, com HUG-003 para reconciliação dos gates. Não reativar
CoreLink/runners nem substituir silenciosamente testes para obter verde.
`acceptance_x4_wire` existe, mas não comprova o objetivo do `acceptance_x4`
solicitado pelo wrapper. Os testes preservam explicitamente essa distinção.
Target localizado também não significa teste executado, sem mocks ou offline.

## Evidência e fronteiras

O JSON gerado identifica wrapper, linha/argv declarado, target e arquivo de
origem, hashes e indisponibilidades. `tests_executed`,
`semantic_equivalence_claimed`, `whole_wp_ready` e `independent_review` não são
promovidos a true. SHA-256 do relatório do corpus fixado:
`a483bc4e03065900d39926714c37f3b7df7513d9bbd7c2d54403631724ade2eb`.
Esse pin detecta deriva da representação, não comprova a interpretação sozinho.

Os 19 testes do diagnóstico cobrem declarações restritas, recusa de ambiguidade,
referências indisponíveis, não substituição de X4, repetição, preservação de
bytes, limites de leitura, symlinks/FIFOs, escrita exclusiva e erros limitados.
Leitura usa descritores relativos/no-follow; sem garantia contra escritor hostil
com a mesma identidade. Uma falha de escrita pode deixar um relatório parcial;
somente saída bem-sucedida e verificação do resultado indicam conclusão.

O algoritmo percorre a fonte selecionada e os metadados; a ordenação determinística
das 52 entradas e das chaves JSON é geração, não prova de validação linear genérica.
Caps: 16 MiB por entrada, 8 MiB por leitura de fonte e 256 MiB acumulados; paths
até 4096 bytes/64 componentes. Não são limites universais de RSS.

As verificações de schemas/goldens do gerador continuam delimitadas em
`ACCEPTANCE-HANDOFF.md`: verificar outputs não prova executar `gen_fixtures`;
`UPDATE_SCHEMAS=1` muda comparação para escrita. Esse comportamento não é alterado.

Este documento integra o diagnóstico antes entregue localmente. A decisão final
dos 15 critérios continua no pacote de qualificação, com revisão distinta da
autoria. Integração não fecha #380 nem admite HUG-057 automaticamente.
