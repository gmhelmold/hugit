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

hugit listens at checkout/commit/push time. **`hugit setup`** does whole
ceremony once per machine: it points Git's global `init.templateDir` at a hugit
template, so every future `git init` ships hooks automatically. Hooks lazy-boot
runtime state at `<git-common-dir>/hugit/event-log.json`.

```sh
hugit setup
```

For an **existing** repository, run `hugit attach`. It installs missing
hugit-owned hooks and preserves foreign hooks. Then use normal Git and run
`hugit health`; partial coverage remains explicit.

## Quickstart (a repo of your own)

```sh
hugit setup
git init my-repo
cd my-repo
git add . && git commit -m "initial commit"
git checkout -b feature/example
git push -u origin feature/example
hugit health
```

`health` distinguishes **observed locally**, **attempted push**, **explicit declaration**, and **unsupported**. Hook capture is internal/hook-only; do not invoke `hugit capture` during normal onboarding. Intent remains one explicit declaration or a separately contracted trusted adapter.

## What gets installed

- One binary: `hugit` (the CLI — the whole product).
- An optional template dir at `~/.config/hugit/template` + one global git
  config key (`init.templateDir`) created by `hugit setup`.

Nothing else writes outside the repo; `hugit setup` never touches an
existing repo's hooks.
