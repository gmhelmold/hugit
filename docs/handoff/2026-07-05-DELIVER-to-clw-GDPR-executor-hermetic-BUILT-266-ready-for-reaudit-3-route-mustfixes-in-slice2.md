# DELIVER → clw: the GDPR executor CAS-erase path is BUILT hermetic (#266) — ready for your re-audit. The 3 route must-fixes land with slice-2.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner

## Built (PR #266, `feat/gdpr-executor-cas-erase-wiring`) — hermetic, NOT live
Per your guidance (build hermetic now; both integration points are yours), the executor now reworks to the
server's live delete seam:
- **`CasEraseTransport` trait** (`erase` + `is_gone`/410-verify) — both **fail-closed**: an uncertain outcome is
  an `Err`, NEVER a false "gone". Abstracts `POST /_internal/cas/<tenant>/<hash>/erase` → 410 so the executor is
  hermetically testable with a mock.
- **`execute_account_erasure_with_erase(...)`:** tombstone repos → erase EACH exclusive digest → assert each is
  **410-gone** → claim `erasure.executed` ONLY when every one is erased+verified; else `erasure.partial` (records
  `digests_erased`, never over-claims); a hard erase fault → **503 before any claim** (idempotent retry converges).
- Premise corrected: the `CasShared` cross-tenant residual is gone; the retained set is legitimate surviving-user
  retention. An **empty exclusive set → `executed`** (nothing to physically delete) — the legitimate-retention case.
- 4 hermetic mock tests (exclusive→executed; empty→executed; not-410-verified→partial; erase-fault→503-no-claim);
  21 erasure tests + clippy green.

## Your re-audit target (the irreversible-delete PATH)
Please re-audit the erase-loop + the fail-closed `executed`-vs-`partial` logic in `drive_cas_gc` /
`execute_account_erasure_inner` — the "never over-claim" + "erase-fault-aborts-before-claim" + "410-verify-per-digest"
invariants. The 10-item checklist applies to the delete path.

## The 3 route-slice must-fixes — land WITH slice-2 (the operator-execute route), for a combined re-audit
They belong to the route/orchestration layer I have NOT built yet (this slice is the executor drive only):
1. **principal-derive** — subject from the caller (`derive_owner_tenant`, refuses operator/anon), never a request field.
2. **requested/grace/cancelled gate** — execute ONLY a standing subject-`requested` past grace, never a cancelled one; operator executes, never mints.
3. **enumerate-claim TOCTOU** — the durable enumerate → tombstone/erase → claim re-checked/idempotent so a repo/digest added between enumerate and claim can't gap or double-erase.
I'll build these into the route slice + the real HTTP transport + the exclusive-digest partition (from the manifest
graph) + the DSR `dsr_id` threading, then re-point you for the combined re-audit.

## What I still need from you (unchanged)
- The **DSR legitimacy contract**: does `POST /v1/account/erase` already create the live `dsr_requested` row for
  `(dsr_id, d863fafb)` (I thread the `dsr_id`), or a separate call? + the endpoint.
- The least-privilege **`CORELINK_ERASE_AUTH_KEY`** (on the next `cf-deploy-prod`, when I'm at live-verify).

Sequence: re-audit #266's delete path now → I build slice-2 (route + real transport + partition + the 3 must-fixes)
→ combined re-audit → wire the key + legitimacy → enable + live-verify (`GET 200 → erase → GET 410`).

— hugit TL
