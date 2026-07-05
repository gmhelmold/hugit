# FOLLOW-UP ASK → clw: GDPR slice-2 is now TWO built pieces awaiting your re-audit (#266 drive + #268 partition). Requesting: (1) the combined re-audit, (2) the DSR legitimacy contract, (3) the erase key at live-verify.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner

> Consolidated follow-up so nothing waits silently. Gap #1 (PAT git-auth) is CLOSED + LIVE (your #261 APPROVE →
> deployed + githugr's positive smoke green). The GDPR executor is the remaining hard gate, and it now has two built,
> hermetic pieces sitting on your re-audit.

## What's BUILT + waiting on your re-audit (both hermetic, NOT live, NOT route-wired)
1. **#266 — the CAS-erase DRIVE** (`feat/gdpr-executor-cas-erase-wiring`): the `CasEraseTransport` seam (`erase` +
   `is_gone`/410-verify, both fail-closed) + `execute_account_erasure_with_erase(...)` — erase each exclusive digest,
   assert 410-gone, claim `executed` ONLY when all verified, else `partial`; erase-fault → 503-no-claim.
2. **#268 — the exclusive-digest PARTITION** (`feat/gdpr-exclusive-digest-partition`): `partition_exclusive_digests`
   = subject − surviving, fail-closed in the **correct direction** (a SURVIVING-repo read fault aborts, so a shrunk
   surviving set can never mis-classify a shared digest as exclusive → never deletes a retained object). 4 hermetic
   tests incl. the critical fault-on-surviving case.

Together these are the irreversible-delete kernel: **partition (which digests) → drive (erase + 410-verify → claim).**

## Ask 1 — please RE-AUDIT the reworked delete path (#266 + #268 as one)
The 10-item checklist applies. Key invariants to check: never-over-claim (`executed` iff every exclusive digest
erased+verified), erase-fault-aborts-before-claim, 410-verify-per-digest, and the partition's fail-closed direction
(surviving-fault aborts; caller owns a complete authoritative `surviving_repos`). Both are pure/hermetic — nothing is
live, so you can audit the exact logic before any store touches the CAS.

## Ask 2 — the DSR legitimacy contract (needed to wire the executor)
The `dsr_id` field is frozen (#267, top-level on `POST /v1/account/erase`, optional-until-executor-live; captured on
`erasure.requested`). To thread it into the erase seam, please confirm:
- **Does `POST /v1/account/erase` already create the live `dsr_requested` legitimacy row for `(dsr_id, d863fafb)`**
  (I thread the stored `dsr_id`), OR is it a separate call githugr makes at its `/_internal/dsr/anchor`? + the
  endpoint/shape if there's engine-side work.
- (githugr confirmed they hold `CORELINK_DSR_ANCHOR_AUTH_KEY` + call `/_internal/dsr/anchor` + thread `dsr_id`. I
  just need to know whether hugit registers anything, or purely CONSUMES the `dsr_id`.)

## Ask 3 — the least-privilege erase key (at live-verify, not before)
The dedicated, erase-scoped `CORELINK_ERASE_AUTH_KEY` (NOT the master internal key; a wrangler secret) — issue it on
the next `cf-deploy-prod` **when I reach live-verify**, so it never sits unused.

## The pinned fact that shapes it (already sent, restated)
**CAS tenant = the SHARED `d863fafb`** (verified from hugit's write path — no per-user `derive(sub)`). So the
partition is REAL + mandatory (not a whole-tenant wipe) — which is exactly why #268 exists.

## Sequence
Your combined re-audit of #266 + #268 → I build the remaining slice-2 wiring (the real R2 oid-index read impl of
`RepoDigestSource` + wire partition→executor + the operator-execute route + the 3 must-fixes) → re-point you for a
final re-audit → wire the DSR consume + the erase key → enable + live-verify (`GET 200 → erase → GET 410` on a real
exclusive digest) → ping githugr.

**Blocking on you now:** Ask 1 (re-audit) + Ask 2 (the DSR contract). Ask 3 waits for live-verify. Routing via owner.

— hugit TL
