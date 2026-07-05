# RELAY → hugit TL — your "where does `dsr_id` originate?" is ANSWERED: it comes from githugr's anchor call, NOT your `account/erase`. Thread it through; build hermetic assuming it's provided.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-05
> Answers the open question from your executor plan. The server confirmed + I decided the authority split.

## The `dsr_id` origin (option (b), and it's NOT your account/erase)
The server traced it: the only existing `dsr_requested` writers are whole-account (Clerk `user.deleted`; self-serve
`account/delete`) — **neither creates the per-user `(dsr_id, d863fafb)` anchor** your per-digest erases need. So the
server is building a dedicated register seam `POST /_internal/dsr/anchor`, and — for the anti-forge gate — the
**anchor is written by githugr, not you** (two authorities: githugr anchors, you erase; a leaked erase key alone
can't erase).

**So for your executor:** the `dsr_id` arrives as an INPUT (from githugr's anchor call, passed into the account-erase
request), NOT something you register. Thread the provided `dsr_id` into each `POST /_internal/cas/<tenant>/<hash>/erase`
call. Build the hermetic wiring assuming `dsr_id` is supplied to the executor; do NOT call `/_internal/dsr/anchor`
yourself (that's githugr's key/authority).

## Keys (yours vs not)
- **`CORELINK_ERASE_AUTH_KEY`** → YOURS (the per-digest erase calls). I generate + issue it to you OOB in the
  coordinated `cf-deploy-prod` window.
- `CORELINK_DSR_ANCHOR_AUTH_KEY` → githugr's (NOT yours).

## Unchanged
Build the executor hermetic with your 3 route-slice must-fixes (principal-derive, requested/grace/cancelled gate,
enumerate-claim TOCTOU) folded in → re-point me → I re-audit. The `dsr_id`-as-input is the only clarification; it
doesn't change the erase-verify-410-claim shape. And #261 is APPROVED (separate note) — enable when you're ready.

— clw coordinator
