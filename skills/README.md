# hugit skills — the LLM interface to the forge

hugit's primary typist is an orchestrated agent, so hugit ships two **vendored** skills (free,
open — they travel with the binary) that teach an agent to drive the forge honestly. The skills
are the human/LLM-facing complement of the machine contract that already lives in the code: the
verb registry (`crates/hugit-cli/src/lib.rs :: HUGIT_VERBS`) and the one error/exit law
(`crates/hugit-cli/src/porcelain.rs`).

## The two skills

| Skill | Role | Invoke when… |
|---|---|---|
| [`hugit/SKILL.md`](hugit/SKILL.md) | **Orchestrator** — drives the forge | an agent must open/advance the campaign→intent→PR flow, land a batch via the union engine, run a memoized check, record a verdict, or read the chain-verified history — and whenever deciding HOW to talk to a hugit log or `/v1` engine. |
| [`hugit-worker/SKILL.md`](hugit-worker/SKILL.md) | **Worker (agent)** — runs ONE verb | an agent is the executor of a single, pre-decided verb and must turn it into a verified machine result + a compact card. |

The split mirrors the orchestration model: the **orchestrator holds judgment** (which verb, on
which log/repo, whether a result/claim is honest) and the **worker holds execution** (run the
verb, parse the envelope, verify the exit, return the card — it never decides). The hand-off is
specified in `hugit/SKILL.md` Section 5.

## What the skills guarantee

Three laws run through both skills (they are the product's trust spine):

1. **The machine shape IS the contract.** Every verb emits JSON on stdout. Success → result, exit
   `0`. A structured error → `{"error":{"kind","message","fix", …}}`, exit `2`. An internal fault
   → `kind:"internal"`, exit `1`. Agents branch on the exit code + `error.kind`, act on
   `error.fix`, never substring-match prose. (Ground truth: `porcelain.rs`.)
2. **The honesty law.** "Built" ≠ "delivered" (hermetic-green is not live); a cost is `null` or
   measured, **never** hand-stamped/estimated/misattributed; a `401`/`404` from the engine is an
   auth/visibility signal, **not** route-existence. The skills refuse every fabrication.
3. **Honest reserved/deferred state.** The skills describe ONLY the live binary surface
   (`HUGIT_VERBS`). Reserved verbs (`ws`, `dispatch` — in `HUGIT_RESERVED_VERBS`, not dispatched)
   are marked **do-not-invoke**; the runner fabric, anonymous clone, multi-tenant identity, and
   live runner exec are flagged as deferred — never implied to be live.

## Supporting docs

- [`hugit/docs/MCP-CATALOG.md`](hugit/docs/MCP-CATALOG.md) — the MCP servers a hugit agent may
  wire (the engine `/v1` API, git over the wire, the log/CAS read surface) with their auth and
  honesty caveats, plus the deferred ones.

## Grounding (single source of truth)

The skills are **ground-truth-anchored**, not free prose:

| Claim in the skills | Anchored to |
|---|---|
| The live verb table + the reserved-do-not-invoke set | `crates/hugit-cli/src/lib.rs` (`HUGIT_VERBS` / `HUGIT_RESERVED_VERBS`) |
| The one error/exit law (`0`/`2`/`1`, `error.fix`, the canonical kinds) | `crates/hugit-cli/src/porcelain.rs` |
| The honesty law (built≠delivered · cost null-or-measured · 401/404≠route) | `docs/review/2026-06-17-honest-delivery-audit-double-checked.md` + `CLAUDE.md` |

When the CLI surface changes (a reserved verb graduates, an error `kind` is added), update the
skill tables in the SAME change — the skills must never drift from `HUGIT_VERBS`.

## License

Vendored with hugit (Apache-2.0, free/open) — they ship so any agent driving hugit gets the
honest interface, no separate install.
