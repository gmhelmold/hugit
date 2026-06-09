#!/usr/bin/env python3
"""hugit session fence — makes it IMPOSSIBLE for hugit sessions/agents to
write into ANY sibling HuGR project (especially corelink-server, which has its
own live sessions). Owner mandate 2026-06-05: "zero interference - AT ALL".

Design (hardened 2026-06-09 after the 7-lens audit found the old substring
denylist bypassable — `find <sib> -delete`, `git -C <sib> gc`, path-assembly):

  * DEFAULT-DENY the parent. Anything under ~/Documents/HuGR/ that is NOT under
    .../hugit/ is forbidden for Edit/Write/NotebookEdit. New siblings are blocked
    automatically — no enumeration to keep in sync.
  * Bash referencing a sibling is allowed ONLY if the WHOLE command is a single,
    composition-free, ALLOW-LISTED read-only invocation (read-only programs, or
    an allow-listed read-only `git` subcommand). Composition / redirection /
    command-substitution with a sibling reference => DENY (cannot be classified).
  * Fail-closed: any ambiguity, parse failure, or unreadable input => DENY.

Exit 2 = deny (stderr shown to the model). `--selftest` runs the audit's bypass
vectors as assertions.
"""
import json
import os
import re
import shlex
import sys

# The protected parent, in the forms a command might spell it.
_HOME = os.path.expanduser("~")
REAL_PARENT = "/Users/gustavoschneiter/Documents/HuGR/"
PARENT_FORMS = [REAL_PARENT, "~/Documents/HuGR/", "$HOME/Documents/HuGR/"]
HUGIT_SEG = "hugit"  # the ONLY child of the parent that hugit may mutate

# Read-only, side-effect-free programs (NOT find/awk/sed/xargs/tee — those write).
READONLY_PROGS = {
    "cat", "ls", "grep", "rg", "head", "tail", "wc", "stat", "file", "diff",
    "less", "more", "column", "cut", "sort", "uniq", "nl", "tr", "comm",
    "shasum", "sha256sum", "md5", "md5sum", "basename", "dirname", "realpath",
    "readlink", "du", "tree", "pwd", "echo", "true", "test", "wc",
}
# git subcommands that never mutate a repo.
READONLY_GIT = {
    "log", "show", "diff", "status", "blame", "cat-file", "ls-files",
    "ls-tree", "rev-parse", "rev-list", "describe", "for-each-ref", "shortlog",
    "name-rev", "whatchanged", "grep", "show-ref", "symbolic-ref", "merge-base",
    "var", "count-objects", "verify-pack", "cherry",
}
# Anything that signals shell composition / redirection / expansion. If a
# sibling is referenced AND any of these appear, we refuse to classify => deny.
COMPOSITION = ["&&", "||", ";", "|", "$(", "`", ">", "<", "\n", "&", "${", "$["]


def deny(msg: str) -> None:
    print("hugit session fence: " + msg, file=sys.stderr)
    sys.exit(2)


def _normalize(path: str) -> str:
    """Expand ~ and $HOME so sibling detection can't be tilde-dodged."""
    p = path.replace("$HOME", _HOME)
    if p.startswith("~/"):
        p = _HOME + p[1:]
    return p


def write_path_forbidden(path: str) -> bool:
    """Default-deny: under the HuGR parent but not under .../hugit/."""
    p = _normalize(path)
    if not p.startswith(REAL_PARENT):
        return False
    rest = p[len(REAL_PARENT):]
    seg = rest.split("/", 1)[0]
    return seg != HUGIT_SEG  # any sibling (or the bare parent) => forbidden


def sibling_refs(cmd: str):
    """Return the set of sibling paths a command literally references.

    A reference to `.../HuGR/hugit/...` is NOT a sibling. A reference to the bare
    parent (e.g. `HuGR/*`) counts as a sibling reference (could glob siblings).
    """
    refs = set()
    for form in PARENT_FORMS:
        start = 0
        while True:
            i = cmd.find(form, start)
            if i == -1:
                break
            after = cmd[i + len(form):]
            m = re.match(r"([A-Za-z0-9._-]+)", after)
            seg = m.group(1) if m else ""
            if seg != HUGIT_SEG:  # "" (bare parent) or any non-hugit child
                refs.add(form + (seg or "<bare-parent>"))
            start = i + len(form)
    return refs


def is_readonly_git(args) -> bool:
    j = 0
    # strip global options (-C <path>, -c <kv>, --git-dir=, pager flags, ...)
    while j < len(args):
        a = args[j]
        if a in ("-C", "-c"):
            j += 2
            continue
        if a.startswith(("--git-dir", "--work-tree", "--namespace")):
            j += 1
            continue
        if a in ("-p", "--paginate", "--no-pager", "--no-replace-objects",
                 "--bare", "--literal-pathspecs"):
            j += 1
            continue
        if a.startswith("-"):
            j += 1
            continue
        break
    if j >= len(args):
        return False
    sub, rest = args[j], args[j + 1:]
    if sub in READONLY_GIT:
        return True
    # subcommands that are read-only only in listing/query modes:
    if sub == "branch":
        write = ("-d", "-D", "-m", "-M", "--set-upstream-to", "-u",
                 "--edit-description", "--unset-upstream", "-c", "-C", "--move")
        return not any(f in rest for f in write)
    if sub == "tag":
        return any(f in rest for f in ("-l", "--list")) or (
            len(rest) == 0)
    if sub == "config":
        return any(f in rest for f in ("--get", "--get-all", "--get-regexp",
                                       "--list", "-l")) and not any(
            f in rest for f in ("--add", "--unset", "--unset-all",
                                "--replace-all", "--remove-section"))
    if sub == "remote":
        return len(rest) == 0 or rest[0] in ("-v", "--verbose", "show",
                                             "get-url")
    if sub == "worktree":
        return len(rest) >= 1 and rest[0] == "list"
    if sub == "reflog":
        return len(rest) >= 1 and rest[0] == "show"
    if sub == "stash":
        return len(rest) >= 1 and rest[0] in ("list", "show")
    if sub == "submodule":
        return len(rest) >= 1 and rest[0] in ("status", "foreach")
    return False


def bash_is_readonly(cmd: str) -> bool:
    """True only if the whole command is a single allow-listed read-only call."""
    if any(tok in cmd for tok in COMPOSITION):
        return False  # composition/redirection/expansion with a sibling => deny
    try:
        toks = shlex.split(cmd)
    except ValueError:
        return False  # unbalanced quotes etc. => fail-closed
    i = 0
    while i < len(toks) and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", toks[i]):
        i += 1  # skip leading NAME=VALUE assignments
    if i >= len(toks):
        return True  # assignments only, no command runs
    prog = toks[i]
    if prog == "git":
        return is_readonly_git(toks[i + 1:])
    return prog in READONLY_PROGS


def evaluate(tool: str, ti: dict):
    """Return None to allow, or a deny-reason string."""
    if tool in ("Edit", "Write", "NotebookEdit"):
        path = ti.get("file_path") or ti.get("notebook_path") or ""
        if write_path_forbidden(path):
            return (f"writing into a sibling HuGR project is FORBIDDEN ({path}). "
                    "Only paths under .../HuGR/hugit/ are writable. Default-deny.")
        return None
    if tool == "Bash":
        cmd = (ti.get("command") or "").strip()
        refs = sibling_refs(cmd)
        if not refs:
            return None  # no sibling referenced => hugit-local, allow
        if bash_is_readonly(cmd):
            return None  # single allow-listed read-only invocation, allow
        return (f"this command references a sibling project ({sorted(refs)}) and "
                "is not a single composition-free read-only invocation. BLOCKED "
                "(fail-closed). Allowed: a lone read-only program (cat/ls/grep/"
                "diff/...) or an allow-listed read-only git subcommand (log/show/"
                "status/...); never a mutation, pipe, redirect, &&, ;, or $(...).")
    return None


def main() -> None:
    try:
        data = json.load(sys.stdin)
    except Exception:
        deny("unreadable hook input - failing CLOSED.")
    reason = evaluate(data.get("tool_name", ""), data.get("tool_input") or {})
    if reason:
        deny(reason)
    sys.exit(0)


def _selftest() -> None:
    SIB = REAL_PARENT + "corelink-server"
    deny_cases = [
        ("Bash", {"command": f"find {SIB} -delete"}),
        ("Bash", {"command": f"git -C {SIB} gc"}),
        ("Bash", {"command": f"git -C {SIB} update-ref refs/heads/x HEAD"}),
        ("Bash", {"command": f"git -C {SIB} reflog expire --all"}),
        ("Bash", {"command": f"git -C {SIB} branch -D main"}),
        ("Bash", {"command": f"cat ok.txt && rm -rf {SIB}/x"}),
        ("Bash", {"command": f"R={SIB}; rm -rf \"$R\""}),
        ("Bash", {"command": f"R={SIB} rm -rf \"$R\""}),
        ("Bash", {"command": f"echo hi > {SIB}/poison"}),
        ("Bash", {"command": f"sed -i '' s/a/b/ {SIB}/f"}),
        ("Bash", {"command": f"cat {SIB}/x | tee {SIB}/y"}),
        ("Bash", {"command": f"python evil.py # {SIB}"}),
        ("Edit", {"file_path": f"{SIB}/src/lib.rs"}),
        ("Write", {"file_path": "~/Documents/HuGR/hugr-wallet/x"}),
        ("Write", {"file_path": "$HOME/Documents/HuGR/techlead/x"}),
        ("Edit", {"file_path": REAL_PARENT + "newsibling/y"}),
    ]
    allow_cases = [
        ("Bash", {"command": f"git -C {SIB} log --oneline -5"}),
        ("Bash", {"command": f"git -C {SIB} status"}),
        ("Bash", {"command": f"cat {SIB}/Cargo.toml"}),
        ("Bash", {"command": f"grep -rn foo {SIB}/src"}),
        ("Bash", {"command": f"git -C {SIB} branch --list"}),
        ("Bash", {"command": "cargo test -p hugit-checks --locked"}),
        ("Bash", {"command": f"grep foo {REAL_PARENT}hugit/crates/x.rs"}),
        ("Edit", {"file_path": REAL_PARENT + "hugit/crates/x.rs"}),
        ("Write", {"file_path": _HOME + "/.hugit/known_hosts"}),
        ("Bash", {"command": "ls /tmp"}),
    ]
    fails = []
    for tool, ti in deny_cases:
        if evaluate(tool, ti) is None:
            fails.append(("SHOULD DENY", tool, ti))
    for tool, ti in allow_cases:
        if evaluate(tool, ti) is not None:
            fails.append(("SHOULD ALLOW", tool, ti))
    if fails:
        for f in fails:
            print("FAIL:", f, file=sys.stderr)
        sys.exit(1)
    print(f"fence self-test OK ({len(deny_cases)} deny + {len(allow_cases)} allow)")


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        _selftest()
    else:
        main()
