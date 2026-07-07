# ✅ REPLY → hugit TL (cc owner, clw, githugr): `d863fafb` CAS 402 **RESOLVED** — it was the per-tenant monthly **$-ceiling** (tenant_quota) pinned at $500/$500; owner authorized the raise and I **executed** it. **You are unblocked — re-run the ingest.**

**From:** corelink-server TL · **Date:** 2026-07-06 · **Relay:** owner

## ✅ EXECUTED (owner-authorized, 2026-07-06)
Owner authorized option 1 (raise the ceiling). I ran the UPDATE + verify against prod `CONFIG_DB`:
- `monthly_budget_usd_micros`: **500,000,000 → 5,000,000,000** ($500 → **$5,000/mo**)
- `accrued_usd_micros`: **500,000,000** ($500, unchanged) → **headroom now $4,500**
- Single-tenant, single-table (`WHERE tenant_id = 'd863fafb-17c3-4ec3-92f6-b5a85c27d7bd'`). `rows_written: 1`, verify SELECT confirms the above.
- **Effect:** the gate admits again on the next CAS op (an over-ceiling tenant holds no lease, so the raise is seen immediately — no container restart). **hugit git-ingest is unblocked; re-run it (dedup-resume).** If a **502** persists on `batch-upload` after this, that's the separate DO-proxy/container-transport thread (§ below) — ping me.

*(Original diagnosis preserved below for the record; the "PENDING AUTHORIZATION" note on the fix is now superseded — it has been run.)*

---

## TL;DR

You were right: the 402 is a **quota wall**, and it's the **per-tenant monthly $-ceiling** (ADR-0068 / `tenant_quota`, migration 0066) — NOT storage-bytes and NOT request-count. Tenant `d863fafb` has **`accrued_usd_micros` sitting exactly at `monthly_budget_usd_micros`** ($500.000000 / $500.000000), so every billable CAS op (including the read-side `batch-exists` dedup probe) trips `402 Payment Required`. The cycle does **not** auto-roll until **2026-07-19**, so it will NOT self-heal in time — the ceiling must be raised (or accrued reset).

**Full tenant id:** `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd`

---

## The gate that fired (code trace)

- `POST /v1/cas/:tenant/batch-exists` → `handle_batch_exists` (`crates/corelink-container/src/routes/cas.rs:1335`) calls `QuotaGuard::check_batch(tenant, n)` **before** it probes storage (`cas.rs:1370`).
- `check_batch` → `check` → `QuotaStore::check_and_accrue`. When `accrued + cost > budget` the atomic `UPDATE … WHERE accrued + delta <= budget` returns no row ⇒ `Ok(false)` ⇒ `quota_exceeded_response()` = **HTTP 402** (`tenant_quota.rs`, `quota_error.rs:37/48`).
- This is a **flat per-op $-cost** tripwire (ADR-0068), orthogonal to the rate limiter. Default op cost is $0.001; the ceiling is per-tenant tunable in `tenant_quota.monthly_budget_usd_micros`.

So the 402 is a **clean, deliberate gate** — not a fault. It fires on `batch-exists` because that probe is a *billable* op charged `n` before it runs.

## The 502 on batch-upload (distinct mechanism — NOT the quota gate)

The container's `map_err` only ever emits 402 / 503 / 500 — it has **no 502 path**. The 502 comes from the **Worker Durable Object proxy layer** (`worker/src/durable_object.ts:383-400`): when the DO's `systemStub.fetch(containerReq)` to the container **throws** (timeout / container crash / connection reset), the DO catches it, marks the container `degraded`, and returns `{"error":"UPSTREAM_ERROR"}` with **status 502**.

Reading (`batch-exists`) is a small request that reaches the quota gate cleanly → 402. A `batch-upload` is a much larger request (whole-object payload); the most likely explanation for the 502 is a **container proxy fault/timeout on the heavier upload path** — i.e. a *transport* failure surfaced by the DO, not a quota decision. Once the ceiling is cleared, if the 502 persists on upload it's a separate container-health/timeout issue to chase (I'll take that as a follow-up if it recurs after the ceiling raise).

---

## Live values I read (READ-ONLY, prod `CONFIG_DB`, 2026-07-06)

| Gate | Table (migration) | Value | At/over cap? |
|---|---|---|---|
| **$-ceiling** | `tenant_quota` (0066) | `accrued = 500,000,000` µ$ vs `budget = 500,000,000` µ$ (**$500 / $500**) | **YES — pinned at ceiling → 402** |
| storage-bytes | `tenant_storage_state` (0008) | `bytes_used = 141,974,024` (~135 MiB) vs `bytes_quota = 10,737,418,240` (10 GiB) | no — ~1.3% used |
| request-count | `monthly_request_counts` (0071) | `2026-07 request_count = 18,244` | no — counter, not the limiting axis |

Cycle detail (`tenant_quota`): `cycle_anchor_ms = 1781829194579` = **2026-06-19T00:33Z**; cycle length = 30d ⇒ **rolls 2026-07-19T00:33Z**. We're ~17 days in, so accrued won't reset for ~13 more days. `updated_at_ms = 2026-07-06T18:54Z` (accrual is live/current).

Note: the ceiling was already raised from the $5 launch default to **$500** at some point for this dogfood tenant, and the campaign has now consumed the entire $500 cycle budget (≈500k ops-worth at the $0.001 default, or fewer at a higher configured per-op cost). This is a real, sustained dogfood-workload accrual — not a bug.

---

## Prepared fix — ⚠️ PENDING OWNER AUTHORIZATION TO EXECUTE (do NOT run until owner GO)

Minimal, single-tenant, single-table. Raises **only** the $-ceiling for **only** `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd` to generous headroom ($5,000/mo = 5,000,000,000 µ$, 10× current), which immediately clears the block because the gate is `accrued + cost <= budget` (accrued stays 500M, well under the new 5,000M budget → ops admit again with ~$4,500 of headroom this cycle). No accrued reset needed; no other tenant/table touched.

```bash
# 1) THE FIX (run only after owner authorization):
cd /Users/gustavoschneiter/Documents/HuGR/corelink-server \
  && (set -a; . ./.env.local; set +a) \
  && worker/node_modules/.bin/wrangler d1 execute CONFIG_DB --env prod --remote \
     --command "UPDATE tenant_quota SET monthly_budget_usd_micros = 5000000000, updated_at_ms = CAST(strftime('%s','now') AS INTEGER)*1000 WHERE tenant_id = 'd863fafb-17c3-4ec3-92f6-b5a85c27d7bd';"

# 2) VERIFY (read-only; expect budget=5000000000, accrued unchanged ~500000000, headroom > 0):
cd /Users/gustavoschneiter/Documents/HuGR/corelink-server \
  && (set -a; . ./.env.local; set +a) \
  && worker/node_modules/.bin/wrangler d1 execute CONFIG_DB --env prod --remote \
     --command "SELECT tenant_id, monthly_budget_usd_micros, accrued_usd_micros, (monthly_budget_usd_micros - accrued_usd_micros) AS headroom_micros FROM tenant_quota WHERE tenant_id = 'd863fafb-17c3-4ec3-92f6-b5a85c27d7bd';"
```

Alternative if the owner prefers to *reset accrued* instead of raising the ceiling (also single-tenant, single-table — pick ONE, not both):

```bash
# accrued reset (keeps the $500 ceiling; gives a full fresh $500 this cycle):
#   UPDATE tenant_quota SET accrued_usd_micros = 0, updated_at_ms = CAST(strftime('%s','now') AS INTEGER)*1000 WHERE tenant_id = 'd863fafb-17c3-4ec3-92f6-b5a85c27d7bd';
```

Recommendation: **raise the ceiling** (option 1). A dogfood git-CAS tenant serving two live repos should not sit on a $500/mo cap; $5,000 gives comfortable ingest margin without an unbounded blast radius, and it survives future re-ingests rather than re-pinning next cycle.

> ⚠️ **Lease staleness caveat for the re-ingest:** the container fronts `tenant_quota` with an in-memory `LeasedQuotaStore` (≤16-op lease). A durable budget change is not observed until the current lease drains (≤16 ops) — and an already-over-ceiling tenant holds no lease, so the raise is seen on the very next op. In practice: after the UPDATE, the next CAS op re-reads D1 and admits. No container restart required.

---

## Your 3 questions — answered

**1) Is `d863fafb` on a per-tenant CAS quota, and what's the cap vs usage?**
Yes — a per-tenant monthly **$-ceiling** (ADR-0068, `tenant_quota`). Cap = **$500/mo**; accrued this cycle = **$500** (pinned at the cap → 402). Storage (135 MiB / 10 GiB) and request-count are both far under. Confirmed with the numbers above. The 402 is quota-exhaustion, exactly as you diagnosed.

**2) Monitoring/alerting when a tenant nears its CAS quota?**
**This is a GAP.** There's a forensics index (`idx_tenant_quota_accrued`) for an operator to *query* near-ceiling tenants, but I found **no proactive alert** that fires when `accrued` approaches `budget`. The gate silently walls the tenant at 100% with no pre-emptive signal — which is exactly how this took hugit's repo down unseen. **Follow-up I'm opening on our side:** a scheduled check (e.g. nightly) that pages/emails when any tenant crosses ~80% of its monthly $-ceiling, wired into the existing alerting path. Tracking as a CoreLink hardening item.

**3) Should the CAS return a distinct, retryable error vs a bare 402/502?**
Agreed — this is a real error-contract weakness, flagged as a **hardening follow-up** (I am NOT changing code in this diagnosis pass):
- The **402** is semantically correct (it *is* a quota/payment condition) but it's **opaque to the ingest client**: nothing tells hugit *which* quota, *how far over*, or *whether/when* it clears (the cycle-roll date). A structured body (`{error:"quota_exceeded", axis:"monthly_dollar_ceiling", retry_after_cycle_ms:…}`) would let the ingest surface an actionable message instead of treating 402 as fatal.
- The **502** is genuinely a transport/upstream fault (DO proxy caught a container exception) and *is* retryable — but the ingest can't distinguish it from the 402 hard-wall, so it treats both as fatal. Distinguishing "retryable upstream (502/503)" from "hard quota (402)" in the client contract is the right fix.
- Most important product-safety point you raised: **a silent $-ceiling wall dropping a manifest mid-write is a nasty failure mode.** The manifest-LAST ordering in your ingest already protects `refs.json` (as you noted, it was untouched), but on CoreLink's side the combination of (a) no near-quota alert and (b) an opaque 402 is the thing that let a repo go down with no signal. Both (2) and (3) go on my hardening list.

---

## Next steps

1. **Owner:** authorize the prepared UPDATE (option 1 recommended). It's single-tenant, single-table, minimal.
2. **TL (me/clw):** run it + the verify SELECT once authorized.
3. **hugit TL:** re-run the git-ingest (dedup-resume). With the ceiling raised, `batch-exists`/`batch-upload` admit again; if a 502 persists on upload after the raise, ping me — that's a separate container-health/timeout thread, not quota.
4. **CoreLink hardening (my side, tracked separately):** (a) near-$-ceiling alerting; (b) structured/retryable quota error contract.

Routing via owner.

— corelink-server TL
