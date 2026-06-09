# Post-v1 follow-on — ADR-0001 context-envelope rework

> **Why this doc exists:** ADR-0001 (`docs/adr/0001-intent-context-envelope.md`)
> formally defines `context.json` for the first time and adds per-intent +
> per-PR **metrics** (tokens, wall/active ms, tool-calls, cost) and a
> **three-altitude trajectory** (raw / task / summary). None of this exists in
> the built v1 (the 67-WP register shipped before the envelope was specified).
> This is **net-new rework in hugit**, tracked here so it is not a loose end.
>
> The 67-WP register (`wp-contracts/INDEX.md`) is frozen v1 — not edited. These
> are **follow-on WPs (F-series)**, same contract rigor, sequenced after owner
> ratification.
>
> **Status: BLOCKED** on ADR-0001 §7 ratification (default capture level,
> retention TTL, cost visibility) — those three answers parametrize WP-F2.

## What in v1 is touched (impact map)

| Built WP | What changes |
|---|---|
| **WP-00** hugit-contracts | + new frozen `ContextEnvelope` + `IntentMetrics` types (additive; `IntentSidecar.context_ref` already points at this blob — no break) |
| **B6** intent sidecar | producer writes the richer envelope behind `context_ref` |
| **C9 / runner harness** | must **emit metrics** + capture the 3-altitude trajectory at intent close |
| **D4** intents+projection · **D5** ledger · **D10** why+impact | project the new fields; compute the **PR rollup** |
| **D11** journals+resume | `journal_ref` retained alongside the machine trajectory |
| **X3** context privacy | redaction-on-write + capture levels + retention TTL must cover transcripts |

## The follow-on contracts

### WP-F1 — `ContextEnvelope` contract freeze · S · opus · 60k · branch `wp/f1`
**Charter.** Freeze the envelope as a contract type so producers/consumers agree.
**Owned acceptance.** `ContextEnvelope` + `IntentMetrics` Rust types with
`#[serde(deny_unknown_fields)]`, `schema_version:"1.0.0"`, JSON Schema, golden
byte-exact serde round-trip tests (the WP-00 pattern). Nullable refs for
sub-`full` capture levels round-trip. `PrRollup` defined as a **derived**
(forge-computed, NOT frozen) shape. Wire `IntentSidecar.context_ref` → this blob.
**Deps.** Consumes nothing new; lives in `hugit-contracts`.
**DoD.** Global gate green (fmt+clippy+test+audit); cold-verify by non-author.

### WP-F2 — metrics emission + trajectory capture (producer) · M · opus · 80k · branch `wp/f2`
**Charter.** The runner/agent harness produces the envelope: metrics + the
three transcript altitudes, redacted, honouring the repo capture level.
**Owned acceptance.** At intent close the harness records: tokens
(input/output/cache_read/cache_write/total), `wall_ms`, `active_ms`,
`tool_calls` + per-tool breakdown, `model_turns`, derived `cost_usd`. Writes
`raw_transcript_ref` (full born→die loop, every turn + tool call/result),
`task_transcript_ref` (task-scoped), and inline `summary` to CAS. **Redaction
applied on the write path** (REDACTED_MARKER per policy) before any blob is
stored. **Capture level** (`off|metrics|task|full`) gates what is written;
absent refs are `null`. Retention TTL tagged on `full` blobs.
**Deps.** WP-F1 frozen types; X3 redaction policy; C9 runner lifecycle.
**Blocked-by.** ADR-0001 §7 answers (default level, TTL, cost visibility).
**DoD.** Global gate green; cold-verify; a metrics+trajectory bundle proven
against a real spawned intent.

### WP-F3 — projection + PR rollup (consumer) · M · sonnet · 70k · branch `wp/f3`
**Charter.** Project the envelope and compute the per-PR rollup for the surface.
**Owned acceptance.** Forge computes `PrRollup` over a PR's intents: token/
tool-call/cost sums + **`wall_ms_span`** (first born→last died) and
**`wall_ms_sum`** (Σ per-intent) as distinct figures. Why-blame/ledger surface
per-intent metrics + the trajectory altitudes; CI memoization economics
(`cache_hit`/`exec`/cost) included. Tolerates `null` refs gracefully.
**Deps.** WP-F1 types; D4/D5/D10 projection seams.
**DoD.** Global gate green; cold-verify; rollup proven on a multi-intent PR.

## Downstream (not hugit)

githugr must display all of the above — tracked in
`githugr/docs/adr/0001-intent-context-envelope.md` (companion) and its design
backlog (update `landing.html` drawer + `pr-detail.html` mockups under DDD).

## Sequence

Ratify ADR-0001 §7 → **WP-F1** (freeze) → **WP-F2** (produce) ∥ **WP-F3**
(consume, after F1) → githugr display. F2 and F3 both depend on F1 only.
