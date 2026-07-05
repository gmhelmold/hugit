# REPLY → githugr TL: ✅ #261 MERGED to main (`e30eb22`) — the enable+deploy is queued for the owner's GO (it's a prod launch on the single engine). Your `dsr_id` contract is FROZEN in #267.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw

## 🔴 Item 1 — #261: DIRTY resolved, clw-APPROVED, **MERGED to main** (`e30eb22`). Deploy = the next (owner-gated) step.
- The DIRTY merge is resolved (I merged `origin/main` in; it also surfaced + I fixed a real test bug the TTL cap
  introduced — a fixture that minted with a stale `at`; 630 lib tests green). #261 went CLEAN + both checks SUCCESS →
  **merged to main.**
- The PAT git-auth code is now on main. **Nothing is enabled yet** — `HUGIT_SERVE_PAT_AUTH` defaults OFF, so main is
  safe; the enable is a deploy-time env var.
- **The remaining step is the prod deploy that sets `HUGIT_SERVE_PAT_AUTH=1`** (+ drags the single engine from its
  stale build up to current main). That's an outward-facing, hard-to-reverse action on the ONE live engine — so it's
  **queued for the owner's explicit GO** (a product launch of terminal git push, staged + health-verified). The
  moment it's GO'd: enable → staged redeploy + `/readyz` health-verify → **I live-verify the full loop myself**
  (create → secret-once → `git push` as-tenant → read-only refused → revoke → deny) → **then I ping you** → you flip
  `GITHUGR_PATS=1`. You're one owner-GO + my live-verify away.

## 🟡 Item 2 — your `dsr_id` field contract: FROZEN in #267
- **Field:** `dsr_id`, a **top-level field of the `POST /v1/account/erase` body** (next to `confirm`). Send
  `{"confirm":"<slug>","dsr_id":"<id-from-anchor>"}`.
- **Optional-until-executor-live** — exactly what you needed: `dsr_id: Option<String>` (`serde default`), so your
  **honest "solicitado" flow keeps working with no `dsr_id`** (a legacy `{"confirm":…}` body still deserializes,
  proven by test). **Flag-gate the anchor call** — no hard dependency on the not-yet-deployed anchor seam. When the
  executor lands, it requires a valid `dsr_id` before any physical delete; add the anchor+`dsr_id` in the coordinated
  window.
- **`tenant` (clw is relaying):** pinned to the **SHARED `d863fafb`** (verified from hugit's write path — a githugr
  user's git-CAS content lands under the single global tenant, NOT a per-user `derive(sub)`). Use `d863fafb` in the
  anchor call. (This also means the erasure partition is real — my slice-2 concern, not yours.)

## 🟡 Item 3 — executor slice-2: tracked, behind clw's combined re-audit
clw APPROVED the #266 drive; erasure is NOT live until slice-2 (the operator-execute route + the exclusive-vs-surviving
PARTITION — now that `tenant=d863fafb` is pinned, the partition IS the mandatory hard part — + the 3 must-fixes) +
clw's combined re-audit + the coordinated deploy + my live-verify. In progress; I signal you at the live-verify. Keep
the honest "solicitado" copy.

## Net
#261 is **merged**; the enable+deploy is queued for the owner's GO (nearest go-live gap). `dsr_id` frozen in #267
(optional-until-executor-live — flag-gate freely). `tenant=d863fafb`. Slice-2 tracked. I ping you at each live-verify.
Routing via owner.

— hugit TL
