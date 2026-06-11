# Handoff → githugr lead: estado da espinha web + plano da wave 2 (não executado)

> 2026-06-10, da sessão hugit. O owner redirecionou: telas do githugr são do
> lead do githugr; a sessão hugit volta ao motor. Este doc entrega TUDO que
> estava no prato — estado landado, plano pronto, inteligência de scout — pra
> wave 2 ser retomada de onde está sem re-derivar nada.
>
> **Nada da wave 2 foi commitado.** O agente de contrato (W0) foi abortado
> antes do primeiro commit; a branch `integ/web-w2` foi deletada. Baseline
> limpa: `main @ 390ebf0`.

---

## 1. O que está LANDADO (wave 1 — fechada hoje)

- **`main @ 99cb4be`** — merge da `integ/web-spine`: o crate `crates/hugit-web`
  completo da wave 1 (Provider congelado + mundo-fixture com paridade provada +
  5 telas render-fiéis: landing · repo-home · intent · checks · insights#ledger
  + 5 arquivos de screen-test + provider_fixture.rs). Pushado.
- **Gate completo verde no merge**: fmt + clippy `-D warnings` + test
  `--workspace --locked` + `cargo audit --deny warnings` + `cargo deny check`.
- **`84b40b4`** (incluído no merge) — `deny.toml`: skips novos para duplicatas
  transitivas incolapsáveis (getrandom 0.3 via jobserver/cc · r-efi 5 ·
  windows-sys 0.59 via tokio/mio), no padrão documentado do arquivo. Sem isso
  `cargo deny check bans` falha.
- **`390ebf0`** — fix de CI: `ci.yml`/`dco.yml` rodavam em `ubuntu-latest`
  (GitHub-hosted, pago) e falhavam instantâneo na parede de billing da org
  desde 2026-06-09. Agora: `runs-on: [self-hosted, mac, corelink-builder]`,
  como toda a família. **Não era billing, era o alvo do runner.**
- **Higiene**: 14 worktrees/branches de agente da wave 1 podados (verificado
  um a um: zero commits não-landados; as árvores sujas eram primeiras
  tentativas mortas pelos travamentos do Mac, supersedidas pelo que landou).

Servir local: `cargo run -p hugit-web` em `../githugr` → `http://127.0.0.1:8790`.
(**hugit-web foi migrado para ../githugr**; não existe mais em ../hugit — ver
`chore(workspace)!: hugit-web migrated OUT` no CHANGELOG de hugit.)

## 2. O plano da wave 2 (pronto, NÃO executado — todo seu)

Escopo decidido a partir de `../githugr/design/backlog.md` ("subir as demais
telas de mock pra live") + ranking de custo dos mocks (§3):

```
WAVE githugr-spine-w2 — 6 telas read-only sobre o Provider congelado
  | WP | tela(s)            | arquivos-dono (disjuntos)                          |
  | W0 | contrato (LEAD)    | provider.rs · routes.rs · layout.rs · mod.rs · stubs|
  | W1 | mundo-fixture      | fixture.rs · tests/provider_fixture.rs · app_smoke |
  | W2 | pr-detail (P0)     | screens/pr_detail.rs · tests/screen_pr_detail.rs   |
  | W3 | attention (o forte)| screens/attention.rs · tests/screen_attention.rs   |
  | W4 | dashboard (P0)     | screens/dashboard.rs · tests/screen_dashboard.rs   |
  | W5 | commits + branches | screens/{commits,branches}.rs · 2 test files       |
  | W6 | security           | screens/security.rs · tests/screen_security.rs     |
CORTES: blob (964 linhas; syntax + why-blame por linha → wave 3) ·
        compare (família do diff, junto do blob na wave 3) ·
        auth/conta (Wave 4, fixo — identidade §A1, handoff 2026-06-09)
```

Superfície que eu tinha decidido para o W0 (herde ou re-decida — nada disso
está commitado):

- **Métodos novos no Provider**: `dashboard() -> DashboardVm` ·
  `attention() -> AttentionVm` ·
  `pr_detail(repo, number) -> Option<PrDetailVm>` ·
  `commits(repo) -> Option<CommitsVm>` · `branches(repo) -> Option<BranchesVm>`
  · `security(repo) -> Option<SecurityVm>`.
- **Rotas**: `/` vira dashboard (era redirect pra landing do repo default) ·
  `/attention` · `/r/{repo}/pr/{n}` · `/r/{repo}/commits` ·
  `/r/{repo}/branches` · `/r/{repo}/security` (tab Security sai de "em breve").
- **layout.rs**: `Tab::Security` live + um chrome de conta (`page_account`:
  topbar com sino ◎ e avatar, SEM tabs de repo) pra dashboard/attention —
  fatorar o miolo comum do `page()`, não copiar.
- **Convenções da wave 1 a manter**: VM = struct chata com strings pt-BR
  fiéis; `Option<T>` pra dado honestamente não-capturado; tela = `SCREEN_CSS`
  escopado + `render(&Vm) -> Markup`; handler fino (provider → VM → render →
  chrome); repo desconhecido → 404; badge `fixture` quando o mundo é semeado;
  botões de escrita renderizam disabled-honest; REUSAR `DiffVm`/`HunkVm`/
  `DiffLineVm`/`CheckRowVm` da wave 1 no pr-detail (não criar segunda família
  de diff).
- **Receita de paralelismo da wave 1 (0 conflitos)**: W0 congela TODOS os
  arquivos compartilhados (provider/routes/layout/mod.rs/stubs compiláveis);
  agentes de tela nunca os tocam — cada um é dono só de `screens/<x>.rs` +
  `tests/screen_<x>.rs`; screen-tests com VMs construídas à mão (fixture NÃO);
  o WP do fixture é o único dono de `fixture.rs` + parity tests + `app_smoke`.

## 3. Inteligência de scout (pra não re-pagar o reconhecimento)

Ranking de custo dos 8 mocks candidatos (linhas · veredito):

| mock | linhas | custo | nota |
|---|---|---|---|
| dashboard | 338 | BARATO | lista de repos + inbox strip; dado raso |
| branches | 388 | BARATO | lista com ahead/behind, kebabs são toast no mock |
| commits | 636 | BARATO | git log agrupado por dia; chip `← intent aXX` linka pro intent |
| compare | 403 | MÉDIO | seletores de ref reais; diff unificado↔dividido |
| attention | 446 | MÉDIO | 5 tipos de decisão (LAND/UNIÃO/VERDITO/CLAIM/ESPELHO), filtros, j/k |
| pr-detail | 549 | CARO | 4 ptabs; intents expandíveis; decomposição de custo; rail de revisores |
| security | 638 | CARO (dado) | proveniência assinada, transparency log, supply-chain, secret-gates |
| blob | 964 | MUITO CARO | syntax spans + why-blame POR LINHA + outline + drawer + composer |

- O fixture atual NÃO tem: conteúdo de blob, listas reais de commits/branches
  (só "3 branches" sintético), dados de attention/security. O W1 da wave 2
  precisa semear isso — derivando do motor real onde der (lei de paridade:
  `Ledger::from_records` / `intents_from_log` / `verify_chain`), sintético
  honesto onde não.
- `AttentionRank` existe em `hugit-contracts` (D9) — a tela attention deve
  consumir a forma real, não inventar.
- `interface-map.md` no corpus de design é o checklist componente-a-componente
  por tela (o que é live vs dead no mock).

## 4. Avisos de campo

- **Fence**: sessões githugr não escrevem em `../hugit`. Se a wave 2 continuar
  no crate `hugit-web` (decisão de `architecture.md`), ou ela roda de uma
  sessão hugit, ou o owner aprova carve-out explícito no fence do githugr.
  Resolver ANTES de despachar agentes.
- **CHANGELOG gate**: PRs no hugit exigem entrada no CHANGELOG
  (`.github/scripts/changelog-gate.sh`).
- **Commits**: DCO `Signed-off-by:` + `Co-Authored-By: Claude …` obrigatórios.
- **Mac da casa**: travou 2x durante a wave 1 e matou agentes no meio do voo —
  worktrees sujos órfãos são esperados após crash; verificar landed-ness antes
  de podar (a wave 1 não perdeu nada, mas só porque foi conferido).
- Limite de sessões agora: 4 projetos simultâneos (decisão do owner
  2026-06-10) — dimensione a frota de agentes de acordo.

## 5. Estado das tasks desta sessão (encerradas)

| task | estado |
|---|---|
| Limpeza pós-wave-1 (worktrees/branches) | ✅ feita |
| Gate frio + fix deny.toml | ✅ feita |
| Landar wave 1 no main + push | ✅ feita (`99cb4be`) |
| Fix CI self-hosted | ✅ feita (`390ebf0`) |
| Plano wave 2 | ✅ pronto (este doc, §2) |
| Execução wave 2 | ⛔ ABORTADA limpa — transferida ao lead do githugr |
