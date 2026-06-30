# Go-live WP decomposition — FROZEN spec for fleet execution

> **Status:** 2026-06-30. Derived from the grounded pilot-readiness roadmap
> (`2026-06-30-kungfu-pilot-readiness-roadmap.md`, ~40% ready). This is the FROZEN WP register +
> the disjoint-wave plan the fleet executes. Bar: real GO-LIVE, real consequences → every WP is
> go-live-grade (security-first, complete, e2e-proven, honest), cold-verified, merged on CLEAN.

## The disjointness reality (the go/no-go)
The engine's central files — `server.rs` (router + me/* + auth), `cas.rs` (manifest writes), `git.rs`
(receive/upload-pack finalize) — are touched by MULTIPLE go-live WPs, so those WPs **cannot run in
parallel** (they'd collide). Decomposition: the **disjoint** WPs (own their own files/dirs) fan out
in Wave 1; the **central-file** WPs **sequence** in Wave 2 (each merges before the next), the lead
owning the delicate auth/write seams. Done WPs: authed-clone (#223 merged), epoch-0 (#224 CI).

## WAVE 1 — DISJOINT (parallel, worktree-isolated, no central-file collision)
| WP | owned files (disjoint) | frozen contract / spec | acceptance | dep |
|---|---|---|---|---|
| **W-CONTRACTDOCS** | `docs/contracts/` (new) | The 3 KungFu contract docs: #8 (schemas are `deny_unknown_fields` → versioned bumps NOT serde-default — corrects the prior reply; cite the 1.1.0→1.2.0 break); #6 (`outline_blob(Lang,&[u8])->Vec<SymbolItem{kind,name,line}>`, the frozen `as_wire_str` vocab, totality/empty-on-bad-input); #7 (`ObjectSource::get(oid)`, `resolve_blob_at_path`/`commit_root_tree` sigs, MAX_PATH_DEPTH, the "fact-derivation runs in KungFu's workload, not heavy engine reads" rule) | 3 docs, each grounded (cite the real structs/sigs); no code | — |
| **W-SKILLS** | `skills/hugit/`, `skills/hugit-worker/`, `skills/README.md` | The 2 SKILL.md (orchestrator + agent) per `docs/design/2026-06-30-hugit-skill-llm-interface-design.md`, techlead house style, verb table from `HUGIT_VERBS`, AP-1..AP-8 each, the honesty law + error/exit law encoded, honest about reserved/deferred | verb table == HUGIT_VERBS (no drift); both files ~spec sizes; honest | — |
| **W-SEARCHINDEX** | `crates/hugit-serve/src/handlers/search.rs` + a new `search_index` module | The code-search lazy progressive in-memory index per the 2026-06-29 design (single-thread-safe `Mutex` cache, built across queries, served O(1), HEAD-invalidated, fallback to the live bounded scan) — retires `CODE_SCAN_BUDGET` per-query scan | index-hit serves O(1); cold-fallback == today; bounded; honesty (real-or-absent) | — |
| **W-DIFFVM** | `crates/hugit-serve/src/handlers/{compare,pr_detail,commit_detail}.rs` + `diff.rs` | Wire a REAL `diff_vm` numstat (refs→trees via the git source) into compare/pr_detail/commit_detail; delete the stale "every DiffVm is an empty stub" comment; honest-empty with no git source / no parent; WALL-CLOCK bounded (the tree-diff DoS lesson) | real numstat renders; bounded; honest-empty fallback; the stub comment gone | — |
| **W-MCP** | `crates/hugit-mcp/` (new crate) | The MCP server exposing the 4 tools (claim-disjointness→`hugit-queue::AffectedSet`; land-status→`land/mod.rs` union+bisect; cost-attest→`/v1 insights`+fabric, hand-stamp omitted; liveness-probe→`/readyz`+bounded Bearer probe, refuses heavy reads). User-hosted, ships with hugit. Contracts in the design doc | the 4 tools, structured I/O per the contracts; cost-attest can't hand-stamp; liveness-probe refuses /search; wired to the workspace + gate | W-CONTRACTDOCS (the catalog) |

## WAVE 2 — CENTRAL-FILE (sequenced; lead owns the auth/write seams; each merges before the next)
| WP | central file | frozen contract / spec | acceptance | order |
|---|---|---|---|---|
| **W-IFMATCH** | `cas.rs` (`commit_cas_push_manifests`/`finalize_cas_delete`) + `state.rs` (boot guard) | `put_object_conditional(key,body,expected_etag)->412` on the R2Put trait; thread refs.json + oid-index.json ETags with retry-on-412 (mirror the event-log `put_conditional`) — the HARD pre-condition before `max_instances>1`; a boot/deploy guard refusing >1 instance unless wired | conditional PUT + retry-on-412; the >1-instance guard; tests for the lost-update race closed | 2.1 |
| **W-WEBHOOK** | `git.rs` (receive/delete finalize) + a new `merge_hook` emit module | `KungFuMergeEvent{repo,ref,base_sha,head_sha,touched_paths[],landed_at,actor,cv_hint}` (frozen, #72): compute `touched_paths` via base→head tree-diff at finalize; HMAC-signed SSRF-safe outbound emit fired AFTER finalize Ok (post-durable; a hook fault never regresses the push); OFF the accept loop (bounded queue) + at-least-once + idempotency `(repo,head_sha)`; an adversarial security review of the egress | event emitted post-durable; touched_paths correct; hook fault doesn't `ng` the push; SSRF/secret-leak/DoS audited | 2.2 (after IFMATCH — both touch the finalize path) |
| **W-METENANT** | `server.rs` (me/* routes) + `dashboard.rs`/`attention.rs` + a per-tenant repo index | `build_dashboard`/`build_attention` take the PRINCIPAL + scope to the CALLER's repos (a per-tenant repo enumeration: which repos does `clerk:{org}` own/access); the me/* routes pass the real principal (not ME_DEFAULT_REPO) — closes the cross-principal exposure (githugr seam #1) | me/* resolves to the caller; cross-tenant → only own repos; tests | 2.3 |
| **W-CACHE-GODPATH** | `server.rs` (response headers + `two_tier_auth`) + `authz.rs` | `Cache-Control: private, no-store` on every authed `/v1` response + unconditionally on any private-repo response incl SSE (githugr seam #18); REMOVE the dev-token operator god-path from the `/v1` read door (the owner's go-live decision — a real user gets no god-token; keep operator ONLY where bootstrap genuinely needs it, documented) | private responses uncacheable; no god-bypass on real-user reads; tests; the dev-token posture doc | 2.4 (after METENANT — both server.rs) |
| **W-PROVISION** | `bin/` (new provision bin) + `writes/verbs/` (`POST /v1/repos`) + `state.rs` | The one-shot provision-repo path: seed the `<tenant>/<repo>.json` event-log (carrying `repo.meta{visibility,owner_tenant}`) + git-ingest in ONE op; a real `POST /v1/repos` create/import write verb githugr calls as-the-user (git source or empty → provisioned + immediately live-readable); add to `HUGIT_SERVE_CAS_REPO` runtime (githugr seam #2, my blocker #3) | a freshly-created repo is push+clone live in one op; the write verb; e2e | 2.5 |
| **W-SHED-METRICS** | `server.rs` (accept loop) | A per-request concurrency guard (503 'busy' past N in-flight — sheds a burst instead of stacking the serial loop); `/metrics` (in-flight, p50/p99, per-route counters, decoded-cache occupancy) + a request-id structured log line | a burst sheds 503 not wedge; /metrics serves; cheap (single-thread) | 2.6 (after the other server.rs WPs) |
| **W-BUDGET-AUDIT** | a new `BudgetedSource` wrapper + the CAS-touching handlers | Establish 'every N-sequential-CAS-fetch path threads a wall-clock deadline' as a tested invariant; audit + bound the remaining direct walks (commit_detail, edit, compare, landing, releases, attention, insights, campaign) | each CAS-walk handler bounded; the invariant test | 2.7 (touches handlers — after DIFFVM) |

## External / owner (not fleet-buildable — tracked, parallel)
- **githugr-TL:** thread the per-session token into me/* (after W-METENANT); render-verify the killer-data + drive a real Clerk mint (cross-tenant proof).
- **CoreLink/Runners:** the off-box `/usage` cost source; the ws/dispatch fabric; the revoke-fail-closed confirm.
- **owner:** pilot org→tenant_id; the >1-instance call (after W-IFMATCH); first-public-cost hold.

## Convergence GATE (before declaring go-live ready)
After all WPs merge + a re-deploy: a COMPLETE adversarial security audit of the whole live surface
(not sampled) converged to zero-vuln + the real-user journey proven e2e from the public www + zero
incomplete-served. ONLY then is go-live declared.

— hugit TL
