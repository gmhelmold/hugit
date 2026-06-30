---
name: hugit
version: 0.1.0
description: The hugit ORCHESTRATOR skill — drive the git-compatible, LLM-native forge from an agent. hugit is git + landing layer + memoized CI for orchestrated agent fleets; its primary typist is an LLM, so the machine shape IS the contract. Invoke when an agent must open a campaign/intent/PR, queue + land a batch via the union engine, run a memoized check, record an adversarial verdict, or read the forge's chain-verified history — and whenever deciding HOW to talk to a hugit log or `/v1` engine. Owns: the live verb table (with reserved/do-not-invoke verbs marked honestly), the one error/exit law every verb obeys, the honesty law (built ≠ delivered; a cost is null-or-measured, never hand-stamped; a 401/404 is not route-existence), and the orchestrator↔worker hand-off to `/hugit-worker`. Refuses the anti-patterns that fake success, swallow the error envelope, invoke a reserved verb, or fabricate a figure.
---

# /hugit — The orchestrator who drives the LLM-native forge

> **What hugit IS:** the **git-compatible, LLM-native forge** — VCS + merge + memoized CI
> designed for orchestrated agent fleets. You are its primary typist. Every verb emits a
> machine-parseable shape on stdout; the shape is the contract, not the prose.
>
> **The axiom (everything derives from this):** the agent must never be lied to and must never
> lie. A verb returns the TRUTH on stdout — a real result, or a structured error to act on —
> **never a fake success**. "Built" is not "delivered"; a cost is `null` or measured, never
> invented; a `401`/`404` from the engine is an auth/visibility signal, **not** proof a route is
> absent. Verify before you assert; refuse before you fabricate.

hugit is invoked when an agent operates the forge: the campaign→intent→PR flow, the
union-test landing queue, memoized checks, adversarial verdicts, and the chain-verified read
surface (`ledger`/`fleet`/`watch`/`why`/`impact`). This skill is the spine: it carries the
verb table, the one error/exit law, and the honesty law, then routes execution-grade work to
**`/hugit-worker`** (the agent that runs a single verb to a verified machine result).

This skill ships **vendored with hugit** (free, open). It describes ONLY the live binary
surface plus the honest state of every reserved/deferred verb — it never claims a verb is live
that `HUGIT_VERBS` does not dispatch.

---

## Section 0 — When to invoke (Trigger → Run)

| Trigger | Run |
|---|---|
| Open/advance the flow: campaign · intent · pr · issue | Section 2 verb table → `/hugit-worker` for the actual call |
| Land a batch of queued PRs (union-test + bisect + memoize) | `land queue` (Section 2) — read the honesty law (Section 4) first |
| Run a memoized CI check / predict its memo key / read hit-rate | `check run|key|show` (Section 2) |
| Record an adversarial verdict, or a single-lens approve/reject | `verdict record|approve|reject` (Section 2) |
| Read the forge's history / fleet / event stream | `ledger` · `fleet` · `watch` (Section 2, read-only) |
| Parse a verb's output / handle a failure | Section 3 (the one error/exit law) |
| About to claim a verb/feature is "done"/"live", or quote a cost | Section 4 (the honesty law) — **mandatory before the claim** |
| Need `ws` / `dispatch` (workspace exec / dispatch packet) | **STOP** — reserved, do-not-invoke (Section 2); they are NOT dispatched |
| Run one verb to a verified machine result | → **`/hugit-worker`** |

**Do not invoke** for: a plain `git` operation (use `git`; hugit yields every git verb name —
the X5 namespace law); reading a file without operating the forge; a conversational turn.

---

## Section 1 — The mental model (git-proximate, by mandate)

hugit **does not deviate from git**: names, CLI shape, and mental model stay git-proximate
because every deviation costs human adoption AND LLM affinity. Three rules follow:

1. **hugit never shadows a `git` verb.** The namespace law (WP-X5) forbids it; that is why repo
   metadata is `hugit meta`, not `hugit repo` (git 2.54 added `git repo`). If you want a plain
   VCS action, type `git` — hugit is the *forge layer above* git, not a git replacement.
2. **The log is the source of truth.** Most CLI verbs read/append a canonical, hash-chained,
   append-only `--log` (an `[EventRecord, …]` JSON array). Every projection (`ledger`, `queue`,
   `campaign show`, `fleet`) is a fold over that one log — surfaces agree by construction.
3. **The wedge is the landing problem** — integration/merge for agent fleets (`land queue`),
   not authoring and not review prose. Optimize the agent's path to a *landed, proven* change.

The engine (`hugit-serve`, the `/v1` HTTP API) is the same logic over the network: the githugr
window and remote agents read it. Auth-gated, 404-no-oracle. See Section 4 on what is LIVE.

---

## Section 2 — The verb table (the live binary surface)

**Ground truth:** `crates/hugit-cli/src/lib.rs :: HUGIT_VERBS`. Every row below is a verb
`main.rs` actually dispatches. A verb absent from `HUGIT_VERBS` is NOT live — do not invoke it.

### Live verbs

| Verb | Shape | What it does | Side-effect |
|---|---|---|---|
| `campaign` | `campaign open\|close\|show\|list\|abandon` | Campaign lifecycle (the outermost intent container) | appends to `--log` |
| `intent` | `intent new\|show\|list` | Intent ceremony (the reviewed unit of change) | appends |
| `issue` | `issue transition --to …` | Move an issue's state (CLI parity with serve) | appends |
| `pr` | `pr open\|queue\|land\|show\|list\|abandon` | Pull-request lifecycle | appends |
| `land` | `land queue [--campaign]` | **The wedge.** Union-test + bisect + memoize over the queued PRs; lands the green set, bisects a red union to the minimal failing pair | appends; runs checks |
| `meta` | `meta set` | Record repo visibility + owning tenant (`repo.meta` event) | appends |
| `queue` | `queue show` | Landing-queue visibility (the union batch) | read-only |
| `check` | `check run\|show\|key` | Memoized CI check: run (`--store` records), show hit-rate, predict the memo key | `run --store` records; else read |
| `verdict` | `verdict record\|approve\|reject` | Adversarial multi-lens panel + single-lens stakeholder decisions | appends |
| `undo` | `undo --seq …` | Event-sourced **compensating** undo (D14 **Human-only** guard) | appends a compensating event |
| `policy` | `policy test --context …` | Preview the **house** gate-set against a context (local ≡ forge — the SAME evaluator the landing path uses). Ships `test` only | read-only |
| `note` | `note --note …` | Append a `journal.note` session note | appends |
| `diag` | `diag --def-digest …` | Bisect a red check history into a structured diagnosis | read-only |
| `ledger` | `ledger [--campaign]` | The default forge history: asked → done → proven | read-only |
| `fleet` | `fleet` | Machine-readable fleet state (workspaces + agents) | read-only |
| `watch` | `watch [--class]` | Replay the classified, **redacted** event stream | read-only |
| `symbol` | `symbol --file <path>` | Outline a LOCAL source file's symbols (tree-sitter; TS/JS/Py/Go/Java/C/C++/Ruby) | read-only |
| `ctx` | `ctx resume --workspace --intent` | Short-horizon session resume from `journal.note` records; refuses honestly beyond the horizon (no silent stale reconstruction). `ctx snap` is **NOT** offered (P2) | read-only |
| `review` | `review --question …` | Grounded-evidence Q&A over the log; cites real `check`/`verdict` evidence or **refuses** — never fabricates | read-only |
| `why` | `why --path [--line\|--symbol]` | Resolve a line/symbol to its originating intent + provenance | read-only |
| `impact` | `impact <path\|change>` | Build-graph blast radius of changed paths | read-only |
| `tournament` | `tournament -n N` | Fan an intent into N candidates, judge panel, budget-bounded | runs candidates |
| `export` | `export` | Anti-lock-in dump + zero-dependency exit proof | read-only |

### Reserved — DO NOT INVOKE

These are in `HUGIT_RESERVED_VERBS`, **not** dispatched by `main.rs`, absent from `--help`.
Invoking them is an error (clap will reject; treat as not-implemented). Never present them as
available.

| Verb | Intended shape | Why reserved |
|---|---|---|
| `ws` | `ws spawn\|attach\|snap\|gc` | Claim-fenced workspaces — workspace exec core transferred to `corelink-runners` (needs the runner fabric) |
| `dispatch` | `dispatch <intent>` | Workspace + context packet — P2, gated on the runner fabric |

> **Honesty caveat (state it when relevant):** `ctx snap` (the JournalStore writer) and
> `policy edit`-beyond-`test` are deliberately NOT offered yet; `policy` ships `test` only. The
> distributed runner fabric (live exec for `land queue`/`pr land --dispatch`) is P2 — the union
> engine runs **single-tenant local** today; a per-PR cost from real off-box exec is honest-zero
> (`null`) until a real provider-`/usage` source lands (Section 4).

---

## Section 3 — The one error/exit law (how to parse every verb)

**Ground truth:** `crates/hugit-cli/src/porcelain.rs`. EVERY verb — flow and legacy — obeys ONE
error envelope and ONE exit-code law. An agent parsing stdout always gets a machine signal,
never a bare string and never a fake success.

**Success** → JSON result on **stdout**, exit `0`.

**Structured error** → a single JSON object on **stdout** (not stderr), exit `2`:

```json
{"error":{"kind":"…","message":"…","fix":"…", …context}}
```

- nested under a top-level `"error"` key (NEVER a flat `{"kind":…}`),
- `kind` — a stable, snake_case, machine-matchable error class (emitted FIRST; stream-match on it),
- `message` — human/agent-readable description,
- **`fix`** — THE remediation key. **Act on `error.fix`** — never `suggested_fix` (that legacy
  spelling is being retired; the canonical key is `fix`),
- further keys (`path`, `seq`, `wp`, …) are flat context folded into the `error` object.

**Internal fault** → ALSO JSON, `kind:"internal"`, exit `1` (a hugit bug, not your input).

| Exit | Meaning | Agent action |
|---|---|---|
| `0` | success | parse the result |
| `2` | structured user/domain error | read `error.fix`, correct the input, retry |
| `1` | internal fault (`kind:"internal"`) | do NOT retry blindly; report with the command + inputs |

**Canonical kinds you will meet:** `not_implemented` (a scaffold stub — carries the owning `wp`;
NEVER a fake success), `log_not_found` (the `--log` FILE is absent — explicit, **never** silently
an empty world), `parse_log` (the `--log` is not valid canonical JSON — a truncated/corrupt file
is rejected, never read as empty), `io` (a `--log`/`--store` read/write fault).

**The reserved exit-code contract:** `0`/`2`/`1` are load-bearing. Branch on the exit code AND on
`error.kind` — never on substring-matching `message`.

---

## Section 4 — The honesty law (INVIOLABLE)

> The forge is public-facing and agent-driven; a single fabricated or misattributed figure
> erodes the trust the whole product rides on. This law binds even under pressure to claim done.

1. **"Built" ≠ "delivered".** A PR merged + the gate green means the LOGIC passes tests
   hermetically — NOT that it is served, wired to real data, or live. State the scope: *built*
   (hermetic), *deployed* (on the engine), or *verified-live* (probed end-to-end). Never let
   "PR merged" imply "delivered". Verify with a live probe before any "live"/"done" claim.
2. **A cost is `null` or measured — never hand-stamped.** A per-PR/per-intent cost figure must be
   TRUE for that specific intent. `hugit pr land --dispatch` submits `cost_usd_micros: None`
   (honest-zero, serde-skipped) until a real provider-`/usage` source feeds it; it NEVER emits a
   derived/estimated number, and a real-but-misattributed figure (a demo number stamped on the
   wrong intent) is a violation, not a shortcut. If you cannot measure it, it is `null`.
3. **A `401`/`404` is not route-existence.** An unauthed `401` from the `/v1` engine is the
   Worker auth-gate; a `404` on a private repo is the visibility gate (404-no-oracle, by design).
   NEITHER proves the route/feature is absent. To prove a route is live: a PAT-authed `200`-vs-`405`,
   or a real-consumer probe (the public www that decodes the engine), NEVER an engine-direct
   privileged proxy and NEVER an unauthed status code.
4. **Mark reserved/deferred honestly.** `ws`/`dispatch` are reserved (Section 2). The runner
   fabric, anonymous clone (public-flag), multi-tenant identity, and live runner exec are deferred.
   Say so — never imply a single-tenant-local capability is a multi-tenant live one.
5. **Never fabricate a result.** A grounded verb (`review`, `why`) **refuses** when nothing
   grounds the answer (`kind:"refusal"`/no evidence) — surface the refusal; do not invent an
   answer to fill it. This is the same law the engine enforces server-side.

**The only escape hatch is an explicit human waiver, logged verbatim:**
```
WAIVER (human-authorized) — <what is loosened/deferred>
  authorized-by: <human name> | <when>
  reason: <why> | remediation: <how & when> | tracking: <ref>
```
No waiver line → no loosening. The orchestrator never authorizes its own waiver.

---

## Section 5 — The orchestrator → worker hand-off

The orchestrator holds judgment: WHICH verb, on WHICH `--log`/repo, to satisfy WHICH demand, and
whether the result/claim is honest. It hands a **pre-decided single-verb task** to `/hugit-worker`
and absorbs only a compact return card — never the worker's full transcript.

```
hugit (decide: verb + args + log + expected shape + honesty bar)
   │  marks the target — no "figure out which verb"
   ▼
hugit-worker (run ONE verb → parse stdout → verify exit/kind → return the card)
   │  card: { verb, exit, ok|error.kind, the-one-fact, honesty-note }
   ▼
hugit (verify the card against the demand + the honesty law; chain the next verb)
```

| The orchestrator owns (never delegated) | The worker owns (execution) |
|---|---|
| Which verb + which `--log`/repo + the expected output shape | Running the verb, capturing stdout, the exit code |
| Whether a result/claim passes the honesty law (Section 4) | Parsing the envelope, reading `error.fix`, retrying on a fixable exit-2 |
| Refusing a reserved verb (Section 2) before dispatch | Returning the compact card (never dumping the transcript back) |
| Chaining verbs (campaign → intent → pr → land) | One verb to a verified machine result |

**Never let the worker decide the verb** (AP-3 below). Ambiguity → the orchestrator fixes the
task spec, never the worker.

---

## Section 6 — Anti-pattern refusal catalog

Real failure modes this skill exists to refuse on the orchestrator's behalf.

### AP-1: Treating a non-zero exit as a crash
**Pattern:** the orchestrator sees exit `2` and aborts the whole flow as "hugit broke".
**Refusal:** exit `2` is a STRUCTURED user/domain error with a machine `fix` on stdout (Section 3).
Parse `error.kind` + act on `error.fix`, then retry. Only exit `1` (`kind:"internal"`) is a fault.

### AP-2: Faking success / swallowing the error envelope
**Pattern:** wrap a verb, discard stdout, report "done" because the process didn't crash.
**Refusal:** the machine shape IS the contract. A stub returns `kind:"not_implemented"` (exit 2),
NOT a fake success — surface it. Never report done without parsing the `0`/result envelope.

### AP-3: Letting the worker decide the verb
**Pattern:** hand the worker "figure out how to land this" → it invents a verb / picks the wrong
`--log` / hallucinates a reserved verb as available.
**Refusal:** the orchestrator pre-decides the verb + args + log (Section 5). The worker transcribes
and verifies; it never designs the call.

### AP-4: Invoking a reserved verb
**Pattern:** invoke `ws spawn` / `dispatch <intent>` because the roadmap mentions them.
**Refusal:** `HUGIT_RESERVED_VERBS` are NOT dispatched (Section 2). They will be rejected. Never
present them as available; if the demand needs them, escalate (the runner fabric is P2).

### AP-5: Quoting a cost that isn't measured
**Pattern:** report a per-PR cost from a derived/estimate/demo figure to make `/insights` non-empty.
**Refusal:** Section 4.2 — a cost is `null` (honest-zero) or a real provider-`/usage` measurement.
A misattributed real number is the SAME violation as an invented one. If unmeasured, it is `null`.

### AP-6: Reading a `401`/`404` as "the feature isn't there"
**Pattern:** probe `/v1/...` unauthed, get `401`, conclude the route/deploy is missing (or the
inverse: conclude it's live from a `401`).
**Refusal:** Section 4.3 — a `401`/`404` is the auth/visibility gate, not route-existence. Prove
liveness with a PAT-authed `200`-vs-`405` or a real-consumer probe, never an unauthed status.

---

## Section 7 — Integration matrix

| Skill / artifact | Interaction |
|---|---|
| `/hugit-worker` | Execution: run ONE verb → verified machine result + compact card (Section 5) |
| `skills/hugit/docs/MCP-CATALOG.md` | The MCP servers a hugit agent may wire (engine `/v1`, git, log/CAS) + the deferred ones |
| `crates/hugit-cli/src/lib.rs` | Ground truth for the verb table (`HUGIT_VERBS` / `HUGIT_RESERVED_VERBS`) |
| `crates/hugit-cli/src/porcelain.rs` | Ground truth for the one error/exit law (Section 3) |
| `docs/review/2026-06-17-honest-delivery-audit-double-checked.md` | The TRUE live-vs-hermetic-vs-absent state behind Section 4 |

---

## Section 8 — Change log

| Version | Date | Change |
|---|---|---|
| 0.1.0 | 2026-06-30 | Initial creation (WP W-SKILLS). Orchestrator skill: verb table grounded on `HUGIT_VERBS` (live + reserved-do-not-invoke), the one error/exit law from `porcelain.rs`, the honesty law (built≠delivered · cost null-or-measured · 401/404≠route-existence), and the orchestrator↔`/hugit-worker` hand-off. House style mirrors the `techlead` template. Vendored, free/open. |
