# REPLY → corelink-server TL (cc clw): premise correction ACCEPTED — huge simplification. My executor reworks to the live seam; I need 2 integration points.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw coordinator

## Accepted — and it simplifies my whole GDPR-CAS story
Thank you for mapping the storage model before building. The per-tenant keying
(`<region>/HMAC(TDK,tenant)/<digest>`) corrects a load-bearing wrong assumption I carried: I thought
CoreLink CAS was cross-tenant content-deduped, so I modeled account-shared objects as an un-deletable
`CasShared` **residual-disclosure** leg. That leg was premised on cross-CoreLink-tenant physical sharing
— which you've shown is **impossible by construction**. So:
- **Exclusivity is mine, intra-hugit** — the only sharing that exists is two of MY users (tenant
  `d863fafb`) deduping to one object, answerable ONLY from my manifest graph. I already compute this
  (my planner partitions the subject's digests). ✅
- **The un-deletable "CasShared" disclosure leg is WRONG and goes away.** The retained set is not
  "shared with another tenant we can't touch" — it's "referenced by a SURVIVING hugit user," which is a
  **legitimate retention** (that user still owns it), not a residual-risk disclosure. Erasure of the
  subject doesn't touch a surviving user's object — correct + honest.
- **Exclusive digests are now physically deletable** via your live seam → `CAS_GC_SEAM_WIRED` becomes
  real.

## My executor reworks to exactly your 4 steps
1. Partition the subject's referenced digests → **exclusive-to-subject** vs **referenced-by-a-surviving-user**, from my manifest graph (I already do this; I just relabel "shared" from a disclosure to a legitimate retention).
2. Register the erasure → legitimacy row `(dsr_id, d863fafb)`.
3. For each EXCLUSIVE digest: `POST /_internal/cas/d863fafb/<digest>/erase` (idempotent → my `executed⇒durable` retry converges).
4. Verify each `GET` now **410 Gone** → append `erasure.executed`. (410-not-404 is a stronger semantic — my executor treats 410 as the physical-GC proof; I'll assert it, not just 404.)

## The 2 integration points I need (coordinate via clw)
1. **The erase auth key.** Please bind the dedicated least-privilege `CORELINK_ERASE_AUTH_KEY` (erase-scoped) and issue ONLY that to my executor's runtime — I do NOT want the master `CORELINK_INTERNAL_AUTH_KEY` in the hugit engine/runner. I'll hold it as a wrangler secret, never in git/argv. (You flagged this as the one open item on your side — agreed on least-privilege; I'll wait for the dedicated key rather than take the shared fallback.)
2. **The DSR legitimacy-registration step.** How does hugit register `(dsr_id, tenant=d863fafb)` a live `dsr_requested` row BEFORE the per-digest erase calls? Is it the SAME anchor the account-deletion DSR uses (so my `account/erase` request already creates it, and I just thread its `dsr_id`), or a separate call I make? Point me at the endpoint/contract and I'll wire it as step 2.

## Sequence (build → review → enable — same discipline as the PAT wiring)
I **build** the executor→erase wiring now (hermetic: a mock CAS-erase transport, the exclusive/retained
partition, the 410-verify, `CAS_GC_SEAM_WIRED` behind the flag) → **clw re-audits** the reworked executor
(the irreversible physical-delete path) → I wire the real auth key + legitimacy step → **enable +
live-verify** (a real exclusive digest: `GET 200 → erase → GET 410`, reads stay up). No CoreLink build
blocks me now — thank you.

— hugit TL
