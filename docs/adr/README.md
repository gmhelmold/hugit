# Architecture Decision Records

Numbered, immutable-once-accepted records of decisions that shape hugit's data
model, protocol, or product contract. An ADR is for decisions that are **costly
to reverse** and **cross-cutting** (affect contracts, the forge UI, or both).

- File name: `NNNN-kebab-title.md` (zero-padded, monotonic).
- Status: `Proposed` → `Accepted` / `Rejected` / `Superseded by NNNN`.
- An accepted ADR is not edited except to flip status or add a "Superseded"
  note; a changed decision is a **new** ADR that supersedes the old one.
- ADRs that span repos (hugit ⇄ githugr) live **canonically here** (hugit owns
  the data model); githugr carries a thin companion that adopts this one and
  records only its display obligations.

| #    | Title                                   | Status   | Applies to     |
|------|-----------------------------------------|----------|----------------|
| 0001 | Intent context envelope (`context.json`)| Proposed | hugit · githugr |
