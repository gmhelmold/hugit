# Contract: frozen schemas are `deny_unknown_fields` — versioned, not forward-compat

**Status:** grounded fact contract (KungFu). Every claim below cites `file:line`
in this repo at the time of writing. Read this before assuming a frozen
`hugit-contracts` type tolerates an unknown field "for forward compat" — it does
not, by design.

## The truth (one sentence)

The frozen shared types in `crates/hugit-contracts` are **closed**: each derives
`#[serde(deny_unknown_fields)]`, so an unknown field is a hard deserialization
error, and a shape change is a **versioned semver bump** of the type — NOT a
silent serde-default forward-compat absorb. This is the opposite of the wire
view-models in `crates/hugit-http-contracts`, which are deliberately
serde-default forward-compat.

## Why this matters

These types are the single source of truth for security-critical surfaces
(`RunnerLease`, `VerdictObject`, `AttestationChain`, `FenceManifest`, the
`ContextEnvelope` cost ledger). For an *attestation / accountability* contract, a
forgiving "ignore fields I don't recognize" parser is a hole: a producer could
slip an unrecognized field past a consumer that silently drops it, and the
attested object would no longer mean what the consumer thinks it means. Closing
the type (`deny_unknown_fields`) makes any out-of-contract field a loud failure,
and forces every shape change through an explicit, reviewed version bump.

## Evidence — the types are closed

`crates/hugit-contracts/src/lib.rs:1` states the crate's role: "frozen shared
types … the single source of truth for a shared contract surface … ZERO runtime
logic beyond derive-generated (de)serialization and schema generation."

Every frozen struct/enum module carries `#[serde(deny_unknown_fields)]`. As of
this writing **40 occurrences across all 16 frozen-type modules** (every module
re-exported from `lib.rs:36-56`):

```
crates/hugit-contracts/src/runner_lease.rs        crates/hugit-contracts/src/check_result.rs
crates/hugit-contracts/src/queue_api.rs           crates/hugit-contracts/src/attention_rank.rs
crates/hugit-contracts/src/attestation_chain.rs   crates/hugit-contracts/src/fence_manifest.rs
crates/hugit-contracts/src/check_def.rs           crates/hugit-contracts/src/regen_gate.rs
crates/hugit-contracts/src/context_envelope.rs    crates/hugit-contracts/src/shadow_policy.rs
crates/hugit-contracts/src/event_record.rs        crates/hugit-contracts/src/verdict_object.rs
crates/hugit-contracts/src/export_schema.rs       crates/hugit-contracts/src/diagnosis_object.rs
crates/hugit-contracts/src/app_webhooks.rs        crates/hugit-contracts/src/intent_sidecar.rs
```

A canonical example — `RunnerLease` and its `RunnerState` enum, both closed:

- `crates/hugit-contracts/src/runner_lease.rs:11` — `#[serde(rename_all = "snake_case", deny_unknown_fields)]` on `RunnerState`
- `crates/hugit-contracts/src/runner_lease.rs:28` — `#[serde(deny_unknown_fields)]` on `RunnerLease`

## The contrast — wire VMs are forward-compat, frozen types are not

The two crates make **opposite** serde choices, deliberately:

| | `hugit-contracts` (frozen) | `hugit-http-contracts` (wire VMs) |
|---|---|---|
| `deny_unknown_fields` | **40** occurrences | **0** |
| `#[serde(default)]` forward-compat | not used to widen closed shapes | **169** occurrences |
| evolution model | **versioned semver bump** | **additive, render-when-present** |

Verified by `grep -rn "deny_unknown_fields"` / `grep -rn "serde(default)"` over
each crate's `src/`. The view-models in `hugit-http-contracts` add fields
additively with `#[serde(default)]` so a new engine field is ignorable by an old
githugr renderer (see e.g. the `CostVm.cost_usd_micros` and structured-enum
additions in the repo `CHANGELOG.md` `[Unreleased]` — all explicitly "ADDITIVE +
forward-compat (`#[serde(default)]`)"). A **frozen** type never does this: it
bumps a version instead.

## Versioned bumps — the `ContextEnvelope` cost break (1.1.0 → 1.2.0)

The clearest worked example of "versioned, not forward-compat" is the money
amendment on `ContextEnvelope`. The schema-version constant and its full history
live at `crates/hugit-contracts/src/context_envelope.rs:63-83`:

- `crates/hugit-contracts/src/context_envelope.rs:83` —
  `pub const CONTEXT_ENVELOPE_SCHEMA_VERSION: &str = "1.2.0";`
- `1.1.0` (lines 66-69) — WP-F1b additive amendment (a fourth `Altitude::Session`),
  made the same day as the 1.0.0 freeze "while **zero producers existed** — no
  migration path was ever needed."
- `1.2.0` (lines 71-82) — the **WA4 money-as-integer amendment**. Quoting the
  source doc-comment: "**Money leaves f64.** Every cost field — `IntentMetrics::cost_usd_micros`
  and the derived `CostDecomposition` family's `*_usd_micros` fields — becomes an
  integer **micro-USD** `u64` (`1 USD = 1_000_000` micro-USD) … This is a clean
  **versioned break** of the 1.1.0 wire shape (`cost_usd: f64` is gone); correct
  because **zero producers exist in prod** — no migration path was ever needed."

So `cost_usd: f64` did not become an optional, serde-default-tolerated legacy
field. It was **removed**, the cost identity
`total = work + orchestration + verification + ci` became a bit-exact integer
theorem, and the schema version bumped. A `deny_unknown_fields` consumer fed a
1.1.0 envelope carrying `cost_usd` would reject it — which is the desired
behavior, because the integer/float meaning differs.

The money field's new home:
- `crates/hugit-contracts/src/context_envelope.rs:252` — `pub cost_usd_micros: u64` on `IntentMetrics`
- the same `*_usd_micros: u64` shape repeats across the `CostDecomposition` family
  (`context_envelope.rs:371,387,400,413,431,442`).

## The vectors that pin it — the de-laundered oracle

The freeze is not asserted, it is **pinned by a hand-authored oracle** that the
generator cannot launder. `crates/hugit-contracts/tests/golden_pins.rs:1-45`
documents the design: the generated golden suite is written AND verified by the
same serde derive (its own oracle), so `golden_pins.rs` adds **hand-typed JSON
string literals** that assert the exact serialized field-name SET and ORDER plus
security-critical literal values, with a `tripwire_*` per type proving the
producer's serialization byte-equals the hand-written literal.

For the cost break specifically:
- `crates/hugit-contracts/tests/golden_pins.rs:225-228` — `pin_schema_version_literal`
  asserts `CONTEXT_ENVELOPE_SCHEMA_VERSION == "1.2.0"` ("drifted from the hand
  pin (WA4 = 1.2.0)").
- `crates/hugit-contracts/tests/golden_pins.rs:60,193,211` — the hand-authored
  `ContextEnvelope@1.2.0` literal pins `"schema_version": "1.2.0"`, the exact
  ordered field-name set, and (line 211 comment) that "The money field MUST be the
  integer micro-USD name (WA4 / 1.2.0)".

A second, cross-crate freeze lives under `conformance/`: byte-frozen JSON vectors
plus their sha256 in `conformance/manifest.sha256`, including `IntentMetrics.json`
(the cost shape), `RunnerLease.json`, `FenceManifest.json`, and the four
lease-DTO vectors (`AcquireRequest/AcquireResponse/CloseRequest/CloseResponse`).
The `conformance/manifest.sha256` pins each vector's hash; the
`hugit-invariants`/x4 validator re-checks the exact vector set + two-space format
on any manifest change (see the repo `CLAUDE.md` gate note). The
`CHANGELOG.md` `[Unreleased]` "mirror the four canonical lease-DTO conformance
vectors byte-identical" entry records the anti-drift tripwire that keeps these in
lockstep with `corelink-runners`.

## Rules for a consumer / producer

1. **A new field on a frozen type is a version bump, never a silent add.** Bump
   the type's schema version (the `ContextEnvelope` model: a `pub const … _SCHEMA_VERSION`)
   and re-derive the hand pin in `golden_pins.rs`. Do not reach for
   `#[serde(default)]` to widen a closed type — that defeats `deny_unknown_fields`.
2. **A `deny_unknown_fields` reject is correct, not a bug.** If a producer sends a
   field the consumer rejects, the producer is off-contract or on a different
   version. Fix the version skew; do not loosen the parser.
3. **Money is integer micro-USD `u64` end to end** (1.2.0). Never reintroduce a
   float cost field on a frozen type — the cost identity is an integer theorem.
4. **Forward-compat lives in `hugit-http-contracts`, not here.** If you need
   "old renderer ignores a new field", that field belongs on a wire VM
   (serde-default), not on a frozen contract type.

## Honest caveats

- The "40 occurrences" / "169 occurrences" / "16 modules" counts are exact at the
  time of writing (`grep` over each crate's `src/`); they will move as types are
  added — the **invariant** (frozen ⇒ `deny_unknown_fields`; wire VM ⇒
  serde-default) is the durable contract, not the count.
- This doc describes the schema/serde contract only. The *deployment* status of
  the surfaces that carry these types (what is live vs hermetic) is tracked
  separately in `docs/review/2026-06-17-honest-delivery-audit-double-checked.md`
  and the repo `CLAUDE.md`; nothing here claims a live surface.
