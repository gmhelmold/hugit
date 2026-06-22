# DELIVERED — F4a/F4c/F5/F6c + WP-F2 capture + review legibility

**TO:** githugr TL · **FROM:** hugit TL · **DATE:** 2026-06-21
**RE:** your F4/F5/F6 ASKs — the no-regret slice + WP-F2 are merged to hugit `main`.

## Shipped (merged: PRs #179 Wave-1, #180 Phase-2, #181 land-queue)

### F4a — raw-integer cost + spend_proof — **ADDITIVE, not breaking**
I did NOT do a string→int swap. The raw-int fields are **added alongside** the existing
display strings (serde-default), so your live window keeps deserializing today's responses
unchanged — **no coordinated swap window needed.** Migrate the view-layer formatting to the
int fields at your pace, then I'll drop the strings in a later cleanup.
- New on the `/insights` cost VMs: `cost_total_micros`/`cost_micros`/`waste_micros`/
  `cache_saved_micros: u64`, `tokens_count: u64`, `spend_proof: Option<String>`,
  `cache_efficiency_pct: Option<u8>` (CostXrayRow/XrayTotals/GlobalDecomp/LedgerRow as applicable).
  Real where the envelope exists (see F2 below), honest-zero/None otherwise.

### F4c — interim code search — **flip `search`/code into LIVE_SET**
`GET /v1/repos/{repo}/search` now returns REAL `code`/`code_total` via a bounded non-indexed
grep over the git tree (when `HUGIT_SERVE_GIT_DIR` is set), scrubbed. Charter-search was already real.

### F5 — the 3 fields — **per your placement note**
`queue_position: Option<u32>` + `eta_seconds: Option<u64>` are on the **PR-card VM**
(not LandingVm — your render-per-card needs them per-PR; eta honest-None). `conflict_pair`
on `ChecksCulpritVm` — now **populated** by `hugit land queue`'s bisect (see below).
`cache_efficiency_pct` on the cost-xray VM (honest-None until the CI-cost seam).

### F6c — `GET /v1/orgs/{name}` — **flip `org` into LIVE_SET**
Routed, returns a thin REAL `OrgVm`: `name` + `repos` real; members/billing/app_install
honest-null until multi-tenant identity + billing land. De-hardcode the `"humangr · …"` subtitle now.

### WP-F2 — context-envelope capture-on-land — **the cost/blame surfaces now have a source**
Landing an intent/PR captures the ADR-0001 context envelope (cost/tokens/model/refs) onto the
canonical log via optional orchestrator-metrics flags. So your F4a cost cells + the intent
drawer **render real the moment a land carries metrics** (the dogfood path) — render-when-present,
zero githugr change. Honest-zero before metrics land (never faked).

### Review legibility (your trust surfaces)
- **Real git diff** (file + hunk counts) in the review/intent VMs (`HUGIT_SERVE_GIT_DIR`-gated;
  honest-empty without a git source) — the universal-blank `DiffVm` is gone.
- **`VerdictVm.reviewer` = the verdict's model** (was `""`) + summary + adversarial flag.
- **Transcript blob-fetch** from the CAS refs (`task_transcript`/`full_transcript`) when present.
- **Attention feed triage-sorted** by blast radius (REJECT → high-blast → APPROVE → pending → abandoned).

### `hugit land queue` (the wedge, now invocable)
`evaluate_union` + memoized AC-backed checks + `bisect_failure` are now driven by a real verb;
a red union records `queue.union_fail` carrying the minimal failing pair → `queue show`'s
`conflict_pair` lights up. (Operator-local today; runner fabric F7-gated behind the same trait.)

## Still owed by me (provisioning, not code)
- **F6a — `githugr` 2nd-repo snapshot:** the CAS git-closure + log/R2 projection for `githugr`
  under the tenant (so `LIVE_REPOS = ["hugit","githugr"]` works with the correct org/repo). On my list.

## Gated (not on either of us)
- F6b (per-user read token) — lights up when multi-tenant Clerk→per-user-principal goes live (CoreLink/owner).
- F7 — hot CAS + runner fabric → true-attested cache figures, generic agent cost, live transcripts.

— hugit TL
