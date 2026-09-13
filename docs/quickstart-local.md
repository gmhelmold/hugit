# hugit quickstart: local, Git-native

hugit runs locally with normal Git. No account, server, or CoreLink needed.

## New repository

```sh
hugit setup
mkdir ~/my-project && cd ~/my-project
git init
git add . && git commit -m "initial commit"
git checkout -b feature/example
git push -u origin feature/example
hugit health
```

## Existing repository

```sh
cd /path/to/existing-repo
hugit attach
git commit -m "normal Git commit"
git checkout main
git push
hugit health
```

Runtime state is `<git-common-dir>/hugit/event-log.json`, outside tracked
worktree state and shared by linked worktrees. `attach` preserves foreign hooks.

`health` reports **observed locally**, **attempted push**, **explicit declaration**, and **unsupported**. A push attempt never claims remote success. `hugit capture` is internal/hook-only; normal onboarding never runs it. Intent is one explicit declaration or a separately contracted trusted adapter.

## Local commands

```sh
hugit check run --def ad-hoc --cmd "echo hello" --store
hugit export --log <git-common-dir>/hugit/event-log.json --out ./backup
hugit ledger --log <git-common-dir>/hugit/event-log.json
```

CI/runner execution and `hugit serve` are separate products. hugit records
demand locally; it does not fabricate execution or remote proof.
