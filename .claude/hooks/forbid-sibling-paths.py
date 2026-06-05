#!/usr/bin/env python3
"""hugit session fence — makes it IMPOSSIBLE for hugit sessions/agents to
write into sibling HuGR projects (especially corelink-server, which has its
own live sessions). Owner mandate 2026-06-05: "zero interference - AT ALL".

Blocks: Edit/Write/NotebookEdit targeting a sibling path; any Bash command
that references a sibling path unless it is provably read-only.
Fail-closed: ambiguity => blocked. Exit 2 = deny (stderr shown to the model).
"""
import json
import sys

FORBIDDEN_ROOTS = [
    "/Users/gustavoschneiter/Documents/HuGR/corelink-server",
    "/Users/gustavoschneiter/Documents/HuGR/corelink-workspaces",
    "/Users/gustavoschneiter/Documents/HuGR/hugr-wallet",
    "/Users/gustavoschneiter/Documents/HuGR/HuGR-Smith",
    "/Users/gustavoschneiter/Documents/HuGR/HuGR_Arsenal",
    "/Users/gustavoschneiter/Documents/HuGR/hugr-juiceshop",
    "/Users/gustavoschneiter/Documents/HuGR/_worktrees",
    "/Users/gustavoschneiter/Documents/HuGR/techlead",
]

# A Bash command touching a forbidden path is allowed ONLY if it starts with
# one of these AND contains no mutation token. Everything else: blocked.
READONLY_PREFIXES = (
    "cat ", "ls ", "ls\t", "grep ", "rg ", "head ", "tail ", "wc ",
    "stat ", "file ", "diff ", "find ",
    "git log", "git show", "git diff", "git status", "git blame",
    "git branch -a", "git branch --list", "git worktree list",
    "git -C",  # validated against mutation tokens below
)
MUTATION_TOKENS = (
    ">", "rm ", "mv ", "cp ", "mkdir", "touch ", "sed -i", "tee ",
    "ln ", "chmod", "chown", "truncate",
    # git subcommands, space-prefixed so they match regardless of `-C <path>`
    " add ", " commit", " push", " pull", " fetch",
    " checkout", " switch ", " restore", " reset",
    " rebase", " merge ", " stash", " cherry-pick",
    " branch -d", " branch -D", " branch -m",
    " worktree add", " worktree remove", " worktree prune",
    " clean", " tag ", " remote add", " remote set", " remote remove",
    "npm ", "pnpm ", "yarn ", "cargo ", "make", "pip ",
    "python", "node ", "bash ", "sh ", "zsh ", "source ", "install",
)


def deny(msg: str) -> None:
    print(msg, file=sys.stderr)
    sys.exit(2)


def main() -> None:
    try:
        data = json.load(sys.stdin)
    except Exception:
        deny("hugit session fence: unreadable hook input - failing CLOSED.")
    tool = data.get("tool_name", "")
    ti = data.get("tool_input") or {}

    if tool in ("Edit", "Write", "NotebookEdit"):
        path = ti.get("file_path") or ti.get("notebook_path") or ""
        for root in FORBIDDEN_ROOTS:
            if path.startswith(root):
                deny(
                    "hugit session fence: writing into a sibling project is "
                    f"FORBIDDEN ({root}). hugit work never crosses other "
                    "sessions' repos - especially corelink-server (live "
                    "launch sessions). Owner mandate: zero interference."
                )

    elif tool == "Bash":
        cmd = (ti.get("command") or "").strip()
        hit = next((r for r in FORBIDDEN_ROOTS if r in cmd), None)
        if hit:
            is_readonly = cmd.startswith(READONLY_PREFIXES) and not any(
                t in cmd for t in MUTATION_TOKENS
            )
            if not is_readonly:
                deny(
                    "hugit session fence: this command references a sibling "
                    f"project ({hit}) and is not provably read-only. BLOCKED "
                    "(fail-closed). Read-only inspection (cat/ls/grep/git "
                    "log...) is permitted; any mutation is not."
                )

    sys.exit(0)


if __name__ == "__main__":
    main()
