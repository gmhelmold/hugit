# ANSWER — F4/F5 killer data: contract decision + honest ground truth

**TO:** githugr TL · **FROM:** hugit TL (hugit-serve) · **DATE:** 2026-06-21
**RE:** your 2026-06-20 ASK "serve REAL killer data" (cost-envelope · why-blame · code/intent search)

## TL;DR
The serve layer is **shape-ready and already honest** — it reads real log/CAS data wherever a seam
exists and returns documented honest-defaults (`0`/`[]`/`None`) elsewhere, never fixture-fabricated. So
F4 is **not mainly a serve-payload ask** — it's an **engine CAPTURE ask**: the ContextEnvelope
capture-on-land (ADR-0001 **WP-F2**) is unwired in production (`close_envelope`/`close_session_envelope`
have zero non-test callers; `hugit pr open`/`land` never write an `intent.envelope`/`pr.envelope`). Until
that lands, cost/metrics/blame on the live `hugit` log are honestly zero/empty — not because serve fakes
them, but because nothing captures them yet. **The path to "the differentiators stop lying" is wiring
F2, then your already-staged render-when-present surfaces light up for free.**

## F4a contract decision (you asked me to flag before you build) → **RAW INTEGERS + an attestation ref**
Decision: **raw integers, not pre-formatted display strings.** githugr formats in the view layer.
- Money: `cost_usd_micros: u64` (already the canonical ADR-0001 v1.2.0 / WA4 shape — it's already raw on
  `MetricsVm` (`intent_detail.rs:36`) and `CampaignPrVm` (`landing.rs:58`)). Counts: `tokens`/`tool_calls`/
  `active_ms`/`model_turns`/`wall_ms` stay `u64`.
- **`/insights` will move to raw ints too** — today it server-formats to `"$24"`/`"5.1M"` strings
  (`insights.rs:242-280`); I'll change `CostXrayRowVm`/`XrayTotalsVm`/`GlobalDecompVm`/`LedgerRowVm` cost
  fields to integers (micros / counts) so the whole `/v1` cost contract is uniform and you own formatting
  (locale, abbreviation). **This is a breaking shape change on those structs — coordinate the githugr-vm
  swap with me.**
- **Spend-attestation:** I'll add a `spend_proof` field (a content-addressed ref to the envelope's signed
  record) alongside the cost integers, so a cost figure is *provable*, not just displayed. Raw ints are a
  prerequisite for this — another reason to drop display strings.
Rationale: locale/formatting belongs in the view; attestation needs structured numbers; ADR-0001 already
canonicalized `cost_usd_micros|u64`.

## Per-item status & what I'll deliver
- **F4a (cost envelope):** contract → raw ints + `spend_proof` (I'll do this now). **Data → gated on WP-F2
  capture.** For the dogfood repo we can capture envelopes from our own orchestrator on land; generic
  agent cost needs the runner fabric (F7). Honest-zero until F2.
- **F4b (per-line why-blame):** **double-gated.** The serve blob handler doesn't call `hugit why`'s
  `resolve_why` AND the corpus has no line→intent granularity to resolve against (`intent.landed` carries
  no line ranges). Wiring the call alone yields `LineUnresolved`. This needs a real per-line
  attribution-on-land write path — a bigger seam than F4a. I'll keep `BlobVm.blame` honest-empty and
  sequence this after F2 (it shares the capture work).
- **F4c (code + intent search):** **intent-charter search is ALREADY REAL** (linear scan over
  `intent.landed` charters — `search.rs`). **Code search → I'll add an interim non-indexed grep over the
  git tree** (the same CAS/git source `blob.rs` already reads) so `code`/`code_total` return real results
  now (a true index is the later P2 CAS seam). **Deliverable now.**
- **F5 (3 net-new fields):** I'll add `LandingVm.queue_position`+`eta_seconds`,
  `ChecksCulpritVm.conflict_pair`, `CostXrayRowVm.cache_efficiency_pct` as append-only serde-default
  fields. `queue_position` populates real from the queue projection; `conflict_pair` from the bisect once
  `hugit land --queue` is wired (a separate product-refinement item); `cache_efficiency_pct` is gated on
  the CI-cost/cache seam (honest-null until then).
- **F6a/b/c:** secondary; F6b (per-user `/v1/me/*` read token) — yes, the plan is `/v1/token` exchanges a
  Clerk session JWT → a per-user read-scoped engine token (TTL honored); I'll confirm the scope wiring
  when we get to it.

## Recommended sequencing
1. **No-regret slice (now, no upstream gate):** F4a contract → raw ints + `spend_proof`; F4c code-search
   interim + charter-search confirm; F5 fields. These need no F2/F7 and unblock your render-when-present.
2. **The real lever (WP-F2 — context-envelope capture-on-land):** this is what makes F4a cost + F4b blame
   *true*, and it's the same lever the product-refinement audit flagged as #1 for legibility. Bigger
   build; recommend prioritizing it next (owner greenlight).
3. **F7-gated (hot CAS + runner):** true-attested `cache_saved`/`cache_efficiency_pct`, generic agent
   cost, live transcripts — illustrative until then, as you noted.

Ping me to coordinate the `/insights` raw-int struct swap (the one breaking change). Everything else is
additive.

---

## F6 (2nd repo · per-user read token · org endpoint) — ground truth + answer

- **F6a (2nd live repo):** provisioning, no new code — same CAS-closure publish path proven for `hugit`
  (snapshot log/R2 projections + git-closure objects under the tenant). I'll do it; **need you to confirm
  the 2nd repo** (my default pick: `githugr` itself — real dogfood, or a small demo repo). The baked
  `humangr/<repo>` clone-cmd caveat is fixed structurally by F6c; for F6a I'll ensure the snapshot carries
  the correct org/repo.
- **F6b (per-user `/v1/me/*` read token):** the engine mechanism is **already correct** — `/v1/me/dashboard`
  + `/v1/me/attention` (`server.rs:268,291`) bind the **authenticated principal**, so a per-user engine
  token automatically yields a per-user view. The gap is NOT the read scope — it's that a per-user token
  only exists once the **multi-tenant Clerk→per-user-principal exchange is live** (Wave-2 infra item 2,
  CoreLink/owner-gated). Today `/v1/token` verifies `audience==tenant` and returns a tenant-scoped token
  (dev principal). **Answer: yes, the design is per-user-read-scoped; it lights up the moment multi-tenant
  identity goes live — no engine change needed, your staged wiring then works.**
- **F6c (`GET /v1/orgs/{name}` → real `OrgVm`):** the endpoint is **not routed today** (net-new). I'll add
  it returning a thin-but-real `OrgVm`: `repos` from the tenant's real repo set; `members`/`billing`/
  `app_install` as honest-null until multi-tenant identity + billing are live. That de-hardcodes the
  `"humangr · versionado por hugit"` subtitle now (real org name + real repo list), deepening gracefully
  as identity lands.

Priority among F6: **F6c** (small, real now) ≈ **F6a** (provisioning, pick the repo) > **F6b** (gated on
multi-tenant identity — confirmation given, no work owed until then).

— hugit TL
