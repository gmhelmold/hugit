# WP-E3 — status / badge compatibility emitter

squad E · S · sonnet · 50k · branch: wp/E3

## Charter
Make hugit checks visible to the existing GitHub ecosystem: emit check results
as GitHub statuses and a status badge that reflects true state within a stated
staleness bound, with honest degradation when the status API is down or rate-
limited. The ecosystem must never notice a seam.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E3; this WP owns items ① ② ③.)*

- **①** checks appear as GitHub statuses
- **② 🔧** badge reflects true state within a stated staleness bound; status-API
  down → last-known + observable staleness, never silent wrong
- **③(+)** status-API 429/5xx: retry w/ backoff → eventually true state; no
  stuck-pending; failures observable

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `CheckResult` — the check outcomes projected to GitHub statuses.
- `AppWebhooks` / GitHub App auth — the authenticated status-API channel.
- **B1** App skeleton (`hugit-app`): the webhook/event source whose check
  results E3 emits. Consumed as a frozen producer; not modified.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-mirror/src/status/` — status emitter, badge renderer, staleness-
  bound tracker, status-API backoff/retry, observable-failure surfacing.
- `tests/status/emit_*.rs`, `tests/status/badge_staleness_*.rs`,
  `tests/status/api_backoff_*.rs`.

## Dispatch packet
- This contract file.
- Frozen `CheckResult`, `AppWebhooks`/auth anchors; B1 event-source signature.
- Anchor: `crates/hugit-mirror/lib.rs` barrel exports `status`.
- Conventions: badge state carries an explicit **staleness timestamp/bound**;
  on API failure the badge shows **last-known + observable staleness**, never a
  silent wrong value; no terminal **stuck-pending** state.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Status emission (①)**: each `CheckResult` maps to a GitHub commit status
  (context = the check id) via the App auth client.
- **Badge staleness (②)**: badge reflects true state within a **stated**
  staleness bound (a published constant). When the status API is down, render
  **last-known** value **annotated with observable staleness** — never silently
  serve a wrong/stale state as fresh.
- **API 429/5xx (③)**: retry with **bounded backoff** until true state is
  emitted; the system **never stays stuck-pending** (a terminal failure is
  surfaced as an observable failure status, not an indefinite pending); failures
  are observable in the surface.

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` green on `wp/E3`.
- Owned items ① ② ③ red→green; failing suites committed first.
- Cold verification by non-author; security review at SEAL.
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (① ② ③) green.
- Zero writes outside Claims.
- Evidence bundle (status-appears proof, badge staleness-bound + API-down last-
  known proof, 429/5xx backoff→true-state no-stuck-pending proof) attached to
  SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs · items ①②③ red→green ·
deviations = none | waiver-ref.
