# hugit go-live runbook — the git-local forge, first-tenant

> The exact sequence that takes a real repo to "GO-LIVE" on hugit — the
> complete capture → why → PR → land loop, em git **ou** jj, zero server, zero
> account, zero CoreLink in the default path. Every step is journey-proven
> (named tests in §7); nothing herein is a stub or a plan.

## 0. What "go-live" means (and what it does not)

- **YES — a live working forge on a real repository:** the agent orchestrator (ou
  você) trabalha normally (git cru, or jj cru) e hugit observes silently in
  the background (hooks or MCP tool), records the what to the git graph,
  answers provenance (`why`), bundles captured commits into PRs, and lands
  them via the local queue — all local, all verifiable.

- **NO — CI exec / runner fabric, multi-tenant identity, server hosting:**
  essas live in sibling/infra lanes (corelink-runners;githugr surface;`hugit
  serve` out of v1). hugit records the **demand**; it does not fabricate the
  execution (honest).

## 1. Build + init

```sh
git clone https://github.com/gmhelmold/hugit.git && cd hugit
cargo build --release
./target/release/hugit --version

# In YOUR project (new dir): the full git-proximate ceremony;
# in an existing git repo: only the hugit layer (never touches .git):
cd ~/my-project
~/hugit/target/release/hugit init
# → .git/ (if absent) + .hugit/ + .hugit/log.json + the 4 silent git hooks
#     post-commit / post-checkout / pre-push / post-merge
```

> `hugit init` is exercised via the library entry point (X5 no-shadow law; git
> init exists). Behavior is identical.

## 2. Work normally — git hooks capture silently

Run git **as usual**. Every act is captured in the background by a silent hook —
no wrapper, no new tool, no friction:

| You run | The captured `ref.update` payload |
|---|---|
| `git commit -m "feat: x"` | `{ref, branch, target, files}` |
| `git checkout -b feat/agent` | `{checkout:true, from, to, branch}` |
| `git push origin feat/agent` | `{attempt:true, refspecs, shas}` |
| `git merge feat/agent` | `{merged_from, target}` |

**Silent contract (hard guarantees):** a hook never fails/blocks git (exit 0
always, even if hugit is missing; async background child; prints nothing;invents
nothing — the events are the raw frozen `ref.update` kind, honest payloads.

Verify the trace: `hugit watch --log .hugit/log.json --class git-activity`,
`hugit fleet --log .hugit/log.json`, `hugit ledger --log .hugit/log.json`.

##  3. jj / agent-layer — MCP tool `capture`

jj fires **no** git hooks (`jj describe`+`jj git export` write refs directly`. So an
LLM tool-calling agent records its activity using the hugit-mcp `capture` tool —
**in place of** the raw jj act, same seam the silent hooks use:

| jj act | `capture` kind | oid / qualifiers |
|---|---|---|
| commit: `jj describe -m <m>` + `jj git export` | `commit` | `oid` = `commit_id` (the git oid) |
| checkout: `jj new` (=`git checkout -b` analog.| `checkout` | `oid`=new `commit_id`, `from`=prior,`branch`=`change_id.short()` |
| push: any push attempt | `push-attempt` | `shas` (the local shas) |
| merge-ish: `jj squash` (≈`git merge --squash`) | `merge` | `oid`=result, `from`=source |

The git oid lire: `jj log -r @ --no-graph -T 'commit_id ++ "\n"'`. Always pass
`verify:true` → the tool reads the SAME canonical log backand returns the landed
`seq` + `event_hash` — a **confirmed** capture, never a bare "dispatched"
promise (a dispatched-but-unconfirmed capture is a tool error, fail-closed|.



##  4. Answer "why did this file evolve"

```sh
hugit why --log .hugit/log.json --path src/lib.rs
#   → the captured commit that touched the path (origin)

hugit why --walk --log .hugit/log.json --path src/lib.rs
#   → the FULL captured chain, newest-first, every link a distinct raw event
#     (seq/hash/author/recorded_at/oid/branch/qualifiers/files — never fused)
```

## 5. PR + land — captured commits are members

```sh
hugit pr open --pr PR-1 --campaign my-campaign \
  --author-kind orchestrator --run-id run-1 \
  --commit <captured-commit-oid> \
  [--intent <intent-id>] \
  --log .hugit/log.json

hugit pr queue --pr PR-1 --log .hugit/log.json
hugit land queue --campaign my-campaign --log .hugit/log.json
#   → {"verdict":"green", "landed":["PR-1"], ...}
hugit pr show --pr PR-1 --log .hugit/log.json
```

A captured commit oid (or a pushed sha — a `git push` is captured-commit
proof, the pre-push hook records it under `shas`)) is accepted as an **external
member** (`commit_ids`)— never forged into an intent; an unseen sha stays
`commit_not_found`. Commit-only PRs (no intents) land fully via the queue.



## 6. Exit proof — zero lock-in

```sh
hugit export --log .hugit/log.json --out ./backup
# → ./backup/repo.git  (a REAL git repo)
#   ./backup/export.json          (the full JSON envelope)
#   ./backup/redaction-manifest.json
```

##  7. Go-live checklist (verification, all on `main`

```sh
cargo test -p hugit-mcp                                    # 45/45 (10 capture cases)
cargo test -p hugit-cli --test acceptance_capture         # 13 hook journeys
cargo test -p hugit-cli --test acceptance_capture_jj_checkout_merge   # 1 (REAL jj,MCP e2e;SKIPs loudly sem jj)
cargo test -p hugit-cli --test acceptance_gitlocal_journey # 4 (init local, check memo, export)
cargo test -p hugit-cli --test acceptance_land_queue       # land queue (commits-only PRs land)
```

Manual smoke on your real repo:

```sh
hugit why --log .hugit/log.json --path <your-file>      # resolves
hugit watch --class git-activity --log .hugit/log.json   # trace classified
hugit pr show --pr PR-1 --log .hugit/log.json          # terminal PR state
```

A green run of these = the loop is live on your machine.** No claim is made
beyond local, hermetic-proven behavior (see honest scope).

##  8. Honest scope — what is NOT on this road

- **CI / runner exec** (`ws`/`dispatch`) → `corelink-runners` (separate project,
  needs the runner fabric; hugit records the demand only).
- **`hugit serve`** — optional forge-host binary, out of v1 (a separate surface).
- **Multi-tenant / identity / GitHub App mirror** — forge-surface/githugr
  infra, owner-gated (not a hugit build gap)。

---

*Journey-proven on `main`: `acceptance_capture` (13),`acceptance_capture_jj_checkout_merge`
(REAL jj;MCP tool e2e),`acceptance_gitlocal_journey` (4),`acceptance_land_queue`,
and hugit-mcp 45/45.