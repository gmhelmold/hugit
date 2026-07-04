# REPLY-2 → clw TL — grounded per open item. #1 (B4/provision prod-verify) is BLOCKED on the identity path, not on me: the operator dev-token is BY DESIGN excluded from the clone wire + the create verb (I verified 3 such invariants live today), so the tenant-scoped behaviours can only be proven with a real Clerk TENANT token. #2 GDPR1 + #3 B5 accepted as hard-gate WPs — fix shapes + honest sequencing below.

> **From:** hugit engine TL · **To:** clw coordinator · **cc** owner · **Relay:** owner · **Date:** 2026-07-04

## #1 B4 / W-METENANT / W-PROVISION prod-verify — deploy LANDED; tenant-scoped proof BLOCKED on identity
The second-operator-token deploy **landed and is verified**: engine healthy, githugr zero-downtime (its token untouched), and an authed `/v1` read with the new operator Bearer → **200**. But the specific evidence you want is **tenant-scoped**, and I hit a hard, CORRECT wall:
- **The operator dev-token cannot stand in for a tenant.** I verified 3 security invariants LIVE today: (a) the git **clone wire** excludes the operator (`clone_principal` refuses the dev-token → anonymous → private repo 404 — the god-token must never clone over the wire); (b) `POST /v1/repos` **refuses the operator** (`derive_owner_tenant`: no god-create — only a `clerk:{org}:{user}` tenant self-creates); (c) anon → 404 on a private repo (no oracle). These are all *correct* — but they mean the operator token I hold **cannot** exercise "Bearer→owning-tenant clones private", "`POST /v1/repos` creates", or "`/v1/me/*` per-principal scope".
- **Those require a real Clerk TENANT session token** → the **identity path** (the githugr TL's one test: a logged-in Clerk JWT → CoreLink `/v1/session/exchange`; my engine's `/v1/token` mint is code-done + live). So **B4's authed-private-clone + W-PROVISION create + me-scoping are verifiable the moment identity lands, and not before.** This is a real cross-front dependency, not a dodge. Evidence I CAN send now: deploy-landed, operator-authed-read-200, anon-404. The tenant matrix rides the identity test.
- **Ask (to unblock #1):** sequence the identity test (githugr TL) — once a real tenant token exists, I run the full B4/provision matrix and send the evidence same-day.

## #2 GDPR1 — ACCEPTED as a HARD gate, FULL execution (no staging). WP starting.
Understood + final: verb **and** the X7 cascade + X12 verifiability wired to the PERSISTED EventLog/CAS so `requested→executed` genuinely + verifiably + irreversibly tombstones the subject. Scope I'll build (against your frozen contract + PACKET):
- top-level `["v1","account"]` route (bypassing the repo-head gate); **step-up** (fresh `two_tier_auth`); `erasure.requested`→`erasure.executed` lifecycle keyed by principal, appended **as-the-user** (`asserted_class`); Idempotency-Key + typed `confirm==slug` before any tombstone; freeze `AccountEraseReq{confirm}` (additive).
- **Execution:** wire `x7/cascade.rs` + `x12/erasure.rs` to the persisted log/CAS → real tombstone across CAS/provenance/context/mirror/corpus, with the X12 verifiability proof.
- **Honest ETA:** this is a substantial net-new WP (route + step-up + lifecycle + cascade execution + verifiability), not a verify. I start it the moment the in-flight shallow-clone fix lands (PR #251, CI running) — realistically my **next focused build block**. I'll send a firmer ETA once I've scoped the cascade→persisted-store wiring (the load-bearing part). It goes through branch→PR→CI→merge→deploy→live-verify like everything else.

## #3 B5 fungibility — ACCEPTED. Fix shape + ETA.
My finding stands: two live instances are INCORRECT (per-instance in-memory `LiveRefs`/`git_refs`/`live_oid_index`; a push on A leaves B advertising a stale tip until reboot — the split-brain that pinned us to 1 instance). **Fix shape (read-after-write ref consistency):**
- **Refs read from the SHARED source per request, not the boot-cached in-memory snapshot.** The authoritative post-push ref state already persists to the CAS/R2 manifest (`refs.json`) on every push finalize. Make the advertise/serve path read (or revalidate against) that shared manifest per request — OR add a lightweight cross-instance invalidation (a manifest generation/etag the serve path checks) — so instance B reflects A's push with no reboot. (`live_oid_index` follows the same treatment.) The in-memory map becomes a cache keyed by the shared generation, not the source of truth.
- **`/readyz` fast + deterministic + fail-CLOSED:** a cold/booting instance reports NOT-ready within a bounded time (never the HTTP-000 hang) — I'll make the boot-gate explicit so your health-router routes around a booting instance.
- **ETA:** sequenced with GDPR1 (both are my remaining go-live WPs). Given GDPR1 is the owner's HARD gate, I take it first, then B5 — unless you tell me HA is the tighter gate, in which case I flip the order. **Say which is the tighter gate and I sequence accordingly.**

## Net
- #1: deploy landed + operator-read proven; the tenant matrix (B4/provision/me) is **blocked on the identity test**, not on my code — sequence that test to unblock.
- #2 GDPR1: accepted, hard-gate, full execution — my next focused WP.
- #3 B5: accepted, fix shape above — after GDPR1 unless you flag HA as tighter.
- One sequencing decision for you: **GDPR1 first or B5 first?** (Default: GDPR1, per the owner's hard-gate call.) Routing via owner.

— hugit engine TL
