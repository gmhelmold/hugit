# hugit

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

> Git-local provenance and landing records for humans and LLM agents.

Hugit observes normal Git work through hooks, records intent and review context
in a local integrity-checked log, memoizes local checks by content, and exports
portable evidence. No server, account, or CoreLink dependency exists in default
path.

## Quickstart

Download release from [GitHub Releases](https://github.com/gmhelmold/hugit/releases)
or build locally:

```sh
git clone https://github.com/gmhelmold/hugit.git
cd hugit
cargo build --release
```

Configure future repositories once:

```sh
hugit setup
git init my-project
cd my-project
git add .
git commit -m "initial commit"
hugit health
```

Attach existing repository without replacing foreign hooks:

```sh
cd /path/to/repository
hugit attach --preview
hugit attach
hugit health
```

With remote configured, normal workflow stays Git-native:

```sh
git checkout -b feature/example
git commit -am "describe change"
git push -u origin feature/example
hugit health
```

`health` reports machine-readable hook coverage. Interpret observed hook facts
as **observed locally** and `pre_push` only as **attempted push**; user claims
still require **explicit declaration**, while unobservable operations remain
**unsupported**. Missing/conflicting hooks produce **partial coverage**, never
invented confirmation.

Normal `git commit`, checkout, merge, rewrite, ref transaction, and push attempt
invoke installed hooks. Hugit writes receipts under
`<git-common-dir>/hugit/receipts/`, drains them into
`<git-common-dir>/hugit/event-log.json`, then exposes same state through read
commands.

## Observed Journey

Hugit Reproducible Evidence Report exercises selected executable against synthetic
local Git repository:

```text
git commit
    |
post-commit hook
    |
receipt -> hash-chained ref.update
    |
intent + explicit PR binding
    |
memoized check + supplied verdict/usage
    |
local PR envelope + ledger/watch/export
```

Independent verifier reopens retained repository, runs stock `git fsck`, checks
event chain, recomputes 14 selected semantic oracles, validates two refusal
records and one declared not-applicable boundary, validates export, and checks
BagIt SHA-256 inventory. Exit code alone cannot pass semantic claim. Package
retains exact executable bytes/hash but does not prove source-to-binary build
provenance or cryptographic execution causality.

```sh
./scripts/benchmark-feature-ledger.sh --run-id local-example
python3 scripts/verify-evidence-report.py /path/printed/by/runner
```

Current journey-v1 report selects 17 claims: 14 semantic observations, 2
expected command refusals, and 1 discontinued runner boundary marked
`not_applicable`. This is not “17/17 product completeness” and not performance
benchmark. Method and reference run:
[docs/benchmark-feature-ledger.md](docs/benchmark-feature-ledger.md).

## Capabilities

| Surface | Current local behavior |
|---|---|
| `setup`, `attach`, `detach`, `health` | install/adopt hooks and inspect local capture state |
| `capture` | internal hook receipt producer/drainer; direct CLI input remains caller supplied |
| `campaign`, `intent`, `pr`, `queue` | record and project local lifecycle |
| `check` | execute local subprocess on memo miss; reuse file AC result on exact key hit |
| `verdict` | record caller-supplied lens decision; no model call |
| `ctx usage` | record caller-supplied counters; no provider authentication |
| `land queue` | exercise local union/bisect simulator; no repository-tree integration |
| `why`, `ledger`, `fleet`, `watch`, `review` | integrity-check and project local records |
| `symbol` | tree-sitter outline for local or committed source |
| `dock` | bind and reconcile local worktree records |
| `export` | write Git plus JSON exit artifact from exportable canonical corpus |

Full source-derived register, per-mode status, implementation paths, and required
evidence live in [docs/feature-ledger.md](docs/feature-ledger.md).

## Provenance Boundaries

- Hook-created commit target comes from Git. Direct CLI/MCP `capture` accepts
  caller text and does not prove object reachability.
- Commit gains intent/model/usage/verdict context only through explicit PR
  binding. Unbound commit stays unlabelled.
- Usage counters and verdicts are submitted assertions. Hugit preserves and may
  price them against frozen local card; it does not authenticate provider source
  or call review model.
- Canonical log uses unkeyed hash chain. It detects partial edits, insertion,
  deletion, and reorder; writer with full file access can rewrite whole chain.
- Pre-push hook records attempt, not remote acceptance.
- `land queue` runner currently always succeeds; red pairs come from
  `conflicts-with:` fixture strings. It proves simulator mechanics only.
- `ws`, top-level `dispatch`, remote AC, identity, tenancy, forge/hosting,
  mirror deployment, runner execution, and external runner attestation are
  discontinued current-product scope.

## Local Storage

- Canonical log: `<git-common-dir>/hugit/event-log.json`
- Capture recovery: `<git-common-dir>/hugit/receipts/`
- File AC: explicit `--ac` path or adjacent local store
- Intent sidecar: explicit `--store` path

Linked worktrees share Git common-dir runtime. Tracked worktree does not contain
canonical log by default.

## Symbol Outline

```sh
hugit symbol --file src/main.rs
hugit symbol --ref HEAD --path src/main.rs --git-dir .
```

Supported languages: Rust, TypeScript, TSX, JavaScript, Python, Go, Java, C,
C++, and Ruby.

## Export

```sh
hugit export \
  --log .git/hugit/event-log.json \
  --out /tmp/hugit-export
```

Export verifies local chain first and writes Git/JSON artifact plus redaction
manifest. Some canonical events containing sensitive receipt/fingerprint fields
are intentionally refused; evidence report therefore tests export using separate
clean CLI-created corpus and states that limitation.

## Documentation

- [Installation](docs/installation.md)
- [Feature ledger](docs/feature-ledger.md)
- [Evidence report method](docs/benchmark-feature-ledger.md)
- [Evidence research](docs/research/2026-09-13-professional-evidence-report.md)
- [Product brief](docs/product/product.md)
- [Whitepaper](docs/whitepaper/hugit-v1.md)

## License

[Apache-2.0](LICENSE)
