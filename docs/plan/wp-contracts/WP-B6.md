# WP-B6 — intent sidecar
squad B · S · sonnet · route sonnet · budget 50k · branch: wp/B6

## Charter
Build the intent sidecar in `hugit-app/sidecar`: parse/validate/render PR intent
metadata (charter + acceptance + context-ref) as a PR comment + check summary,
respond to malformed sidecars with an actionable comment, persist the corpus to
CAS keyed by intent_id, and guarantee the sidecar is non-authoritative (never
gates or blocks landing). This builds the intent corpus later phases and the
experiment gate consume.

## Owned acceptance
B6 owns all 4 items of B6 (no split). VERBATIM from decomposition v2.0 §2:

① parsed/validated/rendered · ② malformed→actionable comment · ③ corpus→CAS by
intent_id · **④(R2) negative: sidecar is non-authoritative — never gates or
blocks landing**

## Contract deps
Consumes from `hugit-contracts` (frozen): **IntentSidecar** (the schema with
`authoritative:false` + `intent_id`), **AppWebhooks** (PR comment + check-summary
write-back). Depends on B1's App skeleton. No contract type authored or changed
here.

## Claims
`crates/hugit-app/sidecar/` (sidecar parser/validator, renderer, CAS corpus
writer, the non-authoritative guard). Does NOT touch the rest of
`crates/hugit-app/` (B1) or `crates/hugit-app/ui/` (B7).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B6.md`).
- `hugit-contracts` (IntentSidecar, AppWebhooks) + B1's App skeleton.
- whitepaper §4 (Intent object — charter/acceptance/ctx; intent_id), §4.1
  (PROPOSED state — "an issue is simply an Intent parked here").
- command-catalog (intent sidecar: non-authoritative, builds the corpus the
  experiment gate needs).
- warp-10-days §Squad B (B6 deliverable; depends B1).
- The failing acceptance suite at `tests/acceptance/wp-B6/`.
Estimated packet size: ~34k tokens (inside 50k).

## Implementation notes
Every fork pre-decided:
- **Parse/validate/render (①):** parse the PR-attached sidecar into the
  `IntentSidecar` type; validate against its schema; render charter +
  acceptance + context-ref as a PR comment AND a check summary via AppWebhooks.
- **Malformed → actionable (②):** a malformed sidecar produces an actionable PR
  comment naming the specific validation failure (which field, what's expected)
  — never a silent drop, never a hard error that blocks the PR.
- **Corpus → CAS by intent_id (③):** the validated sidecar is written to the
  CoreLink CAS (`/v1/cas`) keyed/addressable by `intent_id`; `clw` is the
  reference client.
- **Non-authoritative (④):** the sidecar NEVER gates or blocks landing — assert
  the landing path (B4) reads no authoritative signal from the sidecar; its
  `authoritative` field is hard-`false` and there is no code path by which a
  sidecar failure holds a PR. The corpus is provenance, not a gate.
- **intent_id lifecycle:** the id minted here is the same logical intent id
  carried into phase D (X9③ guarantees identity; B6 mints, D4 natively
  reconstructs — one id, one lifecycle).
- **CoreLink consumed as CLIENT only** — zero server changes.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①–④ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
All 4 owned items green; zero writes outside `crates/hugit-app/sidecar/`;
evidence bundle (render sample, malformed-comment sample, CAS-by-intent_id
proof, non-authoritative assertion) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, render + malformed samples,
CAS key proof, non-authoritative proof), deviations = none | waiver-ref.
