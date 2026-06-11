# What hugit needs from CoreLink Runners — integration contract v1.2 (draft)

> **Version note — E-DOCS (2026-06-11):** this file records the §12 amendment
> required by WA4: `cost_usd|f64` → `cost_usd_micros|u64` in the
> `IntentMetrics` money field (owner-ratified 2026-06-11, contract 1.2.0
> SHIPPED on the hugit side). The change propagates the integer micro-USD
> decision to the corelink-runners integration spec and to the githugr
> delivery manifest. The §12 amendment-log entry that WA4's break skipped is
> added here and must be applied to the live contract in corelink-runners.
>
> **How to apply:** in `corelink-runners/docs/spec/hugit-integration-contract.md`,
> edit the §13.1 table row for `cost_usd` and append a v1.2.0 row to the
> Amendment log (§12). See the diff description below.

---

## Amendment — §13.1 money field: `cost_usd_micros|u64`

### What changed

`IntentMetrics.cost_usd` was `f64` (floating-point USD). WA4 (CHANGELOG entry
"WA4 money as integer micro-USD") renamed and re-typed this field across the
`hugit-contracts` frozen type, 9 fields in total, with exact-integer cost
identity and checked_add fail-closed. The frozen contract is now at schema
version **1.2.0** (additive — all other fields unchanged).

The wire representation:

| Before (1.1.0) | After (1.2.0) |
|---|---|
| `"cost_usd": <f64>` | `"cost_usd_micros": <u64>` |
| e.g. `"cost_usd": 0.001234` | e.g. `"cost_usd_micros": 1234` |

The unit is integer micro-USD: 1 USD = 1,000,000 cost_usd_micros. This is
exact-integer representation; f64 epsilon drift at scale is eliminated.

### Edit to §13.1 table in the live contract

Replace the `cost_usd` row:

```
| `cost_usd` | `f64` | derived COGS in USD (NEVER a billable meter; for trust/audit) |
```

with:

```
| `cost_usd_micros` | `u64` | derived COGS in integer micro-USD (1 USD = 1,000,000 units; NEVER a billable meter; for trust/audit; exact-integer, no f64 epsilon) |
```

Also update the schema version reference in §13.1 from `"schema 1.1.0"` to
`"schema 1.2.0"`.

### §12 amendment-log entry to append

| Version | Date | Author | Summary |
|---|---|---|---|
| v1.2.0 | 2026-06-11 | hugit techlead (E-DOCS) | §13.1 money field rename: `cost_usd\|f64` → `cost_usd_micros\|u64` (integer micro-USD, 1 USD = 1,000,000 units). Owner-ratified 2026-06-11 as part of WA4 (CHANGELOG). This is additive — all other §13.1 fields and all of §0–§12 are unchanged. The §13.4 conformance-vector drift tripwire applies: new conformance vectors must be committed byte-identical in both repos before the amendment is considered applied. |

### §13.4 conformance-vector update

The conformance vector manifest at `../hugit/conformance/` must include a
vector for the renamed field. Hugit's golden suite already pins
`cost_usd_micros` at the new integer type (WA4, hand-pinned goldens). The
corelink-runners side must update its pinned vector to match before the seam
is considered fully propagated.

---

## Amendment log (this document)

| Version | Date | Author | Summary |
|---|---|---|---|
| draft-1.2.0 | 2026-06-11 | hugit techlead (E-DOCS) | Records the §12 amendment for `cost_usd_micros` rename. Apply to `corelink-runners/docs/spec/hugit-integration-contract.md` on branch `integ/seed-runner`. |

*The live contract in corelink-runners is the source of truth; this file is
the hugit-side draft + rationale record. Mark APPLIED once the corelink-runners
commit lands.*
