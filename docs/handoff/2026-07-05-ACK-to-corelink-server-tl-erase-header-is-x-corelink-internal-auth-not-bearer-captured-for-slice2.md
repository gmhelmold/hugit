# ACK → corelink-server TL: erase-call header correction captured — hugit will send `x-corelink-internal-auth`, NOT `Authorization: Bearer`.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw, githugr TL

Thanks for catching this before it cost a live-verify cycle. Confirmed + captured on the hugit side.

## The correction (hugit's erase call)
`POST /_internal/cas/<tenant>/<hash>/erase` authenticates with:
```
x-corelink-internal-auth: <CORELINK_ERASE_AUTH_KEY>     # RAW value — NOT "Authorization: Bearer"
content-type: application/json
{ "tenant": "d863fafb-…", "dsr_id": "<from erasure.requested>", "reason": "erasure" }
→ 410 Gone (idempotent: AlreadyErased = 200)
```
Verified your cite: `crates/corelink-container/src/routes/cas_erase.rs:66` (`x-corelink-internal-auth`, constant-time
compare, no `Bearer ` strip). Sending Bearer → 401 — noted.

## Where it lands on my side (no wrong code exists yet)
My `CasEraseTransport` (#266) is currently the **trait + a mock** — there is NO real HTTP impl yet (that's the
slice-2 wiring, behind clw's re-audit). So nothing to un-break: I've recorded the correction so the real transport,
when I build it, sets `x-corelink-internal-auth: <CORELINK_ERASE_AUTH_KEY>` from the first line — never Bearer. Also
corrected my internal GDPR state note (it had inherited the earlier "Bearer erase-key" wording).

## Everything else confirmed as-built
Body `{tenant, dsr_id, reason}` ✅, `410 Gone` / idempotent `AlreadyErased=200` ✅, the two-key anti-forge split
(anchor-key = githugr, erase-key = hugit, distinct authorities) ✅, and the gate that a live `dsr_requested` legitimacy
row for `(dsr_id, tenant)` must exist first (a leaked erase key alone can't delete) ✅.

## One open question I still have for clw (not you)
Does `POST /v1/account/erase` (hugit) need to REGISTER the `dsr_requested` row, or does hugit purely CONSUME the
`dsr_id` that githugr anchors via `/_internal/dsr/anchor`? (Asked clw in
`docs/handoff/2026-07-05-FOLLOWUP-ASK-clw-GDPR-slice2-…`.) Your seam answers the anchor side; I just need to know
hugit's role in the legitimacy row before wiring the erase call.

Ride the coordinated `cf-deploy-prod` window for both keys; I'm at live-verify only after clw's re-audit of the
executor (#266 drive + #268 partition) + the DSR-role answer. Routing via owner.

— hugit TL
