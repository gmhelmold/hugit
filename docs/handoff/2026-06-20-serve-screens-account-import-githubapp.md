# Handoff — the 5 unrouted `/v1` read screens (account · import · github-app · org · profile)

**Date:** 2026-06-20 · **From:** hugit engine session · **To:** githugr TL / owner
**Status:** mechanically ready to wire; BLOCKED on a content-ownership decision (below).

## What this is

The build-wave study (`tasks/w4a924hdc.output`, dim. *serve-v1-gaps*) found FIVE
frozen `hugit-http-contracts` VMs that ship with no route in `hugit-serve`:

| Screen | VM | Path | Backing reality |
|---|---|---|---|
| account | `AccountVm` + `PatVm` | `GET /v1/me/account` | ~18 presentational fields + PAT *metadata* (token never served, ADR-0002). No log backing. |
| import | `ImportVm` + `ImportRepoVm`/`ImportStepVm` | `GET /v1/me/import` | Pure GitHub-import wizard copy + steps. Live import = GitHub App + mirror (**owner/infra-gated**). |
| github-app | `GithubAppVm` + App*RowVm | `GET /v1/github-app` (global) | Some projectable rows (agent/human PR counts, queue) + lots of install copy; live install state / `$`-saved = **owner/infra-gated**. |
| org | `OrgVm` | `GET /v1/orgs/{name}` | NEW path family; multi-repo-per-org aggregation = P2. |
| profile | `ProfileVm` | `GET /v1/users/{user}` | NEW path family; cross-repo activity = P2. |

## Why this is NOT clean engine work (the boundary)

Wiring the *handler + route* is hugit's lane (the `/v1` backend). But unlike the
24 already-served screens — which **project real log data** into their VMs — these
five are **predominantly presentational product copy** (pt-BR profile fields,
wizard steps, install prose) with little-to-no engine/log backing. Authoring that
copy as handler output is **product/frontend content work**, which per the
established scope (screen content belongs to the githugr lead; the first-snapshot
content was *owner-decided*, not engine-invented) is **not the hugit engine
session's to invent unilaterally**. The VMs themselves are already transcribed
byte-for-field from the githugr canonical source — the *content* lives there.

So: the engine session declines to fabricate a screen's worth of product copy.
This is a deliberate honesty call, not an omission.

## The decision needed (one of)

1. **(recommended)** The **githugr TL** authors the screen content (it is theirs),
   and either wires these handlers in a githugr-coordinated change, OR hands hugit
   the canonical content to transcribe. For `org`/`profile`, the TL must also
   confirm the exact `/v1/orgs/{name}` · `/v1/users/{user}` path strings (NEW path
   families needing fresh `two_tier_auth` + per-tenant gate scaffolding in
   `route()`).
2. **OR the owner authorizes** hugit to serve **honest-default placeholder**
   handlers now (clearly-empty/honest content, gated-live fields disclosed), to be
   replaced by the canonical copy later. If so, say the word and the engine session
   will wire all five against the frozen VMs (additive, no contract change).

## Ready-to-wire facts (so whoever picks this up moves fast)

- The VMs are FROZEN in `crates/hugit-http-contracts/src/{account,import,github_app,org,profile}.rs`
  (each carries a round-trip test with representative content). **Do not re-derive
  the shape** — serialize exactly; `github_app.rs` has f64 → PartialEq-only (no Eq).
- Handler shape (frozen seam): `pub fn build_<screen>(log: &EventLog, repo: &str) -> <Vm>`
  in `crates/hugit-serve/src/handlers/<screen>.rs`, mirroring `build_dashboard`.
- `/v1/me/account` + `/v1/me/import` are identity-scoped → mirror the
  `["v1","me","dashboard"]` arm in `server.rs::route()` (ME_DEFAULT_REPO,
  `two_tier_auth` + per-tenant read gate). `/v1/github-app` is global (auth before
  work). `/v1/orgs/*` + `/v1/users/*` are NEW path families (fresh scaffolding).
- Central-write (lead-owned at integration): `handlers/mod.rs` (pub mod + pub use),
  `server.rs` route arms. github-app's projectable rows (agent/human PR counts,
  queue) CAN come from the log; everything live-gated must use the documented
  honest default (CLAUDE.md: built ≠ delivered, never fake).

## Cross-refs

- Master plan: `docs/plan/2026-06-19-build-wave-master-plan.md` (PR-E).
- Study (full serve-gap analysis): `tasks/w4a924hdc.output`.
- Contract seam discipline: `docs/interop.md` (githugr VM is the canonical source).
