# hugit quickstart: setup, normal Git, health

Normal onboarding: `setup` or `attach`, then normal Git, then `health`.
Hooks observe local Git facts; they never replace Git workflow.

## New repository

```sh
hugit setup
mkdir my-project && cd my-project
git init
git add . && git commit -m "initial commit"
git checkout -b feature/example
git push -u origin feature/example
hugit health
```

`setup` configures Git's global `init.templateDir`. Future `git init` copies
hugit-owned hooks. Runtime state is
`<git-common-dir>/hugit/event-log.json`, outside tracked worktree state and
shared by linked worktrees.

## Existing repository

```sh
cd /path/to/existing-repo
hugit attach
git commit -m "normal Git commit"
git checkout main
git push
hugit health
```

`attach` preserves foreign hooks. Run `hugit attach --preview` before writes
when existing tooling needs inspection.

## Truth labels

- **observed locally**: hook saw local Git fact.
- **attempted push**: pre-push hook saw local attempt, not remote success.
- **explicit declaration**: user or trusted adapter declared intent.
- **unsupported**: hugit has no fact for coverage.

`hugit capture` is internal/hook-only. Normal workflow never invokes it
manually. Intent remains one explicit declaration or a separately contracted
trusted adapter.

## Inspect local facts

```sh
hugit watch --log <git-common-dir>/hugit/event-log.json --class git-activity
hugit fleet --log <git-common-dir>/hugit/event-log.json
hugit ledger --log <git-common-dir>/hugit/event-log.json
```

Hooks never block Git, print nothing, and record no invented intent. CI/runner
execution, `hugit serve`, accounts, and CoreLink credentials are outside this
local workflow.
