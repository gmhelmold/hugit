# githugr — inventário de gaps (o que falta pra ser completo)

> 2026-06-09. Deep-dive do que ainda falta, partindo do que já foi desenhado nos
> mockups (repo home/Code, Landing(PRs) kanban + gaveta de PR com intents/
> context.json/diff/verdicts/maximize). Status: **✅ desenhado · 🟡 parcial · ❌ falta**.
> Prioridade: **P0** = MVP (sem isso não shippa) · **P1** = logo depois · **P2** =
> later · **🟡mirror** = linguiça, deep-link pro GitHub.

---

## A. Repo / Code (table-stakes, clone-GitHub-fiel)
| item | status | prio |
|---|---|---|
| Code home (file tree + README + About) | ✅ | P0 |
| **Blob/file view** (syntax highlight, nº de linha, raw, copy, permalink) | ❌ | P0 |
| **Why-blame na linha do arquivo** (hover → intent/modelo/raciocínio/prova) — killer #2 | ❌ | P0 |
| Navegação de diretório + breadcrumb na árvore | ❌ | P0 |
| **Go to file** (fuzzy, atalho `t`) | ❌ | P1 |
| Commit/intent **standalone** (página de um commit) | ❌ | P1 |
| **História** — `git log` (altitude máquina) ↔ `hugit log`/Ledger (altitude intent), com toggle | ❌ | P1 |
| Branches: lista + **compare/diff entre refs** | ❌ | P1 |
| Diff standalone (compare A..B), unified ↔ side-by-side | 🟡 (diff só na PR) | P1 |
| Raw / download / "abrir no workspace" (hidratar) | ❌ | P1 |
| Tags / Releases | 🟡mirror | P2 |
| Editar arquivo no browser | ❌ | P2 |
| **Busca de código** (índice semântico) | ❌ | P2 (research-grade) |
| Code-intelligence (jump-to-def, hovers) | ❌ | P2 |

## B. Landing (PRs)
| item | status | prio |
|---|---|---|
| Kanban (bundles por campanha, subliminar) | ✅ | P0 |
| Gaveta do PR: header + intents(commits) + context.json + verdicts + maximize + breadcrumb | ✅ | P0 |
| **Diff/arquivos do PR** (file list + hunks) | 🟡 (1 hunk demo) | P0 |
| **Vista em lista** (flat, GitHub-style) como toggle do kanban | ❌ | P1 |
| **Abrir PR / criar intent** (o fluxo de criar) | ❌ | P0 |
| **Conversa/timeline do PR** (comentários, eventos, discussão humana) | ❌ | P0 |
| **Comentário inline no diff** (revisar linha) | ❌ | P0 |
| **Interrogação** ("onde toca o dinheiro?", "me convença") — killer | ❌ | P1 |
| **Submeter verdict / aprovar / rejeitar** (fluxo de review humano + painel) | 🟡 (mostra, não interage) | P0 |
| Reviewers, labels, assignees, linked issues, milestone | ❌ | P1 |
| Draft ↔ ready-for-review | ❌ | P1 |
| **Resolver union-fail** (re-fatiar ou serializar — UI) | 🟡 (mostra bloqueado) | P1 |
| **Regenerative rebase** (re-executar intent na base nova) — mecânica core | ❌ | P1 |
| Opções de land (requisitos, fila, política) | 🟡 (botão Land) | P1 |

## C. Checks / CI (memoizado)
| item | status | prio |
|---|---|---|
| **Aba Checks** (lista, cache-hit vs exec, affected-targets, hit-rate) | ❌ | P0 |
| **Logs do check** (output, byte-identity) | ❌ | P0 |
| **Auto-bisect / diagnóstico** (red → culpado) — killer | ❌ | P1 |
| Flake stats / quarentena | ❌ | P1 |
| Cost X-ray rollup (custo por check/PR/campanha) — killer #5 | 🟡 (chips) | P1 |

## D. Security
| item | status | prio |
|---|---|---|
| **Aba Security** (atestação/proveniência SLSA, transparency log) | ❌ | P1 |
| Supply-chain (deps pinadas por digest, cargo audit/deny) | ❌ | P1 |
| Secret-scanning (o gate de secrets) | ❌ | P1 |
| Erasure / right-to-be-forgotten (cascade) | ❌ | P2 |

## E. Insights / Ledger
| item | status | prio |
|---|---|---|
| **Ledger** (asked→done→proven, por campanha) — killer | ❌ | P0 |
| Insights: velocity, hit-rate, custo/economia, contribuição (humanos+agentes) | ❌ | P1 |
| **Self-explaining codebase / knowledge layer** — killer #6 | ❌ | P2 |
| Session replay + diff-of-minds (viewer real) — killer #4 | 🟡 (botão) | P2 |

## F. Settings / Política / Sync
| item | status | prio |
|---|---|---|
| Settings geral + collaborators/teams | 🟡mirror | P1 |
| **Editor de política** (gates fail-closed = nosso branch-protection) | ❌ | P0 |
| **Config de sync GitHub** (direção, repos, mirror) | ❌ | P0 |
| Webhooks / integrações | 🟡mirror | P2 |
| Secrets do repo (broker) | ❌ | P1 |

## G. Global / conta (fora do repo)
| item | status | prio |
|---|---|---|
| **Lista de repos / dashboard** (seus repos, trocar de repo) | ❌ | P0 |
| **Login / auth (GitHub OAuth)** | ❌ | P0 |
| **Criar / importar repo** ("one-command GitHub import") | ❌ | P0 |
| **Inbox de Atenção** (decisões que precisam de você — aprovar land, resolver union-fail) — *o forte; ≠ observabilidade-ao-vivo, que é local* | ❌ | P1 |
| Página de org/owner | ❌ | P2 |
| Busca global | ❌ | P2 |
| Perfil / settings de conta | ❌ | P1 |

## H. Cross-cutting (sistema/UX)
| item | status | prio |
|---|---|---|
| **Cmd-K palette** (conjunto completo de comandos + busca) | 🟡 (visual) | P0 |
| Atalhos de teclado em tudo (j/k, etc.) | 🟡 | P1 |
| **Updates ao vivo** (htmx/SSE pra progresso do union-test, fila) | ❌ | P1 |
| Empty states, loading, erros | ❌ | P0 |
| Responsivo / mobile | ❌ | P2 |
| Acessibilidade (a11y, foco, aria) | ❌ | P1 |
| Toasts / confirmações | ❌ | P1 |

## I. Backend / dados (pra deixar de ser mockup)
| item | status | prio |
|---|---|---|
| **Crate `githugr`** (axum + maud + htmx + Alpine + Tailwind) — scaffold | ❌ | P0 |
| **`Provider`** (read-layer sobre refstore/proto/checks/queue/ledger/mirror) | ❌ | P0 |
| Auth/sessão (OAuth GitHub) | ❌ | P0 |
| **Write actions** (land/approve/comment/policy) — gated, depois do read-first | ❌ | P1 |
| Wiring ao event-log vivo (DO por repo) + CoreLink CAS | ❌ | P1 (depende do P2) |
| SSE/WS pro live | ❌ | P1 |
| Renderização de diff/syntax (lib server-side) | ❌ | P0 |
| Markdown render (README, comentários) | ❌ | P0 |

---

## Os "buracos grandes" (o que mais falta, resumido)
1. **Tudo que NÃO é Landing ainda é mockup-zero:** Code/blob, **Checks**, **Security**, **Insights/Ledger**, **Settings/Política**. Só Code-home e Landing-PRs têm tela.
2. **Conversa + review humano no PR** (comentário inline, aprovar/rejeitar, interrogação) — o ciclo de review não existe ainda.
3. **Nível conta:** login (OAuth), lista de repos, criar/importar repo — sem isso não tem produto, só uma tela solta.
4. **O Ledger** (asked→done→proven) — nosso diferencial #1 de "visibilidade" e ainda não tem tela.
5. **Editor de política** (= branch protection nosso) e **config de sync** — operacional crítico.
6. **Backend inteiro:** a crate `githugr`, o `Provider` sobre as crates que já temos, auth, e o render de diff/markdown. É o que transforma os mockups em produto.
7. **Killer features além do why-blame/land-preview:** interrogação, auto-bisect, cost-rollup, replay/diff-of-minds, knowledge-layer — listadas mas não construídas.

## Sequência sugerida (waves de construção)
- **Wave 0 — fundação:** scaffold `githugr` + `Provider` + auth + render diff/markdown + Cmd-K real + empty/loading states.
- **Wave 1 — repo core:** Code home (live), blob+why-blame, diff/compare, história (git↔ledger), Landing(PRs) live + diff completo.
- **Wave 2 — o ciclo PR:** conversa/timeline, comentário inline, verdict/aprovar/rejeitar, interrogação, criar PR, union-fail resolve.
- **Wave 3 — as outras abas:** Checks (logs, bisect, hit-rate), Insights/**Ledger**, Security (atestação/supply-chain), Settings/**Política**/Sync.
- **Wave 4 — conta + import:** dashboard de repos, criar/importar, inbox de Atenção.
- **Wave 5 — killers + polish:** cost-rollup, replay/diff-of-minds, knowledge-layer, live SSE, a11y, responsivo.
