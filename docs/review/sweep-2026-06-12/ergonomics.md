# Ergonomics / Machine-Surface Review — hugit HEAD 5457730

**Scope:** agent-facing machine surface of the `hugit` CLI (crates/hugit-cli).
**Lens:** LLM-agent ergonomics — JSON discipline, exit-code law, error envelope
consistency, determinism, idempotency, discoverability.
**Method:** live binary (dev build) driven against real verb paths in temp dirs;
source read for context. READ-ONLY — no edits, no commits.

---

## Findings

### F-1 — P2 — `kind` singular/plural inconsistency

**Repro:**
```
hugit tournament -n 0 --intent x   # {"error":{"kind":"invalid_argument",...}}
hugit campaign list                 # {"error":{"kind":"invalid_arguments",...}}
```
`invalid_argument` (singular, domain rule in `main.rs:308`) vs
`invalid_arguments` (plural, clap-parse path in `main.rs:479`). An agent
branching on `kind` must handle both spellings for what is semantically the same
class. SOTA: one stable token, always the same spelling. The clap-parse path
should normalise to `invalid_argument` (or vice-versa) and both should be
enumerated in the schema.

**File/line:** `crates/hugit-cli/src/main.rs:308` (singular) vs `main.rs:479`
(plural clap path).

---

### F-2 — P2 — Error envelope key order: `fix` precedes `kind`

**Repro:**
```
hugit tournament -n 0 --intent x
→ {"error":{"fix":"...","kind":"invalid_argument","message":"..."}}
hugit tournament -n 999 --intent x
→ {"error":{"cap":16,"fix":"...","kind":"policy_cap_exceeded","message":"...","requested":999}}
```
Inside the `error` object `fix` is consistently serialised before `kind`. An
agent parsing the envelope to branch on `kind` must parse the full object rather
than streaming until it sees `kind`. SOTA: `kind` first, then `message`, then
`fix`, then context — the discrimination key leads.

`serde_json::json!({...})` preserves insertion order; the fix is reordering the
literal in `PorcelainError::to_json()` (`crates/hugit-cli/src/porcelain.rs:122`).

---

### F-3 — P2 — No machine-readable schema for CLI success envelopes

**Observation:** `crates/hugit-contracts/schemas/` contains JSON Schema files for
data types (`EventRecord`, `VerdictObject`, `IntentSidecar`, etc.) but there is no
schema for the CLI's own success-output shapes: `campaign open`, `pr open`,
`pr land`, `campaign show`, `intent show`, `export`, `impact`, `tournament`, etc.
An agent integrating hugit must reverse-engineer the shapes from prose docs or
by running the binary — it cannot `$ref`-validate its parser against a published
schema.

SOTA: a machine-readable contract (JSON Schema, OpenAPI component, or a
`hugit schema --verb <name>` subcommand) for every success envelope. The
`HUGIT_VERBS` constant and the error-law doc exist in source but are Rust-only,
not consumable at agent runtime.

---

### F-4 — P2 — `pr list`: `count` means "total PRs" not "shown PRs"

**Repro:**
```
hugit pr list --log L --state queued
→ {"count":2,"prs":[],"shown":0}
```
`count` is the total number of PRs on the log; `shown` is the number after
filtering. An agent calling `if result.count == 0 { skip }` will silently skip
a non-empty log when a filter is active — it reads `count=2` but `prs=[]`.
The naming convention (`count` vs `shown`) is not documented in `--help`.

SOTA: either rename to `total` and `count` (conventional), or document the
distinction explicitly in `--help`. `campaign list` uses just `count` (total)
with no `shown` — the discrepancy is cross-verb.

**File:** `crates/hugit-cli/src/pr/mod.rs:1192-1197`.

---

### F-5 — P1 — No-args exits 0 with plain-text help on stdout

**Repro:**
```
$ hugit            # stdout: plain-text help block; exit=0
$ hugit --help     # stdout: same plain-text block;  exit=0
```
When an agent runs `hugit` with no arguments (e.g., discovery or a botched
dispatch), exit 0 is indistinguishable from a successful command. The stdout
is prose, not JSON, so a JSON-expecting parser either throws or silently
discards it. The agent cannot distinguish "binary present, invoked incorrectly"
from "command succeeded with empty output".

`--help` being exit 0 with prose is git-standard and intentional. But
**no-subcommand** is a missing-subcommand error: the code routes it through
`ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand` in `main.rs:462` and
exits 0 alongside the plain-text display. SOTA: no-subcommand should be
`exit 2` + `{"error":{"kind":"missing_subcommand","fix":"run hugit --help"}}` on
stdout. `--help` / `--version` remain exit 0 plain-text (deliberate).

**File:** `crates/hugit-cli/src/main.rs:460-467`.

---

### F-6 — P3 — No shell-completion subcommand

**Observation:** `hugit --help` does not list a `completions` subcommand. An
agent orchestrating a shell-tool invocation cannot programmatically discover
verb tokens; the verb registry (`HUGIT_VERBS`) is Rust-only. Shell completion
also helps human operators who review agent-generated commands.

SOTA: `hugit completions bash|zsh|fish` emitting a JSON manifest or a shell
completion script. Low-urgency for a pure-agent consumer; medium for human
oversight.

---

## What IS already SOTA

- **Zero stderr leakage.** Every error (domain, parse, internal) is JSON on
  stdout only. Zero bytes on stderr across all tested paths (why/impact/
  tournament/export/campaign/intent/pr/checks/queue/check/verdict).
- **Exit-code law: 0 / 2 / 1 — consistent across ALL verbs.** Every domain
  error is exit 2; internal faults are exit 1 (also JSON). No verb tested
  produced a wrong exit code on a genuine error path. The one clap trap
  (missing subcommand `DisplayHelpOnMissingArgumentOrSubcommand`) is identified
  above (F-5) but all other clap-parse errors correctly exit 2.
- **Idempotency everywhere it matters.** `campaign open`, `intent new`,
  `pr open`, `pr land`, `pr land --settle`, `pr abandon` all return
  `already_exists/already_queued/already_landed/already_abandoned` boolean on
  re-run, exit 0. Safe to retry.
- **Stable key-sets.** Both success and error envelopes have the same key-set
  on every call — no missing keys, no optional shape changes between first-run
  and re-run.
- **`fix` hints are actionable.** Every error `fix` is a concrete `hugit`
  command the agent can retry, not prose advice.
- **`kind` is snake_case throughout** (except F-1's singular/plural split).
- **JSON always on stdout on success.** No verb mixes prose and JSON on a
  success path.
- **Alphabetically sorted keys on success envelopes.** `campaign show`,
  `campaign open`, `pr open`, `pr land`, `intent show` all have alphabetically
  ordered top-level keys — Serde's insertion-order preserving combined with
  alphabetically-authored `json!({})` literals gives deterministic byte-stable
  output. Not guaranteed by a documented invariant but observed consistently.
- **Data-type schemas exist.** `crates/hugit-contracts/schemas/` publishes
  JSON Schemas for `EventRecord`, `VerdictObject`, `IntentSidecar`,
  `CheckDef`, `CheckResult`, `ContextEnvelope`, etc. — the input/persisted
  contract is machine-verifiable.
- **Scrub-on-append.** Secrets in free-text fields are redacted before
  reaching the hash-chained log — the write surface is safe for agent use.
- **Structured context.** Domain errors carry structured context fields (e.g.,
  `"path"`, `"pr_id"`, `"cap"`, `"requested"`) that an agent can inspect
  without parsing message strings.

---

## Machine-contract consistency verdict

**Mostly consistent, with two clear gaps (F-1, F-2) and one P1 invariant
break (F-5).**

The one-error-law holds across every verb tested: the `{"error":{...}}` envelope
shape is identical whether the error comes from a domain rule, a missing file, a
clap-parse failure, or an internal fault. Stdout is the only channel; stderr is
always empty. Exit 2 is universal for domain errors; exit 1 for internal faults.

The gaps are: one `kind` spelling inconsistency (F-1), key order inside the
error object (F-2), and the no-args `exit 0` + plain-text trap (F-5).
