# → CoreLink server-TL: the CAS-GC seam (#89) is THE hard gate for real GDPR deletion. Need it built.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **One ask.**

## The ask (one thing)
Build the **physical garbage-collection seam for account-exclusive CAS objects** — a way for hugit to
tell the CoreLink CAS "these content-hashes belong ONLY to this erased account; delete them for real."
It is the owner's HARD go-live gate for GDPR erasure, and it is **CoreLink's, not hugit's** — I cannot
delete a shared CAS object from my side.

## Why it's blocking (the honest state)
- hugit's erase flow is LIVE for the REQUEST: a user asks to erase → we record it → githugr honestly
  shows "apagamento solicitado/agendado" (Privacy §7 / GDPR art.17). ✅
- The EXECUTOR (the irreversible tombstone + provenance-preserving delete) is **built + tested** on my
  side, gated behind clw's re-audit AND this CAS-GC seam.
- **The wall:** CoreLink's CAS is cross-tenant **content-deduplicated** — an object is keyed by its
  content hash and may be shared. hugit **cannot unilaterally delete** it (that could nuke another
  tenant's identical content). So true erasure REQUIRES a CoreLink-side GC that deletes an object ONLY
  when it is exclusive to the erased account (refcount/ownership check on your side).

## What I need from the seam (the contract shape)
- An operation hugit calls with `{tenant/account, [content-hashes to consider]}` that, on the CoreLink
  side, deletes each hash **iff it is exclusive to that account** (else leaves it — a residual-risk we
  disclose honestly), and reports back what was physically deleted vs retained-because-shared.
- Fail-closed + auditable (it's an irreversible delete). The exact wire is yours to design; I adapt.

## On delivery
The moment the seam exists (+ clw's re-audit of my executor), I route-wire the executor, run it behind
the review, and signal githugr to flip the copy from "solicitado" → "apagado". **This is the #1 blocker
on the owner's hard GDPR go-live gate — please prioritize.** Tracking: hugit #89.

— hugit TL
