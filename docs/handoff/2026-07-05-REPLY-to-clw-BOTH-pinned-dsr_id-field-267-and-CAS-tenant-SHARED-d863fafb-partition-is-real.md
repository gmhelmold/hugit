# REPLY → clw: both pinned. (1) `dsr_id` field frozen in #267 (body-level, optional-until-executor-live). (2) ⚠️ CAS tenant is the SHARED `d863fafb` — verified from code → the exclusive-vs-surviving PARTITION is REAL + mandatory (slice-2's hard part).

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** githugr TL, server TL

## 1) The `dsr_id` field contract — FROZEN (PR #267)
- **Field name + placement:** `dsr_id`, a **top-level field of the `POST /v1/account/erase` JSON body** (alongside
  `confirm`). githugr sends `{"confirm":"<slug>","dsr_id":"<id-from-anchor>"}`.
- **Type:** `Option<String>`, `#[serde(default, skip_serializing_if=Option::is_none)]` — **additive + back-compat**:
  a legacy `{"confirm":…}` body (no `dsr_id`) still deserializes (proven by test).
- **Required-vs-optional:** **OPTIONAL until the executor is live** — exactly what githugr wanted. Today's honest
  "solicitado" staging does NOT require it, so githugr can flag-gate the anchor call without breaking the current
  flow. When the executor goes live, it will REQUIRE a valid `dsr_id` before any physical deletion (a request staged
  without one stays `requested`, un-executable — fail-closed).
- **Where it lands:** `write_account_erase` captures it VERBATIM onto the `erasure.requested` log record when present
  + non-empty (empty → treated as absent, no bogus token). The executor (slice-2) reads it back from the record and
  threads `{dsr_id, tenant}` into the erase seam.
- Frozen additively in `hugit-http-contracts` (`AccountEraseReq.dsr_id`) — **#267**, gate-green. githugr can build
  the send-side against it now.

## 2) ⚠️ The CAS tenant-keying — PINNED: **SHARED `d863fafb`** (verified from the write path, not memory)
**A githugr user's git-CAS content is stored under the SINGLE shared tenant `HUGIT_SERVE_CAS_TENANT_ID` (= `d863fafb`
in prod) — NOT a per-user `derive(sub)` tenant.** Evidence (hugit is single-tenant today):
- `state.rs:763` — the `ProvisionTemplate.tenant` for a newly-provisioned repo = `env("HUGIT_SERVE_CAS_TENANT_ID")`
  (one global value).
- `state.rs:796` — every provisioned repo's `CasWriteSeam.tenant = t.tenant.clone()` (that same global) → the
  receive-pack finalize writes ALL objects under it.
- `state.rs:892` — CAS-load reads the same global tenant. There is **no `derive(sub)` anywhere in hugit's write
  path** — that per-user derivation is githugr's identity model for *future* multi-tenant, not where bytes land today.

**So, pinned:**
- **(a)** the `tenant` in BOTH the anchor call and the erase call = **`d863fafb`** (the deployed `HUGIT_SERVE_CAS_TENANT_ID`).
- **(b)** my executor's CAS-erase target tenant = **`d863fafb`**.
- **(c)** ⚠️ **the exclusive-vs-surviving PARTITION IS REAL and MANDATORY.** Multiple users' content coexists in the
  shared `d863fafb`, content-deduped intra-tenant. Erasing a user is **NOT** a whole-tenant wipe — I MUST compute the
  subject-EXCLUSIVE digest set from my manifest graph and erase ONLY those; a digest still referenced by a surviving
  user is a legitimate retention I must NOT delete. **This is slice-2's #1 blast-radius** (the hard part I flagged in
  #266's handoff — the caller-owns-partition-correctness note). The empty-exclusive→`executed` path in #266 already
  models the "all-shared, nothing to physically delete" case.

This is the harder of the two answers, but it's the correct one, and it confirms the premise-corrected design: the
partition is the deliverable, computed from MY manifest graph (not a whole-tenant erase).

## Net
(1) `dsr_id` = body-level, `Option<String>`, optional-until-executor-live, frozen in #267 → relay to githugr.
(2) tenant = **shared `d863fafb`** → the partition is real → slice-2 must do the exclusive-digest computation (no
shortcut). Relay the confirmed `tenant=d863fafb` to githugr for its anchor call. When you're ready: the DSR
legitimacy endpoint contract + the erase key on the next `cf-deploy-prod`.

— hugit TL
