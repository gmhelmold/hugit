# Master Plan — `hugit-serve` Wave 1 (the `/v1` HTTP engine port)

**Status:** contracts FROZEN + verified (PR branch `feat/hugit-serve-w1`); this plan
pins the build to the last detail so the fleet **cannot** guess. Source request:
`../githugr/docs/handoff/2026-06-13-hugit-http-server-request.md`. Wire oracle +
field sources cold-verified by a 7-agent read-only research fleet (2026-06-13).

---

## 0. The honest data-readiness finding (READ THIS FIRST — it changes the launch)

The window's VMs assume a LOT of data a HEADLESS git+forge engine does not locally
hold. Field-by-field source analysis (cold, against the code) classifies every field
as **REAL** (a real engine fn), **PRESENTATION** (adapter humanizes a real datum), or
**STUB** (no local source → honest default `""`/`null`/`0`/`[]`, never faked). The five
reads are NOT equally real:

| Read | Real-data verdict | What's REAL | What's STUB (honest) |
|---|---|---|---|
| **checks** | ★★★ strongest | local hit-rate KPIs (`aggregate_kpis` over `<log>.ac`), check rows (name/ok/duration/cache_hit/memo_key), cpill hash/from_pr | bisect/culprit (P2 — needs live AC+queue), cpill command/ago/runner + row log/cost (not in log payload), **fleet** KPIs `cache_hit_rate_pct`/`cache_saved_usd`/`runner_h` (P2 live AC) |
| **commits** | ★★★ strong | the commit rows (`project_machine`: message/sha/intent_id), branches | `age`/`avatar_class` (presentation), `checks_ok` (false unless `check.recorded` exists) |
| **home** | ★★ half | branch/branch_count/tag_count/branches, `last_commit`, `commit_count`, contributors-from-principal-chain | **file tree (no git-tree API → `files:[]`)**, readme_html, about.{description,topics,stars,forks,license,languages} (GitHub-mirror, P2) |
| **landing** | ★ cost-real, structurally sparse | PR list/state/queue-pos, campaign chips, **cost** (ledger `cost_usd_micros`→usd), intents | **`main_green`/`main_status` (NO local source)**, file_count, drawer.files, mirror (P2), union (real only when the P2 union-oracle is wired), list_groups, draft_count |
| **prs/{n}** | ★ cost-real, structurally sparse | number/title/state/author/session, campaign, **cost split** (ledger), intents, check_rows, envelope/CAS, why/acceptance (from envelope) | diff/added/removed/file_count (no diffstat seam), impact (blast_radius not wired), source/target_branch, reviewers, labels, mirror (P2) |

**Consequence:** `checks` + `commits` ship as genuinely-real `live`. `home` ships real-
but-sparse (no file tree). `landing` + `prs/{n}` render with REAL cost + PR projection
but MANY honest-empty fields (no diffs, no file counts, no main-CI status, no mirror) —
truthful per the fail-honest contract, but a sparse first `live`. The gap is engine
SEAMS that don't exist yet (a git-tree reader, a diffstat projector, a main-branch CI
status, the live union-oracle + mirror) — several are P2. **This is the cost of the
headless engine serving UI-shaped VMs; it is not a bug, it is the true state.**

**→ Owner decision required (§6) before the fleet builds landing/prs.**

---

## 1. Scope (Wave 1) + decisions (recap)
- 5 reads + Bearer **auth-stub** (real Clerk JWKS / RFC-8693 = ADR-0002 = **P2**).
- Sync HTTP server (tiny_http-class), **not** axum/tokio (supply-chain-strict, async-free
  workspace; the wire is server-impl-agnostic). SSE + writes = Wave 2.
- `hugit-serve` = thin adapter (domain→VM + humanize) OVER the domain-pure engine crates;
  the VM-coupling lives only here. `hugit-http-contracts` = the frozen wire types (DONE).

## 2. The verified read pattern (every handler uses it)
`hugit_cli::checks::load_event_log(path)` → `serde_json::from_slice::<Vec<EventRecord>>`
→ `rehydrate_and_verify` (push_record seq-check + `verify_chain`). A tampered chain →
`chain_broken` → **503 fail-closed**, never projected. (PS-13 chokepoint — reuse, never
re-hand-roll.)

## 3. The transport contract (cold-verified against `githugr-live`)
- Header: `Authorization: Bearer <token>` (exact; parity-asserted).
- Error body: **`{ "code": "<MACHINE_CODE>", "reason": "<pt-BR>" }`** (client reads ONLY
  these two; the spec's `seq?` is unread). 401→AuthExpired (no refresh loop in client),
  403+STEP_UP_REQUIRED, 403+other→Policy, 404→`Ok(None)`, 409→unavailable, 5xx→unavailable.
- The 5 reads are `get_opt`: **404 = not-found OR no-access (no existence leak)**; 200+JSON
  = `Some(vm)`; **200-with-garbage = engine fault** (detail must contain "deserialize").
- **No ETag asserted** by the client (spec says ETag=seq; optional — emit it, don't depend).
- `/readyz` for the window's readiness probe (`GET /v1/repos/{repo}/home` 2s, per request §2).

## 4. Crate + file-ownership map (DISJOINT — kills the shared-file trap)
`crates/hugit-serve/` (new): minimal sync server. The orchestrator (me) PRE-BUILDS the
shared skeleton; each endpoint is a disjoint handler file an agent fills.
- `src/server.rs` (ME) — sync HTTP loop, routing `/v1/repos/{repo}/{home,landing,prs/{n},checks,commits}` + `/readyz`, request dispatch.
- `src/auth.rs` (ME) — Bearer auth-stub middleware: validate `Authorization: Bearer <HUGIT_ENGINE_DEV_TOKEN>`; missing/empty→401 `TOKEN_INVALID`. (Real JWKS = P2, flagged in-code.)
- `src/error.rs` (ME) — the `{code,reason}` marshaler + the status map; `EngineErr` enum → (status, code, reason).
- `src/state.rs` (ME) — shared `AppState` (repo→log path resolution, the verified loader).
- `src/handlers/home.rs` · `commits.rs` · `checks.rs` · `landing.rs` · `pr_detail.rs` (FLEET, one agent each) — `fn build_X(state, repo[, n]) -> Result<Option<Xvm>, EngineErr>` mapping engine→VM per the §5 source map.
- `tests/parity_<x>.rs` (FLEET, with each handler) — boot server, GET with stub token, assert body parses through `hugit_http_contracts::Xvm`; 401 no-token; 404 unknown repo; (checks) honest-null KPIs on an empty log.
- `deny.toml` (ME) — add tiny_http's license(s) to the allowlist; resolve any
  multiple-versions exception. **Gate stays green.**

## 5. The per-field source map (the anti-guess anchor — agents build to THIS)
Each handler brief embeds its VM's table: every field tagged REAL `<fn>` /
PRESENTATION `<formula>` / STUB `<default>`. Engine entry points (verified):
- **refstore:** `replay(log)->RefState` (branches/tags via `refs/heads|tags/` prefix);
  `intent::projection::project_machine(log)->MachineHistory.rows()` (commit rows:
  intent_id/target-sha/message); `intents_from_log(log)->IntentLog`. **No git-tree API.**
- **checks:** `hugit_cli::checks::aggregate_kpis(rows)` (hit_rate_pct/hits/executed/saved_ms,
  honest-null); `CheckRow::from_payload` (name/ok/duration_ms/cache_hit/memo_key/pr_id).
- **queue/pr:** `hugit_cli::pr::{find_pr_opened,find_pr_queued,pr_state}`; `hugit_queue::core::{evaluate_union,landed_in_order,EntryState}`.
- **ledger:** `Ledger::from_records` (verdicts/proven/rejected); `rollup::pr_record`
  (`CostDecomposition.*_usd_micros`); **cost: `usd = micros as f64 / 1_000_000.0`**;
  `CampaignPrVm.cost_usd_micros` = pass-through (no division).
- **policy:** `authorize`/`matrix` (viewer-can/authz); house gates. **No `main_green` seam.**
- Humanize helpers (adapter, `src/fmt.rs`, ME): `humanize_age(unix_ms)->"há 8 min"`,
  `usd(micros)->"$0.34"`, `avatar_class(author)`, locale int `"4.832"`.

## 6. Owner decision (gates the fleet's landing/prs build)
Given §0, choose the Wave-1 fleet scope:
- **(A) Ship the 3 real reads first** (checks · commits · home), defer landing/prs until
  their seams (diffstat · main-CI · union-oracle) exist. Honest, fast, real `live` for 3
  screens. (Recommended — real data beats sparse data.)
- **(B) Build all 5 now** with disclosed honest-stubs on landing/prs (cost+PR real, the
  rest empty). githugr.com goes `live` on all routes but landing/prs look sparse.
- **(C) Build all 5 + open the missing-seam sub-WPs** (a git-tree reader, a diffstat
  projector, a local main-status) as part of this wave — bigger, makes landing/prs real
  too, but is a much larger build (some are P2-bound).

## 7. Verification (the DoD that makes "done" not "judged")
Per handler: (1) `cargo test -p hugit-serve --test parity_<x>` green — body parses through
the frozen contract type; (2) STUB fields assert the honest default (not faked); (3)
401/404/503 paths asserted; (4) `fmt`+`clippy -D warnings` clean. Workspace gate
(fmt/clippy/test/deny/audit) green before merge. The orchestrator cold-verifies each
agent's diff against this DoD + the §5 source map; nothing merges un-checked.

## 8. Fleet dispatch (AFTER §6 + the scaffold)
Pipeline, one agent per chosen endpoint: brief = the frozen `Xvm` type + its §5 field
table + the engine fns + the §7 DoD + "STUB exactly these fields to these defaults; invent
nothing; the parity test is your proof." Disjoint handler files → zero conflict. Each
return cold-verified by the orchestrator. Worktree isolation NOT needed (disjoint files).
