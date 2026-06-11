# hugit — the absorption map (v0.1)

> **Owner decision (2026-06-05):** hugit + CoreLink are **the foundation** to
> take GitHub head-on — the end-state is to **absorb everything git and GitHub
> offer, modernized**. The compat ladder (mirror → landing layer → forge →
> authoritative) remains the *route*; this document declares the *destination*:
> every capability, mapped, with an honest verdict each.
>
> **Owner:** Gustavo Schneiter · **Drafted:** 2026-06-05 · status: REFINING

## The verdict taxonomy

| Tag | Meaning |
|---|---|
| 🟢 **SUPERSET** | we absorb it and exceed it — ours is strictly better |
| 🔁 **TRANSFORM** | absorbed into a new primitive (the Inversions) — same job, different shape |
| 🔌 **COMPAT SHIM** | we speak their format to make migration free, while ours runs underneath |
| 🪞 **RIDE THE MIRROR** | deliberately stays on GitHub early — absorbed late, when pulled |
| ⚪ **EASY, LATER** | trivial on our stack; sequenced by demand, not difficulty |
| ⛔ **NOT OUR WAR** | we choose not to fight here (and say why) |

---

## 1. git — the version control layer

| Capability | Verdict | In hugit |
|---|---|---|
| The object model (blob/tree/commit) | 🟢 kept whole | served as the **projection** over the CAS — real git objects, real wire protocol, forever |
| All git commands | 🟢 kept whole | never shadowed (catalog A) — muscle memory + LLM corpus + ecosystem intact |
| Branches / merges | 🔁 TRANSFORM | claims at dispatch + write-time conflicts + 3-tier resolution + regenerative rebase; branches become auto-managed intent refs |
| History (`log`, `blame`, `bisect`) | 🟢 SUPERSET | two altitudes (intents ↔ commits); blame links to the *why*; bisect memoized and auto-run |
| `rebase -i` / squash / history rewriting | 🔁 TRANSFORM | **history is never rewritten — re-projected** (altitude folding); the biggest footgun is eliminated, not improved |
| Git LFS | 🟢 SUPERSET | native large blobs in the CAS, zero egress, no pointer files, no lock-in rewrite — LFS's documented hate, deleted |
| Submodules | 🔌 COMPAT SHIM | supported as-is for compat; superseded by workspace composition (a workspace can mount other repos' subtrees by hash) |
| Hooks | 🔌 COMPAT SHIM | local hooks keep working; the real mechanism is server-side policy + the event stream |
| `.gitattributes` merge drivers | 🟢 SUPERSET | three-tier resolution server-side (regenerate / AST / gated LLM) — no per-machine driver setup |
| Worktrees | 🟢 SUPERSET | `hugit ws`: <1s, CAS-deduped, claim-fenced, resumable anywhere |
| **What git never had: context** | 🆕 NEW | versioned context snapshots, journals, trajectories — the Inversion-2 layer git cannot express |

---

## 2. GitHub — code & collaboration core

| Capability | Verdict | In hugit |
|---|---|---|
| Repo hosting + web code browse | 🟢 SUPERSET | file tree identical (sacred); plus altitude reading, `why` on every line, semantic search |
| Pull Requests | 🔁 TRANSFORM | **the Intent** — charter + diff + context + evidence + verdicts, born complete; "PR" survives as the mirror's write-back format |
| Code review | 🔁 TRANSFORM | structured verdicts (machine) + interrogation sessions (human) + the attention queue; prose threads survive only on the mirror |
| Merge queue | 🟢 SUPERSET | landing queue: speculative **union testing** (A+B together), build-graph parallel landing, regen rebase — their queue can't see semantics |
| Branch protection / rulesets | 🟢 SUPERSET | policy-as-code: declarative, locally testable, per-principal (humans AND agents), fail-closed |
| CODEOWNERS | 🟢 SUPERSET | claims + policy routing — ownership by contract/target, not path glob; routes to agents too |
| Issues | 🔁 TRANSFORM | an issue = an **intent in proposed state** (charter + acceptance criteria, no code yet); the demand IS the failing acceptance suite |
| Projects / boards | 🔁 TRANSFORM | campaigns + plans + the ledger — boards generated from real state, never hand-moved cards lying about reality |
| Notifications | 🔁 TRANSFORM | the attention queue: risk-ranked, policy-driven, one item at a time — the inbox inversion |
| Wiki | 🔁 TRANSFORM | the knowledge layer: ADRs and decisions bound to intents, queryable (`hugit why`) — docs that can't drift from the code that implements them |
| Discussions | 🪞 RIDE THE MIRROR | community conversation stays on GitHub early; absorbed later as threads bound to intents/campaigns |
| Releases | 🟢 SUPERSET | emitted from intent history (changelog auto-written from charters), artifacts in CAS with attestation, zero-egress downloads |
| Code search | 🟢 SUPERSET | the semantic index: search by meaning, who-calls, contract, *why* — not just text grep at scale |
| Gists | ⚪ EASY, LATER | blobs with a URL; sequenced by demand |

---

## 3. GitHub — CI, security, supply chain

| Capability | Verdict | In hugit |
|---|---|---|
| Actions (CI) | 🟢 SUPERSET | memoized checks + affected targets + CoreLink runners — **the economic engine**: never re-verify the verified; flat pricing; your hardware never metered |
| Actions YAML / marketplace | 🔌 COMPAT SHIM | an Actions-compatible runner (execution core lives in `corelink-runners`, campaign #1 seed — runner-transfer 2026-06-10) executes existing workflows unchanged during migration; native checks-as-code is the destination |
| Status checks API / badges | 🔌 COMPAT SHIM | we emit GitHub-compatible statuses through the mirror so every ecosystem tool (badges, bots, integrations) keeps working |
| Dependabot | 🟢 SUPERSET | dep updates as speculative pre-tested landings by policy — silent when green, one task when red; the 200-PRs/week flood, deleted |
| Code scanning (CodeQL-class) | 🟢 SUPERSET | security scanners are just checks: memoized by tree-hash, affected-target scoped, results as structured findings bound to intents |
| Secret scanning | 🟢 SUPERSET | scanning + the broker: secrets never enter workspaces at all — prevention by construction, not detection after the leak |
| Attestations / SLSA / provenance | 🟢 SUPERSET | falls out of the object model for free: every artifact traces to tree-hash + check-def + model + prompt + runner — deeper provenance than GitHub can express (they can't attest *which model wrote the code under which instruction*) |
| Packages / GHCR (registry) | 🟢 SUPERSET (later phase) | a registry IS a CAS with names — content-addressed packages/containers, zero-egress pulls, cross-tenant dedup of public layers; natural CoreLink extension |
| Environments / deployments | ⚪ EASY, LATER | deployment = a policy-gated landing to an environment ref + the event stream; integrate with existing CD first |

---

## 4. GitHub — compute & AI

| Capability | Verdict | In hugit |
|---|---|---|
| Hosted runners | 🟢 SUPERSET | CoreLink campaign #1: ephemeral, cache-warm, cheap (Hetzner-class + R2), flat per-concurrency |
| Codespaces | 🟢 SUPERSET | workspaces (campaign #2): born <1s from CAS, local/remote transparent, build state included |
| Copilot / coding agent / Agent HQ | ⛔ NOT OUR WAR (deliberately) | **we are the ground, not one of the armies.** BYO-orchestrator (Claude, Codex, Devin, OpenHands) as first-class principals. GitHub bundles the model and meters it (10–50× bill shock); we make every model's agents work better — including theirs |
| GitHub Apps / API / webhooks | 🟢 SUPERSET | machine-paced API + replayable guaranteed event stream + per-principal budgets; apps become policies + principals |
| Marketplace (3rd-party apps) | 🪞 RIDE THE MIRROR | early integrations keep working through the mirror + compat statuses; native ecosystem comes after the platform earns it |

---

## 5. GitHub — the social layer (the honest section)

| Capability | Verdict | In hugit |
|---|---|---|
| Stars, followers, profiles, trending | 🪞 RIDE THE MIRROR | **GitHub's real moat is the social graph — we don't storm it, we drain it.** OSS presence stays on the mirror (stars accumulate there); the *work* happens on hugit. Absorbed only if/when the community pulls |
| Forks (social OSS model) | 🔌 COMPAT SHIM → 🔁 | mirror handles OSS fork/PR contributions seamlessly; native model: a fork is just a workspace + intent against someone else's repo (cheaper and saner) |
| Sponsors | ⛔ NOT OUR WAR | payments to OSS maintainers is a fine business — someone else's. We integrate, not compete |
| Explore / feeds | ⛔ NOT OUR WAR (for years) | discovery stays social; we win the workflow, not the feed |

---

## 6. What GitHub doesn't have at all (the new continent)

The absorption map is also an expansion map — these have **no GitHub
equivalent to absorb**:

1. **Versioned context** (snapshots, journals, trajectories, diff-minds)
2. **Claims & capability-fenced workspaces** (conflicts at dispatch; safety by construction)
3. **Regenerative rebase** (re-execute the intent; never hand-merge orthogonal work)
4. **Union testing before landing** (A+B-green, guaranteed)
5. **The attention queue** (the human inbox inverted)
6. **Interrogable changes** ("convince me this is safe")
7. **Fleet-wide flake intelligence & auto-culprit**
8. **The intent ledger** (history a human actually reads)
9. **Tournament intents** (exploration as a primitive)
10. **Model-level provenance** (which model, which prompt, whose command, what cost)

This is where "bater de frente" is won: not by matching their checklist, but
by making their checklist look like the *projection* of a richer system —
which, on hugit, it literally is.

---

## 7. Absorption sequence (tied to the route)

| Phase | What gets absorbed |
|---|---|
| **Landing layer (on GitHub)** | merge queue, review, dependabot's job, CI economics — their workflow pains monetized while they host the bytes |
| **Forge (git-protocol)** | hosting, PRs→intents, issues→intents, code browse, search, LFS, releases, branch protection→policy |
| **Parity push** | Actions shim, packages, environments, statuses/badges, wiki→knowledge, projects→campaigns |
| **The long game** | discussions, marketplace, social — by pull, never by push |

**Two standing rules:** (1) every absorption ships with its COMPAT SHIM — the
ecosystem must never notice a seam; (2) nothing is absorbed worse — if our
version of a capability isn't a 🟢 or an honest 🔁, it waits.
