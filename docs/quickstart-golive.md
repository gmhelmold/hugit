# hugit go-live: local hook-first workflow

Use normal Git. hugit observes local facts and reports coverage honestly.

```sh
# New repository
hugit setup
mkdir ~/my-project && cd ~/my-project
git init
git add . && git commit -m "initial commit"
git checkout -b feature/example
git push -u origin feature/example
hugit health

# Existing repository instead
cd /path/to/existing-repo
hugit attach
git commit -m "normal Git commit"
git checkout main
git push
hugit health
```

Runtime state: `<git-common-dir>/hugit/event-log.json`. It is outside tracked
worktree state and shared by linked worktrees. `attach` preserves foreign hooks.

Health labels facts as **observed locally**, **attempted push**, **explicit declaration**, or **unsupported**. An attempted push is not remote confirmation.

`hugit capture` is internal/hook-only. Do not call it in normal onboarding.
Intent remains one explicit declaration or a separately contracted trusted
adapter.

```sh
hugit watch --log <git-common-dir>/hugit/event-log.json --class git-activity
hugit export --log <git-common-dir>/hugit/event-log.json --out ./backup
```

CI/runner execution, remote hosting, identity, and `hugit serve` are outside
this local workflow.
