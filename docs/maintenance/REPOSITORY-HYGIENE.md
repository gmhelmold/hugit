# Higiene e retenção do repositório

Este registro atende à solicitação do proprietário de não deixar trabalho
perdido, branches soltas ou resultados locais sem rastreamento. Não muda o
planejamento standalone v3, seus cinco axiomas ou critérios de aceite.

## Limpeza de 17 de setembro de 2026

Rastreio: [#449](https://github.com/gmhelmold/hugit/issues/449).
Baseline: `f32e9b9253a9a98f5d245fd3fca700a956ebf1fe`.

O snapshot tinha 96 branches e nenhum PR aberto. Foram retiradas 94 branches:
57 pontas já eram ancestrais do `main`; 37 pontas fora dessa ancestralidade foram
preservadas, nos seus SHAs exatos, em tags `archive/2026-09-17/*` antes da
remoção. Dessas 37, 32 correspondem a cabeças de PRs já integrados, duas são
operações de importação/coleta concluídas e três são experimentos do antigo
servidor. Arquivamento não significa que o patch foi integrado ou validado.

A branch `archive/standalone-v3-import-before-signoff` foi mantida porque é
referenciada por `docs/plan/standalone/github/IMPORT-VERIFIED.md`. Com `main`,
são as duas branches persistentes ao final da limpeza. Branches de PRs novos
têm ciclo de vida próprio e não alteram o snapshot histórico deste registro.

A lista completa, com SHA original, destino de retenção e disposição de cada
referência, está em [branch-retention-2026-09-17.json](branch-retention-2026-09-17.json).
Ancestralidade foi conferida pela API e por Git local. Criação de tags usou
leases de ausência; remoção usou leases dos SHAs exatos, em push atômico.
Se alguma ponta mudasse, a operação recusaria a alteração em vez de apagar
trabalho concorrente. `main` não foi movido pela limpeza das referências.

Os workflows dos commits arquivados foram examinados: as tags de arquivo não
correspondem ao padrão `v*` de release nem às branches dos workflows operacionais.
Nenhum release ou serviço legado foi acionado pela operação. Os commits de
servidor foram arquivados, não transplantados para o runtime standalone.

## Recuperar uma ponta arquivada

Consultar primeiro o SHA e a referência na lista de retenção. Para uma tag de
arquivo, obter esse ref e criar uma NOVA branch de trabalho; não redefinir
`main`, não sobrescrever uma branch existente e não executar o código antigo
como parte da recuperação. Para pontas ancestrais, o SHA continua no histórico
completo de `main`. A branch histórica mantida continua no endereço original.
Uma recuperação de trabalho exige sua própria issue/PR e avaliação de escopo.

## Disciplina para as próximas entregas

- Trabalho de produto tem WP/issue e branch vinculados; nenhum bloqueio é
  escondido em um ZIP, comentário isolado ou rascunho sem responsável.
- Antes de publicar, conferir `git status --porcelain=v1 --untracked-files=all`,
  o diff e os arquivos efetivamente staged. Não usar limpeza destrutiva para
  descartar arquivos que não foram produzidos pela própria execução.
- Logs, arquivos temporários, bundles e evidências B ficam fora do checkout.
  Os artefatos necessários são preservados com origem, identidade e localização;
  não são copiados ao código apenas para produzir um commit.
- Uma entrega termina com commit, PR, checks do candidato e disposição explícita:
  merge verificado ou fechamento fundamentado. PR mergeado não aceita sozinho o WP.
- A exclusão automática da branch de PR após merge foi habilitada. Exceções de
  retenção precisam estar documentadas; apagar a última referência de trabalho
  exclusivo sem preservação continua proibido.

## O que continua pendente, sem aprovação fictícia

[HUG-001 / #380](https://github.com/gmhelmold/hugit/issues/380) permanece em revisão,
com bloqueio de parecer distinto da autoria. A submissão identifica as 15
assertions e as evidências, mas não equivale a aprovação. A sessão CLI separada
tentada nesta manutenção foi recusada por limite de uso antes de produzir um
parecer. Não houve compra de créditos, mudança de limites ou substituição por
uma identidade fictícia. Os dependentes seguem sujeitos ao DAG aprovado.

Os defeitos de produto já identificados continuam nos WPs existentes; limpeza
de branches não os corrige. O programa completo permanece na
[issue-mãe #379](https://github.com/gmhelmold/hugit/issues/379). Não se declara
"zero débito técnico" enquanto essas obrigações não forem demonstradas.
