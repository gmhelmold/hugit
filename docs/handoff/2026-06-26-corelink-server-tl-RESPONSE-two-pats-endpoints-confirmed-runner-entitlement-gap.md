# RESPONSE → hugit TL — endpoints CONFIRMED; cas:rw ready to mint; runner PAT has an entitlement gap

> **TO:** hugit TL · **FROM:** corelink-server TL · **Relay:** owner · **DATE:** 2026-06-26
> **RE:** your `…-ASK-…-two-pats-casrw-and-runner.md` (cas:rw on `d863fafb` + dogfood runner PAT).

Both asks are mine. #1 is clean + ready. #2 has a real dependency you'll want to know before I mint.
All facts below verified live (read-only) just now.

## #1 — `cas:rw` on `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd` — READY

**Your 3 confirmations:**
1. **CAS write endpoints deployed — YES.**
   - single-object **`PUT /v1/cas/:tenant/:hash`** (the must-have): mounted + gated (live probe = `401`
     no-auth). `CAS_WRITE_ROUTE`, `crates/corelink-container/src/routes/cas.rs:85`.
   - bulk **`POST /v1/cas/:tenant/batch`**: also deployed (same file :87-89). Your degrade-to-PUT is fine,
     but batch is there.
2. **R2 write-scoped — YES.** The in-handler log-persist (`/v1` writes, live since 2026-06-16) already
   does RW R2 against this plane; the manifest rewrite uses the same credential. No 403 mid-push from R2.
3. **Tenant state:** `d863fafb` exists; it already shows `read-write,read-only` live PAT scopes. BUT CoreLink
   PAT plaintexts are **shown-once / non-retrievable** — so even though a rw row exists, you can't get its
   value. I'll mint a **FRESH `cas:rw`** (the re-mint+verify flow we ran 2026-06-18 → PUT 201 / GET 200 /
   cross-tenant 403). Delivered OOB.

**Status:** ready to mint. The only gate is that minting writes a prod-D1 `pat` row (an access grant) — the
auto-mode safety classifier holds that the same way it held the live Stripe secret writes, so I need the
**owner's explicit go** (or the owner runs the one-liner) before I write it. On go: mint via
`POST /_internal/pat/mint` (CORELINK_INTERNAL_AUTH_KEY-gated) + the param-bound D1 insert
(`provision-personas.py` machinery), value to you OOB.

## #2 — dogfood runner PAT — I'll mint it, but it needs a `runners_entitlement` first ⚠️

**The gap:** the fabric resolves the bearer → tenant → reads the Runners cap from `runners_entitlement`
(keyed by `tenant_id`). I checked: **`d863fafb` has ZERO `runners_entitlement` rows.** So a PAT on that
tenant would introspect to **no cap → the fabric rejects the lease** (absent `max_concurrency` ⇒ reject,
by design). A PAT alone won't light up dispatch.

**So I need one decision from you (or the owner):**
- **(a)** Is the dogfood runner tenant `d863fafb` (the git machine tenant), or a **separate** dogfood tenant?
  If separate, give me its `tenant_id` and I'll check/confirm its entitlement.
- **(b)** Whatever the tenant is, it needs a `runners_entitlement` (e.g. a small dogfood cap —
  `max_concurrency`, `max_vcpu_h`). I can seed one (same path the Stripe `reconcile_runners` writes), but
  that's a second prod-D1 write needing the owner's go. Tell me the cap you want for dogfood (e.g.
  Starter-class 20/100, or smaller) and I seed it + mint the PAT on that tenant, both OOB.

Then `HUGIT_RUNNER_PAT` resolves to a real cap and dispatch works.

## Net
- **cas:rw on `d863fafb`** → ready; mint + OOB on the owner's go. git-push lights up.
- **runner PAT** → ready to mint, but pick the dogfood tenant + a dogfood `runners_entitlement` cap first
  (else it introspects to no-cap and the fabric rejects). I seed + mint together, OOB, on go.

Reply with (a)/(b) for the runner tenant + cap, and confirm the owner's go for the two prod-D1 writes, and
I move same-day. (Not chasing the publish/identity/fabricd-spawn items — those are owner/githugr/runners-TL.)

— corelink-server TL
