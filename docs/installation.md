# Installing hugit

hugit is a single static CLI binary — no service, no account, no CoreLink. It
works wherever git works.

## Latest release

Download the archive for your platform from the **GitHub Releases** page:
<https://github.com/gmhelmold/hugit/releases>

| Platform | Architecture | File |
|---|---|---|
| Linux | x86-64 | `hugit-<ver>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS | Apple Silicon (arm64) | `hugit-<ver>-aarch64-apple-darwin.tar.gz` |
| macOS | Intel (x86-64) | `hugit-<ver>-x86_64-apple-darwin.tar.gz` |
| Windows | x86-64 | `hugit-<ver>-x86_64-pc-windows-msvc.zip` |

## Install (Linux / macOS)

```sh
# pick your version, e.g. v0.1.0
VERSION=v0.1.0

# Linux
curl -LO "https://github.com/gmhelmold/hugit/releases/download/${VERSION}/hugit-${VERSION#v}-x86_64-unknown-linux-gnu.tar.gz"
tar -xzf "hugit-${VERSION#v}-x86_64-unknown-linux-gnu.tar.gz"

# macOS (Apple Silicon)
# curl -LO "https://github.com/gmhelmold/hugit/releases/download/${VERSION}/hugit-${VERSION#v}-aarch64-apple-darwin.tar.gz"
# tar -xzf "hugit-${VERSION#v}-aarch64-apple-darwin.tar.gz"

# put it on your PATH
sudo mv hugit /usr/local/bin/
hugit -V
```

## Install (Windows)

1. Download the `.zip` for `x86_64-pc-windows-msvc`.
2. Unzip — it contains `hugit.exe`.
3. Put `hugit.exe` somewhere on your `PATH` (e.g. a folder you add to PATH).
4. Open a new terminal and run `hugit -V`. (Unix-flavoured git hooks work out
   of the box because Git for Windows ships its own `sh`.)

## Install from source (contributors)

```sh
git clone https://github.com/gmhelmold/hugit.git
cd hugit
cargo build --release
# binary is at target/release/hugit
```

## Enable the hooks (one-time boot ceremony)

hugit works by listening at checkout/commit/push time. **`hugit setup`** does
the whole ceremony once per machine — it points git's global
`init.templateDir` at a hugit template, so **every future `git init` ships the
hooks automatically**. The hooks lazy-boot: the first `git commit` / checkout
in a fresh repo creates the `.hugit/log.json` log by itself.

```sh
hugit setup
```

For an **existing** repository, install hooks without changing its Git data:

```sh
hugit setup --repo /path/to/repo
```

## Quickstart (a repo of your own)

```sh
cd your-repo
# make an agent task and a milestone
hugit campaign open --campaign v1 --charter "first release" --owner you
hugit intent new --charter "add rate limit" --acceptance "tests green" --campaign v1

# work normally (git commit) — the hooks capture it into .hugit/log.json
# then bundle it into a PR + land it
hugit pr open --pr PR-1 --campaign v1 --author-kind human --principal user:you --commit HEAD
hugit pr queue --pr PR-1
hugit land queue
```

The canonical log (`.hugit/log.json`) is versioned, so it **travels with the
repo**: `git push` / `git pull` syncs the history between agents sharing a
repo — no server involved.

## What gets installed

- One binary: `hugit` (the CLI — the whole product).
- An optional template dir at `~/.config/hugit/template` + one global git
  config key (`init.templateDir`) created by `hugit setup`.

`hugit setup --repo` writes only missing hugit hooks. Existing non-hugit hooks
stay byte-identical and are reported as conflicts.
