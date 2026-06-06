# Training/eval exclusion control (WP-X3 ④)

**Status:** binding control · committed under the X3 test crate ·
executed by `acceptance_x3::item_4_training_exclusion_control_and_audit`.

## Statement

Tenant context snapshots and session journals are **never-training-data**.
No context object and no journal object — at rest, in transit, or in any
export — may be used as training or evaluation data for any model, in-house
or third-party. This holds regardless of tenant consent state for operational
features; training/eval exclusion is an absolute property of the context and
journal object classes (whitepaper §9: "context snapshots … never training
data").

## Scope

- **context** snapshots (the short-horizon resume payload, D11/E5 surfaces).
- **journal** objects (tenant-private session journals, D11).

Both object classes are in scope. Operational access (resume, audit,
export-with-policy) is permitted; training/eval access is forbidden.

## Mechanism

1. Every access to a context/journal object is **classified** at the access
   boundary as `Operational` or `Training` (see
   `TrainingExclusionControl::classify_access`).
2. An `Operational` access is permitted.
3. A `Training` access is **refused** and an **audit-trail entry** of kind
   `access-classified-non-training` is appended, attributing the principal and
   recording that the class is marked never-training-data by this control.
4. No model is invoked by the control. The control + the audit trail it
   produces is the deliverable; the proof is the executed classification plus
   the asserted audit entry, not a training run.

## Audit trail

Refused (non-training-classified) accesses are not silent. Each appends an
`AuditEvent { kind: "access-classified-non-training", principal, detail }` to
the append-only audit trail, so any attempt to route context/journal bytes into
a training/eval path is attributable after the fact.

## Verification

`item_4_training_exclusion_control_and_audit` asserts:
- the control doc is present and **binding** (names context, journal, and
  never-training-data);
- an `Operational` access is permitted with no non-training audit entry;
- a `Training` access is refused AND produces the `access-classified-non-training`
  audit-trail entry.
