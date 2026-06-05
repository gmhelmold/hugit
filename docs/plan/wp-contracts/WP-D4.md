# WP-D4 — intents native + projection
squad D · size M · model route opus · context budget 80k · branch: wp/D4

## Charter
Make Intent objects native over the D1 event log and project them to git: every
landed intent deterministically emits generated commits embedding its
`intent_id`, reproducible from the log. The intent altitude (`hugit log`) and
the machine altitude (`git log`) are two zooms of one store and can never
disagree. Sidecar corpora import by intent_id; interleaved raw pushes stay
external-change, never intents.

## Owned acceptance (VERBATIM from decomposition v2.0 — D4)
> ① commits embed intent_id, reproducible from log
> ② two altitudes consistent (50-intent fixture)
> ③ sidecar corpus importable
> ④(+) mixed fixture (intents + raw pushes interleaved on one ref): altitudes stay provably consistent, externals as external-change

(D4 is NOT split — this WP owns the complete D4 item set ①②③④.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- D1 (`hugit-refstore`) append-only log + `EventRecord` — intents are events;
  projection reads the log. Consumed frozen; not modified.
- `IntentSidecar` (frozen Day-0 type) — the corpus imported by intent_id (③);
  produced by B6, stored to CAS by intent_id. Consumed frozen.
- D3b's external-change events — the raw-push externals the mixed fixture (④)
  interleaves. Consumed read-only; not modified.
- Projection commit bytes land in CAS via the CoreLink CAS surface (frozen
  external; CoreLink tenant only, zero server-side changes).

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-refstore/src/intent/model/` — native Intent objects over the log.
- `crates/hugit-refstore/src/intent/projection/` — deterministic intent→commit
  projection (generated messages embedding intent_id; two-altitude history).
- `crates/hugit-refstore/src/intent/import/` — sidecar-corpus import by intent_id.
- `crates/hugit-refstore/tests/intents_projection/` — owned acceptance suite
  (50-intent consistency fixture; mixed intents+raw-push fixture).
- No writes under `crates/hugit-refstore/src/{log,replay,tamper,compaction,
  coldtier,recovery,undo,concurrency}/` (D1a/b/c) or `crates/hugit-proto/` (D2/
  D3) or any other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §4 (object model +
  projection rule: "Same store, two zooms; they can never disagree because one
  is derived from the other"), §4.1 (intent lifecycle); `docs/product/command-
  catalog.md` (Intents native — first-class provenance OVER git; raw push =
  opaque change-events, never fake intents); `docs/plan/decomposition.md`
  §4 (D4 row) + §6/§8 (no-fake-intents adjudication D3⑤+D4④+E2⑤);
  `docs/plan/warp-10-days.md` D4 row.
- Anchors: generated commits embedding intent_id; intent altitude derived from
  the log; raw pushes stay external-change; one store, two zooms.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; DCO + CHANGELOG `[Unreleased]`; cold-verify by a
  non-author agent.
- Token estimate: ~75k (≤80k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **Projection is deterministic and one-directional:** every landed intent
  emits git commits whose generated messages EMBED the `intent_id`; the commit
  set is reproducible from the log (①). git is the derived projection of the
  intent log — never the other way (no reverse-engineering commits into intents).
- **Two altitudes, one store (②):** `hugit log` (intent altitude) and `git log`
  (machine altitude) are both derived from the same event log; they are
  PROVABLY consistent on a 50-intent fixture — they cannot disagree because one
  is derived from the other.
- **Sidecar corpus import (③):** the `IntentSidecar` corpus (B6, CAS-stored by
  intent_id) is importable by intent_id into the native intent model — one
  lifecycle, one id (cf. X9③).
- **Mixed fixture / externals stay external (④):** on a single ref with intents
  and raw pushes interleaved, the altitudes stay provably consistent and raw
  pushes remain EXTERNAL-CHANGE events — NO intent is synthesized for them. This
  is the D4 leg of the on-record no-fake-intents adjudication (D3⑤+D4④+E2⑤).
- Raw pushes are produced by D3b's write path; this WP consumes them read-only
  and asserts they project as external-change, never fabricates intents from
  them.

## DoD (global)
fmt + clippy + test + audit green · owned items (①②③④) red→green · cold-verify
pass by a non-author agent · DCO + CHANGELOG discipline.

## Completeness
All owned items green · zero writes outside claims · evidence bundle
(intent_id-embedded + log-reproducible commit proof; 50-intent two-altitude
consistency proof; sidecar-import proof; mixed-fixture consistency + externals-
as-external-change proof) attached to the SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned items red→green) · evidence refs (acceptance run, projection
artifact, two-altitude artifact, import artifact, mixed-fixture artifact) ·
claims-respected assertion · deviations = none | waiver-ref.
