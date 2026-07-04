# GDPR1 account-erase — implementation design (the decided plan)

> **Status:** DECIDED design. Contract frozen (PR #252). Verb + route + execution are implemented against THIS.
> **Owner mandate:** hard go-live gate, no waiver, full execution (`requested → executed` genuinely tombstones).
> **Rigor note:** this is the product's ONLY irreversible-deletion path. It ships only after an adversarial audit
> (checklist below). This doc exists so the build is a low-ambiguity follow-through, not a marathon-hour hack.

## The decisions

### D1 — the account-scoped persistence model (the blocker I flagged)
The engine's event logs are **per-repo** (`<r2_tenant>/<repo>.json`); an `erasure.requested` is **account-scoped**
(keyed by the caller's account = its `clerk:{org}` slug). There is no per-account log today.

**Decision:** persist the account erasure lifecycle to a **reserved per-account log** at the WriteStore key
**`_erasure/{account_slug}`** → R2 object `<r2_tenant>/_erasure/{account_slug}.json`.
- **Non-colliding by construction:** the key contains `/`, and `is_safe_repo_slug` REJECTS any slug containing `/`
  — so this key can NEVER be a real repo, is never served over `/v1/repos/{repo}` or the git wire, and cannot be
  clobbered by / clobber a repo. (`account_slug` itself is validated `[a-z0-9-]`, the clerk org shape.)
- **One log per account**, not per-engine — so an account's erasure history is self-contained + the Part-2 cascade
  reads exactly one log to drive one account's tombstone.

### D2 — the request verb (Part 1b, STAGING — no deletion)
`write_account_erase(sink, req, principal_chain, at)`:
- **Subject = the caller's own account**, derived from the verified principal via `write_provision::derive_owner_tenant`
  (refuses operator/anon/malformed → no god-erase / no anon-erase, mirrors provision's `derive_owner_tenant`).
- **Typed confirm guard:** `req.confirm` MUST equal `account_slug` — validated BEFORE any write (fail-closed against
  an accidental irreversible erase). Mismatch → `400`.
- **As-the-user append:** `erasure.requested` with `class = crate::writes::asserted_class(principal_chain)` (a tenant →
  Orchestrator integration authority), subject/account in the payload — NOT the `PrincipalClass::Orchestrator`
  hardcode the `decide` verb uses. Payload `{account, subject, state:"requested", requested_at}`.
- **Load-or-create-append** on the `_erasure/{account_slug}` log: create-genesis if absent (model provision's
  `CasToken::Absent` create-only compare-and-swap), else load-verify-append (model `with_write`). Idempotency-Key
  required (a replayed key = the same recorded request, no duplicate).

### D3 — the route + door (top-level, step-up)
`["v1","account","erase"]` in `route_write` (`server.rs`), beside `["v1","repos"]` (provision), bypassing
`dispatch_repo_write`'s repo-head gate. Auth = `two_tier_auth`; **step-up REQUIRED** — re-apply the gate explicitly
(mirror `server.rs:1036-1047` / the `STEP_UP_VERBS` check in `with_write`), since the top-level path skips the repo door.

### D4 — the EXECUTION cascade (Part 2, the no-waiver part) — IRREVERSIBLE
Wire `x7/cascade.rs::erase_datapoint` (`:448`) + `x12/erasure.rs` (`:216`) — today they operate on their OWN
in-memory stores — to the **persisted EventLog/CAS**, so `erasure.requested → erasure.executed`:
- tombstones the subject's objects across CAS / provenance / context / mirror / corpus;
- emits the `MirrorObligation` residual-risk disclosure;
- appends `erasure.executed` to the `_erasure/{account_slug}` log ONLY after the durable tombstone (fail-closed:
  `executed` ⇒ the store is actually tombstoned, mirror the receive-pack `ok`⇒durable discipline).
- **Idempotent + irreversibility-guarded:** a replay of an already-`executed` account is a NO-OP (never a
  double-tombstone, never resurrects). A `requested` with no matching approval/authority never executes.

## The adversarial-audit checklist (MUST pass before it can tombstone real data)
1. No god-erase / no anon-erase (operator + anon refused at `derive_owner_tenant`).
2. Cross-account isolation: a caller can erase ONLY its own account (subject == derived principal, never a param).
3. Typed-confirm gate fires BEFORE any write; a mismatch leaves zero state.
4. Step-up enforced on the top-level route (not bypassed by skipping the repo door).
5. `executed` is emitted ONLY after the durable tombstone (no `executed`-without-deletion).
6. Idempotent replay (requested + executed) = no-op; irreversibility holds (no un-tombstone).
7. The `_erasure/{slug}` key can never be reached as a repo (the `/`-contains-slug invariant) — a read/clone/git
   probe of it → 404.
8. Secret-scrub at the write boundary (the account slug / confirm are structural, but the guard runs anyway).

## Build order
1. ✅ `AccountEraseReq` frozen (PR #252).
2. `write_account_erase` verb + the `_erasure/{slug}` load-or-create-append + hermetic tests (D2).
3. Route + step-up (D3) + a hermetic route test.
4. The X7/X12 → persisted-store execution cascade (D4) + the full adversarial audit (checklist) — the WP that
   actually tombstones; gated behind the audit before any live enablement.
5. Live-verify with a real Clerk tenant token (the identity path) — the operator token cannot exercise this
   (no god-erase), same dependency as B4.
