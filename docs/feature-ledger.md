# Feature Ledger — hugit (o gestor de agentes que anda em cima do git)

> O equivalente a um "menu" do produto: TUDO que o hugit faz, cada linha
> explica o que FAZ em uma frase curta. Serve de checklist de validação
> manual — se você consegue executar a linha, a feature funciona.

## Como ler / validar
- Cada linha = uma feature + o comando/rótulo que a dispara.
- Validação: rode o comando num repo de teste encenado (`docs/manual-validation.md`).

---

## Evidência — onde cada feature é PROVADA

O que segue é a **coluna de evidência** de cada feature: o teste hermético /
acceptance que a prova (red→green), e se a feature foi **usada de verdade**
por um humano na walkthrough manual deste repo (validada com o binário real,
não só nos testes).

Legenda da última coluna: **T** = provada por teste · **U** = usada de verdade
numa walkthrough manual (binário real, repo real) · **—** = provada só por
teste/invariante, não exercitável por uso manual simples (requer infra/servidor).

| Grupo | Teste que prova (red→green) | Usada de verdade |
|---|---|---|
| hooks + capture (seção 0) | `acceptance_dock_coinage` (e2e worktree) · `crates/hugit-cli/src/capture/mod.rs` (unit) | U (repo real, commit/checkout capturados) |
| campanhas (1) | `crates/hugit-cli/src/campaign/*` (unit) + `acceptance_*_cli` | U (open/close/selo/abandon + guarda sealed testada com `intent new` fix) |
| intents (2) | `crates/hugit-cli/src/intent/new.rs` (8 unit) + `acceptance_wave_m_intent_atomicity` | U (new/show/list + fix de selo) |
| issues (3) | `crates/hugit-cli/src/issue/transition.rs` | U (transition real) |
| PRs (4) | `crates/hugit-cli/src/pr/cli.rs` unit + acceptance pr | U (open/queue/show/list/abandon, D14, commits-only) |
| landing (5) | `crates/hugit-cli/src/land/mod.rs` unit | U (land queue real: landed PR-1) |
| checks/verdict (6) | `crates/hugit-cli/src/checks/mod.rs` + `verdict/mod.rs` unit | U (check run miss→HIT real, verdict approve, policy test) |
| provenance (7) | `why` unit (`main.rs`) + `acceptance_*` | U (why/export/impact/undo/diag/watch/fleet/symbol/ctx/review) |
| custo (8) | `crates/hugit-cli/src/ctx/usage.rs` | U (ctx usage verbatim) |
| dock (9) | `acceptance_dock_{coinage,resolver,reconcile,insights,land}` + dock lib unit | U (coin/ls/land/insight/reconcile num ghost real) |
| serve rotas (10) | `crates/hugit-serve/src/...` unit + e2e | — (exigem servidor/CAS/deploy: bindings R2/D1/PAT) |
| contratos (11) | conformance vectors + x4 validator | T (CI) |
| integridade (12) | `crates/hugit-refstore/src/tamper.rs` + authz unit | U (D14 na prática: `pr open` human exige principal) |
| extras (13) | `hugit-mirror`/`hugit-policy`/`hugit-fence` unit | — (mirror: smoke live; policy: U via `policy test`) |

**Débitos fechados durante esta walkthrough** (vistos no uso real):
- `intent new` ignorava campanha selada no modo `--store` → **FIXADO** + teste que morde (`intent_new_refuses_campaign_sealed_on_default_log`, mutation-probed RED).
- Falso alarme "watch inconsistente" → comportamento correto do detector de bare-hex no sha do commit (40-hex redacta por design).
- Falso alarme "exit 0 em erro de CLI" → era o pipe; sem pipe o exit é 2 (correto).

**O que NÃO é provado por esta walkthrough** (honesto — está marcado `—`):
- Rotas do servidor (`/v1/*`, git wire) — exigem `hugit-serve` com CoreLink CAS/R2/D1/PAT provisionado; o código+unit green, mas não há instância live acessível daqui.
- O lado Omnirouter (irmão) do `CostSampleV1` — só o conformance vector da nossa banda prova a paridade.
- Redação, mirror, fence, MCP — cobertos por unit/hermético, não por uso manual (precisam de infra ou de ambiente específico).

---

## Separar serve do núcleo — decisão de arquitetura (plugin open-source)

O hugit **não precisa de `hugit-serve`** para funcionar. O produto é git-native:

```
repo git (git clone/push/pull normais — qualquer remote)
  └─ .hugit/log.json     ← versionado, viaja com clone/push/fetch (não está no .gitignore)
hugit CLI (local, sem serviço)   ← lê o log, roda todos os verbos
```

- **O CLI e os crates de núcleo não usam `hugit-serve`.** Sentido das dependências: `hugit-serve` → `use hugit-cli` (o serve importa o CLI); nenhum crate de núcleo importa o serve. `hugit-serve` é `publish = false`.
- **`hugit-serve` é a janela web opcional** (o que a UI githugr lê), e o CAS CoreLink é só o deploy DESTA janela. Nada disso entra no plugin open-source: um repo git + o CLI bastam.
- **Fluxo sem serve:** clone → `hugit campaign/intent/pr/check/dock` (local) → `git push` (o log vai junto) → outros agentes `git pull` sincronizam.
- Publicação open-source = CLI + crates de núcleo; serve fica de fora (quem quiser a janela sobe o githugr, opcional).

---

## 0. Instalação do "sensor" (hooks do git)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit init` (library) | Cria `.hugit/` + log vazio + instala hooks do git (post-commit, post-checkout, pre-push, post-merge). | `hugit init` numa pasta; ver `.git/hooks/post-commit` existe |
| Capture silencioso | Toda operação do git (commit, checkout, push, merge) é registrada no log SEM travar git — o hook roda em background e sempre sai com sucesso. | `git commit` num repo init; ver o evento no `log.json` |
| Capture por tipo | Registra o quê aconteceu: commit (arquivos novos), checkout (branch nova), push (hashes enviados), merge (origem+dst). | Commitar, ramificar, push, merge; conferir cada payload no log |

## 1. Campanhas (agrupamento de trabalho)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit campaign open` | Cria uma campanha (ex: "release v2") com dono e missão. | `hugit campaign open --campaign v2 --charter "..." --owner voce` |
| `hugit campaign close` | SELAR a campanha: gera a prova completa (tudo pousou, custo, envelope) e congela. | Abrir intents, pousar; `hugit campaign close` |
| `hugit campaign show` | Mostra o que está pousado, em voo, bloqueado. | `hugit campaign show --campaign v2` |
| `hugit campaign list` | Lista campanhas. | `hugit campaign list` |
| `hugit campaign abandon` | Descarta campanha com motivo (idempotente). | `hugit campaign abandon --campaign v2 --reason "preciso"` |

## 2. Intents (pedidos de trabalho)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit intent new` | Declara uma tarefa: missão + critérios de aceite + campanha. Devolve um ID. | `hugit intent new --charter "rate limit" --acceptance "testes verdes"` |
| `hugit intent show` | Mostra a tarefa + proofs (verdicts, custo). | `hugit intent show --id <id>` |
| `hugit intent list` | Acha pedidos perdidos (por armazenamento/campanha). | `hugit intent list` |

## 3. Issues (rastreamento, paridade com o servidor)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit issue transition` | Move issue entre estados (backlog/open/closed/dispatch). | `hugit issue transition --n 3 --to open` |

## 4. PRs (Pull Requests do time de agentes)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit pr open` | Junta intents numa PR — prova-se que um commit específico pertence à PR. O subagente/autor humano é o escritor (agentes não são autor). | `hugit pr open --campaign v2 --intent <id>` |
| `hugit pr open --commit <sha>` | Aceita um commit capturado como conteúdo (para PRs sem intents, commits-only). | commit; `hugit pr open --commit $(git rev-parse HEAD)` |
| `hugit pr open --commit-ref <branch>` | Resolve a cabeça capturada da branch. | `hugit pr open --commit-ref main` |
| `hugit pr queue` | Coloca a PR na fila de pouso (union queue). | `hugit pr queue --pr <pr>` |
| `hugit pr land` | Marca a PR como pousada (terminal). Flags de custo real + dispatch opcional. | `hugit pr land --pr <pr>` |
| `hugit pr show` | Detalhe da PR + rollup de custo. | `hugit pr show --pr <pr>` |
| `hugit pr list` | Todas as PRs com estado/filtro. | `hugit pr list --campaign v2` |
| `hugit pr abandon` | Descarta PR com motivo (idempotente). | `hugit pr abandon --pr <pr> --reason "nao"` |
| Guarda de campanha selada | Nada mais muda em campanha selada. | Tentar PR numa campanha fechada |

## 5. Landing (o pouso automático do que está pronto)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit land queue` | Roda o motor de união REAL: testa os PRs da fila juntos, isola o par culpado no vermelho (bisect) e pousa os verdes. | `hugit land queue` (com PRs na fila) |
| `hugit queue show` | A fila de pouso, pares em conflito, veredito retido. | `hugit queue show` |
| `queue.union_fail` | União vermelha registra o par mínimo de conflito (bisect). | Causar 2 PRs em conflito; dar land queue |

## 6. Verificação / Checks (o teste memoizado)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit check run` | Roda um check REAL memoizado: árvore+definição+ferramenta = chave; o mesmo conteúdo = HIT da cache, ZERO execução. | `hugit check run --def fmt --store` 2x; 2ª = HIT |
| `hugit check run --env-axi VAR` | Declara variável extra na chave do memo — sem "verde velho" indetectado. | `... --env-axi FOO` trocando FOO |
| `hugit check show` | Mostra checks + taxa de acerto da cache (hit-rate). | `hugit check show` |
| `hugit check key` | Prevê o memo-key de uma check sem rodar. | `hugit check key --tree root --def fmt` |
| `hugit verdict record` | Painel adversarial multi-lente (≥2 modelos distintos); aprova só se TODOS aprovarem. | `hugit verdict record --intent ... --lens 2 --result approve` |
| `hugit verdict approve|reject` | Decisão de stakeholder humano (uma lente, fixed lens). | `hugit verdict approve --intent <id>` |
| `hugit verdict` Q&A | Pergunta com base em evidência real — recusa sem ela. | Consultar algo fora do registro |

## 7. Entendimento / Provença (auditoria)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit why` | Responde: "de onde veio esta linha/arquivo?" → intent de origem. | `hugit why --path src/x.rs` |
| `hugit why --walk` | Projeta a cadeia COMPLETA de provenance (mais recente primeiro). | `hugit why --walk --path src/x.rs` |
| `hugit why --line/--symbol` | Atribui linha específica ou função. | `hugit why --path x.rs --line 42` |
| `hugit impact` | Raio de explosão em grafo de build (Cargo/pnpm/Turbo). | `hugit impact --graph build.json --path x.rs` |
| `hugit tournament` | Expande um intent em N candidatos (orçamento limitado). | `hugit tournament -n 3 --intent <id>` |
| `hugit export` | Prova de saída anti-lock-in: dump do git + envelope JSON + prova redatada. | `hugit export --out pasta/` |
| `hugit undo` | Desfaz um evento (compensatório, humano). Nunca reescreve histórico. | `hugit undo --seq <n>` |
| `hugit ledger` | Visão: pedido → feito → provado, por campanha. | `hugit ledger --campaign v2` |
| `hugit note` | Grava nota de sessão no log (scrub). | `hugit note --note "hoje..."` |
| `hugit diag` | Diagnostica check vermelha: acha o commit/região que quebrou (bisect no log). | `hugit diag --def-digest <hex>` |
| `hugit watch` | Refaz o stream de eventos classificado (landing/verdict/policy/ws/git). | `hugit watch --class landing` |
| `hugit fleet` | Estado das workpaces + agentes (schema versionado). | `hugit fleet` |
| `hugit ctx resume` | Reconstrua uma sessão caída (horizonte de notas; honesto: recusa sem evidência). | Cair de uma sessão; `hugit ctx resume` |
| `hugit review` | Q&A com evidência: responde citando check/verdict ou recusa. | `hugit review --question "..."` |
| `hugit symbol` | Outline de símbolos (tree-sitter) do arquivo local. | `hugit symbol --file src/x.rs` |
| `hugit export` | Prova de saída (ver acima). | (listado) |

## 8. Costos (o cost killer)

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit ctx usage` | Grava custo do provedor VERBATIM (tokens reais, nunca preço). | `hugit ctx usage --intent <id> --model ... --input ... --output ...` |
| `hugit dock` (série completa) | O custo por-unidade — ver seção 9. | — |

## 9. Séria dock (o custo por-unidade)

**Série dock = 6 WPs** (construídos + cold-verificados). Resumo: um repo tem uma ou mais "docks" (worktrees/repo); o custo pousa na dock; a reconciliação decide o balde certo; os insights mostram por-branch; o land verifica o conteúdo.

| Feature | O que faz | Como validar |
|---|---|---|
| `hugit dock coin` | Cria uma dock no checkout (worktree/repo) — o binding físico para custo. Auto-disparado pelo hook. | `git worktree add` num repo init → `dock ls` mostra |
| `hugit dock ls` | Lista docks (com fantasma detectado). | `hugit dock ls` |
| `hugit dock show <id>` | Detalhe da dock. | `hugit dock show <id>` |
| `hugit dock close` | Finaliza + reconcilia (baldes), idempotente. | `hugit dock close --id <id>` |
| `hugit dock reconcile` | Fecha docks-fantasma (worktree apagada) automaticamente. | Apagar worktree; `hugit dock reconcile` |
| `hugit dock insight` | Custo por branch (nunca duplicado) + baldes: investigado / não-rótulo / falsificado. | Com costos → `hugit dock insight` |
| `hugit dock land` | Verifica: SHA da worktree == SHA registrado + aceite verde via executor memoizado. Só pousa se TUDO bater. | `hugit dock land --id <id>` |
| Filhote de custo (spool) | Samples de custo spoolados em NDJSON local; offline NUNCA perdem. | `dock/spool.rs` (testes) |
| Atestação de custo (attest) | Flush spool→log como `cost.sample`, idempotente por run_id, com content_hash verificado (M3) — falsificado é contado mas NUNCA atribuído. | Injetar sample falso; `dock insight` mostra `tampered` |
| F2 honesto | Custo ausente = zero honesto; presente = real. Nunca inventa. | `dock land` sem cost.sample reporta 0 |

## 10. Serve (o backend HTTP que o front lê)

### Git wire (clone/fetch/push via git REAL)
| Feature | O que faz | Como validar |
|---|---|---|
| Git clone/fetch | Git padrão (smart-HTTP upload-pack); a engine serve do CAS (lazy). | `git clone http://engine/.../repo.git` |
| Git push | Push padrão com `unpack ok` + `ok refs` (write path fail-closed: durable só após CAS+D1+refs). | `git push origin main` |
| Delete ref | `git push --delete` (capabilidade advertised). | `git push --delete branch` |
| Live hot-swap | Um push imediato aparece no advertise sem reboot. | Push → `git ls-remote` |
| Thin-pack / incremental | Push incremental sobre histórico CAS-base — resolve base omitida. | Commit novo → push incremental |

### Leitura (GET /v1)
| Feature | O que faz | Validação |
|---|---|---|
| `/readyz` | Healthcheck público (git_serving, git_repo, version, cold-start). | `curl /readyz` |
| `/metrics` | Contadores agregados públicos (zero dados de tenant). | `curl /metrics` |
| `/v1/me/login` | Cartão de auth pública (Clerk→engine token). | curl |
| `/me/dashboard`, `/me/attention`, `/me/account`, `/orgs/{name}` | Painéis do próprio usuário | authed curl |
| `/v1/admin/tokens` | Sessões ativas (tenant-scoped). | authed |
| `/v1/repos/{r}/home` | Árvore raiz + README, cache content-addressed. | authed |
| `/v1/repos/{r}/blob|edit/{*path}` | Bytes reais do arquivo, scrub, 404-no-oracle, history walk | authed |
| `/v1/repos/{r}/commits|compare|branches|chrome` | Commits/diff/branches from CAS | authed |
| `/v1/repos/{r}/insights` | Vista cost/per-branch + ledger + x-ray (owner-gated até custo non-zero) | authed |
| `/v1/repos/{r}/prs`, `prs/{n}`, `intents/{id}`, `campaigns/{name}` | Detalhes | authed |
| `/v1/repos/{r}/events?since=` | SSE replay-then-close, authed+tenant, sempre privado | authed |
| `/v1/repos/{r}/search?q=` | Busca de código servida só de index pré-computado; q capped | authed |
| `/v1/repos/{r}/viewer-can` | Espelho real do gate de escrita por chamador | authed |
| `/v1/repos/{r}/audit?since` | Linha do tempo de authz/denial (operador) | operador |
| `/v1/repos/{r}/erasure` `admin/overview` | Governance de apagamento + visão operador | operador |

### Escrita (POST)
| Feature | O que faz | Validação |
|---|---|---|
| `POST /v1/token` | Clerk JWT → engine token opaco | curl |
| `POST /v1/github/webhook` | GitHub App webhook ingress, HMAC authed, durável (nunca inline) | env |
| `POST /v1/repos` | Cria repo vazio self-service | authed |
| `POST /v1/me/tokens` + `DELETE /v1/me/tokens/{id}` | Mint PAT (secret once) + revoga; PAT não pode criar outro PAT | authed |
| `POST /v1/repos/{r}/prs` | Abre PR de branch pushead vs base (resolve vs LIVE ref snapshot) | authed |
|POST /prs/{n}/land/verdict/comments | Land/verdict/comentrios no PR | authed |
|POST /intents/{id}/usage | Cost-killer seam sobre o wire | authed |
| `POST /v1/repos/{r}/dispatch` | Dispara exec agent (P2/transferred) | authed |
| `POST /v1/repos/{r}/issues/{n}/transition` | Movimento issue | authed |
| `POST /v1/repos/{r}/policy` | Policy write | authed |
| `POST /v1/repos/{r}/erase/{id}/decide` | Decisão de apagamento (operador) | operador |
| `POST /v1/repos/{r}/edit/{*path}/propose` | Propor edição | authed |
| `POST /v1/repos/{r}/undo` | Undo no server | authed |
| `POST /v1/repos/{r}/meta` | Visibility/tenancy write (cache-refresh) | authed |
| `POST /v1/account/erase` `.../execute` `.../cancel` | GDPR1: apagar conta, 2-mãos+grace+situa gauge, cancelar na cooldown | authed/operador |
| D14 write door | Scope-gated: `repo:read` PAT → 403; idempotency-key cap; step-up dev | authed |
| Rate limiting + panic isolation | `shed` 503 por principal + handler panic NÃO derruba engine | load/atack test |

## 11. Contratos congelados (a cola entre hugit e CoreLink/Omnirouter)

| Feature | O que faz | Como validar |
|---|---|---|
| `CostSampleV1` + conformance | DTO de custo byte-idêntico entre hugit e irmão, pined por vector + tripwire | CI x4 |
| Runner lease DTOs (4) | Contrato de lease do runner congelado, byte-idêntico | conformance |
| `ContextEnvelope`/`IntentMetrics` | Envelope de contexto (ADR-0001) | conformance |
| Attestation keyset | Seleção/verificação de chave de atestação, anti-downgrade | conformance |
| `CheckerResult`/`CheckDef` | Definição/resultado de check | — |
| EventRecord chain | Registro hash-encadeado (tamper detectável) | tamper test |

## 12. Integridade / garan hias

| Feature | O que faz | Como validar |
|---|---|---|
| Hash-chain log | Cada evento referencia o hash do anterior; qualquer tamper quebra a cadeia | `hugit log` falha em arquivo alterado |
| D14 authz | Matriz Principal×Endpoint; undo é humano-only; PR author é human/orch, nunca subagente | tentar o subagente |
| Redação | Segredos scrubbed antes de gravar/servir | PAT em charter vira `[REDACTED]` no log/export |
| Repo.meta visibility | `public|private`; leitura de privado = 404 uniforme (no oracle) | anon → 404; public → lê |
| Leitura-authz≠escrita-authz | Ler púbico NÃO permite escrever (push sempre precisa credencial) | anon clone + push → 401 |

## 13. Extras

| Feature | O que faz | Como validar |
|---|---|---|
| Mirror GitHub (outbound) | Espelho one-way hugit→GitHub com per-push. hash verify + fila durável | Teste live (há smoke) |
| Import git history | Importa histórico git com byte-identity + LFS + resumable | integração |
| Bidir sync forge-arbitrated | Sync bidirecional arbitrado pelo forge | — |
| Policy `Engine::house` | Gate-set dco/changelog/secrets; local≡forge byte-idêntico | `policy test --context` |
| Fence broker | Credenciais nunca chegam ao runner | — |
| MCP server | 4 tools p/agent: claim-disjointness, land-status, cost-attest, live-ness | — |
| Fice-manifest | Fence do irmão NV? | — |

---

## Nota honesta (o quer NÃO é feature ainda)
- `ws`/`dispatch` — verbos reservados (P2/transferred para runner).
- `/insights` janela cost — BUILT mas owner-gated até custo non-zero.
- Multi-tenant real — downstream (githugr infra).
- O gateway irmão (Omnirouter) emite o `CostSampleV1` — só o nosso lado está comprovado.

---

## Próximo passo
Eu te guio na **validação manual grupo a grupo** — o roteiro `docs/manual-validation.md` (user-real simulation, já executada: 37-assert go-live script + walkthrough do binário real; evidência + bugs encontrados documentados lá).