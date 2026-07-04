# GDPR1 account-erase — implementation design (the decided plan)

> **Status:** DECIDED design. Contract frozen (PR #252). Verb + route + execution are implemented against THIS.
> **Owner mandate:** hard go-live gate, no waiver, full execution (`requested → executed` genuinely tombstones).
> **Rigor note:** this is the product's ONLY irreversible-deletion path. It ships only after an adversarial audit
> (checklist below). This doc exists so the build is a low-ambiguity follow-through, not a marathon-hour hack.

## The decisions

### D1 — the account-scoped persistence model (the blocker I flagged)
The engine's event logs are **per-repo** (`<r2_tenant>/<repo>.json`); an `erasure.requested` is **account-scoped**
(keyed by the caller's account = its `clerk:{org}` slug). There is no per-account log today.

> **⚠️ CORRECTION (2026-07-04, caught during the handler build — before it became a bug):** the FIRST cut of this
> decision — reuse the repo `LogSink` at key `_erasure/{account_slug}` (relying on the `/` to make `is_safe_repo_slug`
> reject it → never served) — is **internally inconsistent**: `AppState::persist` ALSO gates on `is_safe_repo_slug`
> (`state.rs`), so the very property that makes the key non-SERVABLE makes it non-PERSISTABLE through the LogSink.
> Reusing the repo store is therefore impossible. Also: `idem_lookup`/`idem_record` are private to `writes/mod.rs`
> (not reusable), and `with_write`'s `authorize_write` gate expects repo ownership. So the account log needs a
> **dedicated persistence seam**, below.

**Decision (revised):** persist the account erasure lifecycle to a **dedicated per-account event log** via a NEW,
account-scoped store seam — NOT the repo `LogSink`:
- **Key:** `<r2_tenant>/_accounts/{account_slug}.json` (a reserved R2 sub-prefix). Structurally OUTSIDE the repo
  namespace: the repo read/clone/`/v1/repos` paths resolve `<r2_tenant>/{repo}.json` for a single-segment
  `is_safe_repo_slug` — they NEVER consult the `_accounts/` sub-prefix, so the erasure log can never be served as a
  repo. (`account_slug` = the validated clerk-org shape `[a-z0-9-]`.)
- **Seam:** add `load_account_log(account) / persist_account_log(account, log, expected)` to the store (mirroring
  the repo `load_verified_with_token` + create-only/CAS `persist`, but keyed under `_accounts/` and NOT gated on
  `is_safe_repo_slug` — the account slug has its own validation). Load-or-create-append: create-genesis
  (`CasToken::Absent`) on the first request, CAS-append on subsequent (its own retry loop; the private idem helpers
  are re-implemented account-scoped OR promoted to `pub(crate)`).
- **One log per account**, self-contained → the Part-2 cascade reads exactly one log to drive one account's tombstone.

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
