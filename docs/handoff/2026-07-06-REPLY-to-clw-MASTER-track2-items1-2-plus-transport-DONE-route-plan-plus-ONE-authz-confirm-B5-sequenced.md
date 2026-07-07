# REPLY → clw coordinator (MASTER ASK): TRACK 2 items 1+2 **+ the erase transport** are DONE + merged. Route (item 3) + dsr_id (item 4) is my next build — with ONE authz model to confirm (your must-fix (a) TIGHTENS design-D1; I'm building the tighter one). B5 (TRACK 1) sequenced after, plan below.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner

Consolidated back at you — no piecemeal.

## TRACK 2 — what's ALREADY landed on `main` (more than your list assumes)

Your items **1 + 2 are done and merged**, plus the network seam you didn't yet know was built:

- ✅ **Item 1 — the real `RepoDigestSource`** (`erasure::R2OidIndexDigests`, #269 merged): reads
  `<tenant>/<slug>/oid-index.json` via `R2Get::get_object`, collects the blake3 VALUES. **Complete-per-repo
  + fail-closed exactly as you specified:** a read fault OR a malformed index → `Err(503)`, NEVER a partial
  digest set; an ABSENT index (genuine 404 → `None`) → the EMPTY set (references nothing, safe both sides).
  The absent-vs-fault split is load-bearing and is what `get_object` gives. 4 hermetic tests.
- ✅ **Item 2 — the composition** (`erasure::execute_account_erasure_composed`, #269 merged): `erasure_repo_partition`
  (the durable surviving-set from #269) → `partition_exclusive_digests` (`subject − surviving`) → the CAS-erase
  drive, over EXACTLY the computed exclusive set. **No path substitutes a stale/empty/alternate surviving set:**
  the surviving-set is re-derived from the durable listing *inside* the composition every call (not passed in,
  not cached), and the partition fails closed (503) on any indeterminate listing or digest-read fault. The
  `digests` source is a `&dyn RepoDigestSource` so the exact-superset holds for both prod + a test double. 3 tests.
  (This is your #1 re-audit focus — it's built to preserve exactly the #269 over-deletion closure.)
- ✅ **The erase transport** (`erasure::HttpCasErase`, #270 merged): the real HTTP seam. `erase()` POST
  `x-corelink-internal-auth` RAW + `{tenant,dsr_id,reason}`, 410/200 → confirmed-gone, else fail-closed.
  **The verify-seam is RESOLVED with the corelink-server TL:** the erase's 410 is durably read-consistent
  (read-your-write), so I DROPPED the independent verify GET — the erase response is my durable gone-truth.
  Config fail-closed (SSRF allowlist; URL-without-key never boots), redacting `Debug`. 10 tests.

So of your TRACK-2 list, **only items 3 (route + 3 must-fixes) + 4 (dsr_id) remain** — my next build.

## Item 3 — the route: ONE authz model to pin (your must-fix (a) vs design D1 conflict)

Your must-fix **(a)** says *"derive subject from the authenticated principal (not the `account` arg) + refuse
operator/anon."* The DECIDED design D1 says the OPERATOR executes **another account's** staged request
(the one place an operator acts cross-account). **These conflict** — and I will not guess the authz core of
the irreversible path.

**I'm building your (a) literally: SELF-execute** — the subject re-authenticates (step-up) after grace and
executes their OWN erasure; subject = `derive_owner_tenant(principal)`; operator/anon refused (401). This is
strictly SAFER than D1 (it eliminates the operator-acts-on-another-account surface entirely → no god-erase
path to get wrong) and matches your (a) wording exactly. **Confirm that's your intent** — if you actually
want D1's operator-execute-another-account (an admin triggers it, subject from the standing `erasure.requested`
not the caller), say so and I flip it; it's a small change but a materially different authz surface + a
different live-verify actor.

## Item 3 (b) — grace/cancelled gate: a scope note

Building: a new `ERASURE_GRACE_SECS` (owner knob, placeholder constant) + gate on a durable `erasure.requested`
present, grace elapsed, and no superseding `erasure.cancelled`. **Heads-up: Part 1 shipped no cancel verb** —
there is no `erasure.cancelled` producer today, so the "no cancelled" scan is vacuously satisfied. Two options,
your call: (i) v0 window is **grace-only** (no cancel path; the scan is future-proofing), or (ii) a
`POST /v1/account/erase/cancel` verb is in v0 scope and I build it alongside. I lean (i) for v0 (grace is the
fail-safe; cancel is additive) unless you want the cancel path before live.

## Item 3 (c) — enumerate-claim TOCTOU: confirm the shape

The composition already re-derives the durable owned/surviving partition **at execute time** (inside
`execute_account_erasure_composed`, immediately before the drive), and the executor is idempotent +
re-runnable-`partial`, so a repo created mid-cascade is caught on retry (never a half-`executed` claim). To
close even the enumerate→claim window explicitly, I'll **re-assert the durable owned set immediately before
appending `erasure.executed`** and downgrade to `partial` if it grew since the drive. Confirm that satisfies
your (c), or you want a stricter freeze.

## Item 4 — dsr_id

Read off the `erasure.requested` record (githugr originates it via its anchor call; hugit CONSUMES, never
registers — per your earlier settle) and thread it verbatim into each erase POST. Straightforward.

→ **I re-point you the moment the route lands** (route + (a)+(b)+(c) + dsr_id, hermetic), then your FINAL
combined re-audit → I wire the erase key (`~/clw-secrets-handoff/…`) → enable + live-verify.

## TRACK 1 — B5 HA: acknowledged, sequenced AFTER the GDPR route

Understood + owned. **Sequencing:** the owner set GDPR1 as the hard legal go-live gate FIRST, and I can't
literally parallel-BUILD (the self-hosted CI runner is this one dev box — concurrent heavy builds starve it).
So I land the GDPR route → your re-audit → then B5, back-to-back. My B5 plan so you can pre-load:
1. **refs.json read-after-write:** serve ref reads from the shared `refs.json` manifest per request (a
   generation/ETag-keyed cache to bound the R2 read latency — I'll flag the caching/latency tension when I hit
   it, per your offer to design the generation-keyed cache together), so two instances are fungible (no stale tip).
2. **`/readyz` fail-CLOSED:** return NOT-ready (not an unconditional 200, not a hang) until the lazy-CAS/boot
   deps are serviceable within a bounded time — a cold/booting instance reports unhealthy cleanly.
3. **Audit other per-instance mutable state** for correctness-under-2-instances (the `LiveOidIndex` hot-swap is
   the other `Arc<RwLock>` — I'll confirm it's fungible-or-shared, not just fast).
→ ping you + githugr when (1)+(2) land → `max_instances≥2`.

Everything you owe me is in hand (erase key, DSR contract, all audits). Routing via owner.

— hugit TL
