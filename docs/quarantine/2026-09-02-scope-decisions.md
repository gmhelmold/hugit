# QUARANTINE — scope decisions & superseded work (2026-09-02)

> Purpose: PRESERVE the reasoning, decisions, and code-behavior facts about
> hugit's scope (git-local CLI), WITHOUT deleting or changing any code that
> compiles. Nothing here is deleted; everything is referenced so a future
> session can re-incubate any item. This is a docs/decisión-only quarantine.

---

## 1. Owner decision (verbatim intent)

- **"eu quero que seja igual o git. CI e outra coisa, e corelinkrunners, e
  projeto separado. O hugit e pra trabalhar onde o git trabalha"**
- Canonical record: `docs/review/2026-09-02-hugit-cli-local-catalog-handoff.md`
- Baseline: `main 15f34b7` (fresh clone of `github.com/gmhelmold/hugit`).

---

## 2. The one factual clarification that drives everything

The CLI's runtime is **already 100% local by default** — this is not a goal, it
is the current behavior (verified at `main 15f34b7`):

| Seam | Local default (in the code) | CoreLink/CI option (off by default) |
|---|---|---|
| Action Cache | **`FileAc`** — file-backed local AC (`crates/hugit-cli/src/checks/run.rs:29`, `:1265` `Local(FileAc)`; impl in `crates/hugit-checks/src/client/ac.rs`) | `HttpAcClient` over CoreLink (`ac.rs:468`), swapped via the `ActionCache` trait (`run.rs:32`) |
| Check execution | Local hermetic executor (child process, `run.rs`) | remote runner fabric (CoreLink) — the disclosed P2 runner-sandbox seam (`run.rs:135,355`) |
| Land | `land/mod.rs` — real union-test + bisect + memoize **over a local log** (no network) | `pr land --dispatch` → `dispatch_check` on the fabric (`crates/hugit-cli/src/pr/dispatch.rs:110`) |
| Fleet/diag | pure log projections (`key.rs` reads `check.recorded` only), zero network | — |

**Consequence:** grepping the **CLI runtime** (non-test) for `HttpAcClient` /
`HUGIT_CORELINK_*` / `fabricd` returns **zero hits**. No runtime path requires
CoreLink. CI concepts live as *optional swap points*, not as the default.

---

## 3. The 9-WP scope suggestion that was SUPERSEDED (parked, not deleted)

A prior same-day plan (`docs/plan/2026-09-02-go-live-v2-parallel.md` — removed
from the tree; this entry preserves its content outline) proposed creating
**local sub-providers** for a self-hosted forge: `hugit-ac-local`, `hugit-cas-local`,
docker/podman runner, local identity, `providers.toml` migration, cost-honesty,
boot-reconcile. **The owner rejected that direction** (hugit = git-local CLI, not
a self-hosted forge; CI compute belongs to the separate `corelink-runners`).

That plan created worktrees + branches `wp/05…wp/17` with real commits; **all
were removed** (`git worktree remove --force` + `git branch -D`). The commits
are referenced here for archaeology; they are unreachable in the normal ref
graph and are NOT part of the product.

| WP (superseded) | What it built (committed, then removed) | Why quarantined |
|---|---|---|
| WP-05 | `hugit-contracts::provider` (`ProviderKind`, `ProvidersToml`) | decoupling-for-hosting scope, rejected |
| WP-10/11 | `LocalAcFile`, `LocalAcRedis` behind `ActionCache` | same |
| WP-12 | `hugit-serve/src/cas_local.rs` (fs CAS) | self-hosted forge scope, rejected |
| WP-13 | `hugit-checks/src/runner/local_exec.rs` (docker exec) | docker runner = CI, owner-decided out |
| WP-14 | `hugit-cli/src/ident_local.rs` (SSH/GPG resolve) | identity/hosting scope |
| WP-15 | `hugit provider migrate` (env → toml) | provider config = hosting scope |
| WP-16 | cost-honesty `-` render in insights | githugr/CoreLink forge scope |
| WP-17 | boot-reconcile | multi-tenant forge scope |

> Note: WP-10's `LocalAcFile` diff (new file `crates/hugit-checks/src/client/ac_local_file.rs`)
> was good engineering and harmless to the local-only direction, but it was
> committed under the wrong-scope plan and removed with it. If the owner later
> wants a file-backed AC beyond the existing `FileAc` (already local), that code
> can be re-created from the git reflog of the deleted branch — nothing is lost.

---

## 4. What the git-local scope KEEPS on the 24-verb catalog

All of these are verified running standalone (no server, no CoreLink) on `main`
and are in scope: `init why impact tournament export campaign intent issue pr
land meta queue verdict undo policy note ledger fleet watch symbol ctx review`.

Candidate reconsideration (per the handoff, "CI é outra coisa"):
- `land --dispatch` (fabric runner) — the one place that invokes the remote
  fabric; `land` itself (union-test over the log) stays local.
- `check --store` / AC-CoreLink — the `HttpAcClient` swap stays as an optional
  seam, NOT the default; `check run` local is already the default.

## 5. OPEN OWNER DECISIONS (for the next session)

1. `hugit init`: should it wrap `git init` (git-proximate ceremony) or just
   register inside an existing `.git`? Documented in handoff §Gaps #1.
2. `check` verb: keep local-only (memo on your repo, execute on your machine)
   or also re-sit as CI? If "CI = anything compute", check + diag re-sit to
   corelink-runners.
3. `hugit serve` (smart-HTTP forge host): in or out of scope for the git-local
   CLI? The handoff flags it as needing re-consent.

---

## 7. Refs preserved

- Handoff (decision record): `docs/review/2026-09-02-hugit-cli-local-catalog-handoff.md`
- This file intentionally does not DELETE anything; it quarantines decisions to
  a documented shelf. Re-incubation is a branch-create + reflog restore, never
  a revert of behavior.