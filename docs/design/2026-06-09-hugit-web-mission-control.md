# hugit-web — Mission Control & the forge UI (design)

> 2026-06-09 · owner-directed: build the actual forge interface — "a better
> GitHub" on the infra we already have, GitHub kept in sync as the escape hatch.
> Stack decided: **Rust + axum + maud (compile-time HTML) + htmx + Tailwind**
> (Rust-centric, minimal JS, keeps the single-stack/impeccable-repo discipline).
> Scope decided: **all of it** — differentiated surfaces + table-stakes + a
> vertical slice — because **the data already exists**; this is a read/write UI
> over crates we've already built, not a new forge.

## The thesis of this build

The hard 80% — VCS, event-log, intents, memoized CI, GitHub sync, provenance —
is **done** (14 crates). A "new GitHub" from zero spends years there; we don't.
`hugit-web` is a thin, beautiful **read-layer first** over that data, plus gated
writes later. Where GitHub can't follow (intent + context + memoization) we are
**better by construction**; for the long tail (settings, discussions, projects)
the user "fica à vontade no GitHub" via the live mirror — absorbed by phase, not
all at once (whitepaper §10/§12).

## Stack & crate

- New workspace crate **`crates/hugit-web`** (`[[bin]] hugit-web`): `axum` +
  `tokio` server; **`maud`** for type-safe compile-time HTML (Rust-pure, no
  template-injection class of bug); **htmx** (CDN) for partial swaps / live
  updates; **Tailwind** (CDN for MVP → built stylesheet later) for the look.
  Reuses the **HuGR house design system** (the dark/indigo/cyan theme from the
  whitepaper + pricing pages) so it's beautiful and on-brand day one.
- **Read-only first.** Every screen reads; mutating actions (approve/reject/land/
  policy) come after, behind the policy engine + auth, clearly gated.
- **Data access:** a `Provider` trait the routes read through. MVP impl reads the
  **local event-log / refstore / CAS + fixtures**; the live impl binds to the
  per-repo Durable Object event-log + CoreLink CAS when **P2** lands. (Honest: the
  UI runs on real data shapes now; it goes end-to-end live with P2.)

## Screen inventory → route → data source (the map)

### A. Differentiated surfaces — where GitHub cannot follow (build FIRST)
| Screen | Route | Reads from |
|---|---|---|
| **Ledger** (asked→done→proven, by campaign) | `/r/:repo/ledger` | `hugit-ledger` + `hugit-refstore::intent` |
| **Mission Control** (campaigns → intents → agents; progress/risk/cost) | `/r/:repo` (home) · `/fleet` | `hugit-refstore` + `hugit-ledger` (D5 fleet) + `hugit-queue` |
| **Attention queue** (policy × blast-radius × verdict-confidence) | `/r/:repo/attention` | `hugit-contracts::AttentionRank` (D9) + `hugit-policy` |
| **Intent detail** (charter · **context snapshot** · trajectory · diff · evidence · verdicts) | `/r/:repo/intent/:id` | `hugit-refstore::intent` + `IntentSidecar` + event-log |
| **CI / memoized checks** (hit-rate, cache-hit ⇒ 0, affected) | `/r/:repo/checks` | `hugit-checks` (`CheckResult`) |
| **Landing queue** (union-testing batches, lanes) | `/r/:repo/queue` | `hugit-queue` |
| **Interrogation** ("where does this touch money?") | `/r/:repo/intent/:id/ask` (htmx) | `hugit-diag` (why/impact) + intent context |
| **Provenance** (`why` / `impact`) | `/r/:repo/why` · `/impact` | `hugit-diag` (D10) |
| **Sync status** (GitHub mirror, bidirectional) | `/r/:repo/sync` | `hugit-mirror` |

### B. Table-stakes — clean, modern, fast (build right after)
| Screen | Route | Reads from |
|---|---|---|
| Repo browser / file tree | `/r/:repo/tree/:ref/*path` | `hugit-proto` (objects) + `hugit-refstore` (refs) |
| Code view (syntax-highlit) | `/r/:repo/blob/:ref/*path` | `hugit-proto` blob |
| Diff view | `/r/:repo/diff/:a..:b` | `hugit-proto` |
| PR list / PR-as-intent | `/r/:repo/prs` | `hugit-refstore::intent` (PROPOSED/landable) |
| Commit/history (git altitude) | `/r/:repo/commits/:ref` | `hugit-proto` + `hugit-refstore` |
| Branches / refs | `/r/:repo/refs` | `hugit-refstore` |
| Global / repo search | `/search` | (deferred — semantic index is research-grade) |

### C. Linguiça — DON'T build (mirror-link or drop entirely)
Explicitly out of scope. These are either GitHub's social graph ("not our war"),
low-value vanity, or pure filler. We render a clean **"abrir no GitHub"** deep-link
where it makes sense, and **drop the rest** — no screen, no code.
| Feature | Verdict | Why it's linguiça |
|---|---|---|
| **Wiki** | drop / mirror-link | docs live in the repo / knowledge layer; nobody migrates for a wiki |
| **Discussions** | mirror-link | GitHub's social graph — drained, not stormed |
| **Projects / boards** (beyond campaigns) | mirror-link | **campaigns** already replace this; generic kanban is filler |
| **Sponsors · stars · followers · social** | never build | not our war (§10); the community moves stars, not us |
| **Gists** | drop | scratch-paste; zero relevance to a forge for fleets |
| **GitHub Pages / hosting** | drop | unrelated product |
| **Marketplace / Apps directory** | drop | not our surface |
| **Profile vanity** (contribution graph, achievements) | drop | the **ledger + fleet** are the real signal; vanity adds nothing |
| **Insights vanity** (traffic, commit-activity charts) | drop | keep real fleet/cost/CI metrics; skip the vanity graphs |
| **GitHub-style notifications** | replaced | the **attention queue** is the inbox — strictly better |
| **Releases UI · Packages registry UI** | mirror-link (for now) | packages→CAS registry is a *later* absorption, not MVP |
| **Org / team / settings admin** | mirror-link | heavy, low-differentiation; the GitHub App already handles install/perms |

### D. Deferred — REAL, not linguiça (later wave, not now)
Not filler — genuinely valuable, just not MVP: **code/semantic search** (the index
is research-grade, 8–14 EM), **write-path actions** (approve/reject/land/policy —
gated, after read-first), **auth / multi-tenant** (GitHub OAuth), **packages-as-CAS-
registry**. These get built; they're sequenced after the core, not dropped.

## MVP sequencing (vertical slice → fan out)

1. **Spine (slice that proves the whole thesis on one screen):** repo home
   (Mission Control mini) + Ledger + one Intent detail (with context snapshot) +
   CI status — all wired to a `Provider` over fixtures/local event-log. The app
   shell, nav, theme, htmx live-swap, deploy-able binary.
2. **Differentiated wave (A):** attention queue, landing queue, fleet dashboard,
   interrogation, provenance, sync status.
3. **Table-stakes wave (B):** file tree, code view, diff, PR list, history, refs.
4. **Long tail (C):** mirror deep-links everywhere they belong.

## Non-goals / honest limits (this build)

- Not auth/multi-tenant yet (local/single-tenant MVP; GitHub OAuth in the GA wave).
- Not the write-path actions yet (read-first; approve/land/policy come gated, after).
- Not "literally everything GitHub has" — that's phased absorption; the mirror
  covers the tail meanwhile.
- Runs on local/fixture data until P2 wires the live event-log + CoreLink CAS.

## Quality bar

Same as the rest: `fmt + clippy --workspace --all-targets --locked -D warnings +
test --locked + audit/deny` green; new deps (axum/tokio/maud) hoisted into
`[workspace.dependencies]`, exact-pinned, added to `deny.toml` allow-list; routes
are oracle-tested (a route returns the expected data shape; htmx partials render).
