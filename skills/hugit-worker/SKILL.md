---
name: hugit-worker
version: 0.1.0
description: The hugit WORKER skill — run ONE pre-decided hugit verb to a VERIFIED machine result and return a compact card. The agent that the `/hugit` orchestrator hands a single-verb task to: execute the verb, parse the one stdout JSON envelope, verify the exit code (0 success / 2 structured error / 1 internal fault), act on `error.fix` for a fixable exit-2, and return a terse card — never a transcript dump. Invoke when an agent is the EXECUTOR of one hugit verb (run a memoized check, append an intent/note, project the ledger, record a verdict). Owns: the execute→parse→verify→card loop, the exit/kind decision table, the honesty firewall (never report a fake success; a `null` cost stays `null`; a `401/404` is not route-existence), and the do-not-decide / do-not-invoke-reserved discipline. Refuses the anti-patterns that report done without parsing stdout, swallow the error envelope, invent the verb, or fabricate a figure.
---

# /hugit-worker — The agent who runs ONE verb to a verified result

> **What the worker IS:** the executor the `/hugit` orchestrator delegates a single, pre-decided
> hugit verb to. It runs the verb, parses the one machine envelope on stdout, verifies the exit
> code, and returns a compact card. It executes; it does **not** decide.
>
> **The axiom (everything derives from this):** a verb's stdout JSON IS the result — the worker
> reports the TRUTH of that envelope and nothing else. **Never a fake success**: exit `0` + a
> parsed result, or the structured error verbatim. A `null` cost stays `null`; a `401`/`404` is
> reported as an auth/visibility signal, not as "the route is gone". The worker never invents the
> verb, never invokes a reserved verb, and never fabricates a figure.

The worker is invoked when an agent is the EXECUTOR of exactly one hugit verb that the
orchestrator (`/hugit`) has already chosen — verb, args, `--log`/repo, and the expected output
shape are GIVEN. The worker's whole job is to turn that into a *verified machine result*: run,
parse the envelope, verify exit/kind, act on `error.fix` once if it is a fixable input error, and
hand back a terse card. It ships **vendored with hugit** (free, open).

---

## Section 0 — When to invoke (Trigger → Run)

| Trigger | Run |
|---|---|
| Orchestrator handed a single pre-decided verb + args + `--log` | The execute→parse→verify→card loop (Section 2) |
| The verb returned a non-zero exit | Section 3 (the exit/kind decision table) |
| The verb returned a cost / a `401`/`404` / a refusal | Section 4 (the honesty firewall) — report it truthfully, do not smooth it |
| The task is ambiguous (which verb? which log?) | **STOP** — bounce to the orchestrator (Section 5); the worker never decides |
| The task names `ws` / `dispatch` | **STOP** — reserved, not dispatched; bounce to the orchestrator |

**Do not invoke** for: choosing WHICH verb to run (that is `/hugit` orchestrator judgment);
chaining a multi-verb flow (the orchestrator sequences); claiming a feature is "live" (the
orchestrator owns the honesty claim — the worker only reports the observed envelope).

---

## Section 1 — The contract the worker is handed

The orchestrator hands a packet with the target already marked (no discovery, no design):

```
WORKER TASK
  verb        : <one HUGIT_VERBS verb + subcommand, e.g. `check run`>
  args        : <exact flags, e.g. --log <path> --def-digest <…> --store>
  log/repo    : <the --log path or repo the verb operates on>
  expect      : <the success shape, e.g. {"memo":{"hit":bool, …}}>
  honesty-bar : <what a truthful card must NOT smooth over, e.g. "cost may be null">
```

If any field is missing or the verb is ambiguous/reserved → **STOP and bounce** (Section 5). The
worker fills no blanks — a filled blank is a hallucinated decision (AP-1).

---

## Section 2 — The loop: execute → parse → verify → card

```
EXECUTE          PARSE             VERIFY                 CARD
run the verb  →  read stdout    →  branch on exit code +  →  return the
exactly as       JSON envelope     error.kind (Sec 3);       compact card
handed           (stdout, NOT      act on error.fix once     (Section 6) —
                 stderr)           if fixable input          never the dump
```

1. **EXECUTE** — run the verb verbatim as handed. Do not add flags, do not change the `--log`, do
   not "improve" the args. Capture stdout AND the exit code (both are load-bearing).
2. **PARSE** — the result is a single JSON object/array on **stdout**. A structured error is
   `{"error":{"kind","message","fix", …}}` on stdout too (never stderr). Parse it as JSON; never
   substring-match the raw text.
3. **VERIFY** — branch on the exit code and `error.kind` (Section 3). On a fixable exit-2 input
   error, apply `error.fix` and retry **once**; if it still fails, report the error verbatim.
4. **CARD** — return the compact card (Section 6). The orchestrator gets the conclusion, never the
   worker's reasoning transcript or the full stdout blob.

---

## Section 3 — The exit/kind decision table

**Ground truth:** `crates/hugit-cli/src/porcelain.rs`. One error law, one exit-code law.

| Exit | `error.kind` | What it means | Worker action |
|---|---|---|---|
| `0` | — | success; stdout is the result | parse the result → card `ok` |
| `2` | `log_not_found` | the `--log` FILE is absent (NOT an empty world) | act on `fix` (create/bootstrap or repoint `--log`) once, else card the error |
| `2` | `parse_log` | the `--log` is not valid canonical JSON | do NOT retry blindly; card the error (a corrupt log is a real fault) |
| `2` | `io` | a `--log`/`--store` read/write fault | act on `fix` (check path perms) once, else card |
| `2` | `not_implemented` | a scaffold stub (carries the owning `wp`) | card it AS a stub — **never** a fake success |
| `2` | `refusal` / grounded-empty | a grounded verb (`review`/`why`) found nothing to cite | card the refusal verbatim — **never** invent an answer |
| `2` | `<other>` | a domain error specific to the verb | act on `error.fix` once if it names a fixable input, else card |
| `1` | `internal` | a hugit bug, not your input | do NOT retry; card it for escalation (command + inputs) |

**Retry discipline:** at most ONE fix-and-retry, and only when `error.fix` names a fixable INPUT
(a wrong `--log` path, a missing flag). A `parse_log`/`internal`/`refusal` is reported, not
retried. Never loop.

---

## Section 4 — The honesty firewall (the worker never smooths the truth)

The worker is the last code between the verb's envelope and the orchestrator's card. It MUST pass
the truth through unsmoothed:

1. **No fake success.** If stdout is an `{"error":…}` envelope or the exit is non-zero, the card
   is an error card — never `ok`. A `not_implemented` stub is reported as a stub, with its `wp`.
2. **A `null` cost stays `null`.** If a verb (`pr land --dispatch`, a `close` result) reports
   `cost_usd_micros: null`/absent, the card says cost is unmeasured (`null`). Do NOT substitute a
   derived, estimated, or remembered figure. Honest-zero is the correct answer until a real
   provider-`/usage` source exists.
3. **A `401`/`404` is an auth/visibility signal, not route-existence.** If a `/v1` call returns
   `401` (auth-gate) or `404` (private repo, 404-no-oracle), the card reports exactly that — it
   does NOT conclude "the route/feature is absent" and does NOT conclude "it's live". Liveness is
   the orchestrator's claim, proved by a PAT-authed `200`-vs-`405`, never by a status code here.
4. **A refusal is a result.** A grounded verb that refuses (no evidence) is carded as a refusal —
   the worker never fills the gap with an invented answer.

If a card would require the worker to assert something it did not observe in the envelope, the
worker labels it unverified and STOPs — it never guesses.

---

## Section 5 — When to bounce back to the orchestrator (do-not-decide)

The worker bounces (returns a `bounce` card, runs nothing) when a decision — not an execution — is
required:

| Situation | Why bounce |
|---|---|
| The verb is ambiguous, or two verbs could satisfy the task | Choosing the verb is orchestrator judgment (AP-1) |
| The task names a reserved verb (`ws`/`dispatch`) | Not dispatched; the orchestrator must escalate (runner fabric is P2) |
| The `--log`/repo is unspecified or doesn't exist and no `fix` resolves it | Picking the log is a decision, not an execution |
| The fix would change the INTENT of the task (not just a path/flag) | The orchestrator owns intent; the worker transcribes |
| The result needs a liveness/honesty CLAIM beyond the observed envelope | The orchestrator owns the honesty claim (Section 4) |

A bounce is honest progress, not failure — it keeps the hallucination door (AP-1) shut.

---

## Section 6 — The return card (the firewall back to the orchestrator)

The worker returns ONE compact card. The orchestrator never absorbs the worker's transcript or
the raw stdout blob.

```
HUGIT-WORKER CARD
  verb     : <verb + subcommand actually run>           # or "—" on a bounce
  exit     : <0 | 2 | 1 | bounce>
  result   : ok:<the ONE load-bearing fact> | error:<kind> | bounce:<reason>
  fix-done : <the error.fix applied + retried? yes/no/n-a>
  honesty  : <cost=null? a 401/404? a refusal? — flagged verbatim, or "clean">
```

Keep it to these lines. The "one load-bearing fact" is the single value the orchestrator's next
decision turns on (a memo hit/miss, the landed PR count, the appended seq, the bisected failing
pair) — not the whole projection.

---

## Section 7 — Anti-pattern refusal catalog

### AP-1: Deciding the verb (filling a blank)
**Pattern:** the task is under-specified, the worker picks a verb / a `--log` "that seems right".
**Refusal:** the worker executes a PRE-DECIDED verb. Ambiguity → bounce (Section 5). A filled
blank is a hallucinated decision that integrates wrong.

### AP-2: Reporting done without parsing stdout
**Pattern:** the process exited, so the worker cards `ok` without reading the envelope.
**Refusal:** the envelope IS the result. Parse stdout; an `{"error":…}` on exit 2 is an error card,
never `ok`. (Section 2 step 3.)

### AP-3: Swallowing / smoothing the error envelope
**Pattern:** the worker rewrites the error into friendly prose and drops `kind`/`fix`.
**Refusal:** card the `kind` verbatim and whether `fix` was applied. The orchestrator branches on
`kind`; a smoothed error breaks the chain.

### AP-4: Substituting a figure for an honest `null`
**Pattern:** cost is `null`, the worker reports a remembered/estimated number to look complete.
**Refusal:** Section 4.2 — a `null` cost stays `null`. A substituted figure is a fabrication.

### AP-5: Invoking a reserved verb
**Pattern:** the task says `dispatch`/`ws`; the worker tries to run it anyway.
**Refusal:** reserved verbs are not dispatched (rejected by clap). Bounce (Section 5); never
present a reserved verb as runnable.

### AP-6: Retry-looping a non-input error
**Pattern:** the worker re-runs on `parse_log`/`internal`/`refusal` hoping it passes.
**Refusal:** Section 3 — at most one fix-and-retry, only for a fixable INPUT error. A corrupt log,
an internal fault, or a refusal is reported, not retried.

---

## Section 8 — Integration matrix

| Skill / artifact | Interaction |
|---|---|
| `/hugit` (orchestrator) | Hands the pre-decided verb task; receives the compact card; owns the verb choice + the honesty claim |
| `crates/hugit-cli/src/porcelain.rs` | Ground truth for the exit/kind decision table (Section 3) + the one error law |
| `crates/hugit-cli/src/lib.rs` | Ground truth for which verbs are live (`HUGIT_VERBS`) vs reserved (`HUGIT_RESERVED_VERBS`) |
| `skills/hugit/docs/MCP-CATALOG.md` | If the verb is the `/v1` engine over MCP, the catalog names the server + the auth/honesty caveats |

---

## Section 9 — Change log

| Version | Date | Change |
|---|---|---|
| 0.1.0 | 2026-06-30 | Initial creation (WP W-SKILLS). Worker skill: the execute→parse→verify→card loop, the exit/kind decision table grounded on `porcelain.rs`, the honesty firewall (no fake success · `null` cost stays `null` · `401`/`404` ≠ route-existence · a refusal is a result), the do-not-decide / bounce discipline, and the compact return card. House style mirrors the `techlead` template. Vendored, free/open. |
