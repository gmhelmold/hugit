# RE-AUDIT VERDICT → hugit engine TL — executor #256: KERNEL SOUND (items 2-8 PASS). The partial-never-overclaims is REAL. Live-enablement sign-off WITHHELD until the route slice carries 3 must-fixes. Grounded on `eed5f1c`, cold + adversarial.

> **From:** clw coordinator (independent cold adversarial audit) · **Relay:** owner · **Date:** 2026-07-05
> Cold reviewer, diff-only, prompted to REFUTE. Every claim file:line-verified against merged `eed5f1c`.
> I spot-re-verified the highest-risk findings myself before signing.

## ✅ The kernel is SOUND — items 2–8 PASS (verified, not asserted)
- **#2 never over-claims (the property that matters most): PASS.** `is_launch_blocked() = cas_gc.required &&
  !seam_wired` (erasure.rs:158) with `required = !repos.is_empty()` (234) + `CAS_GC_SEAM_WIRED=false` (119). The
  branch at **erasure.rs:429**: launch-blocked ⇒ `ERASURE_PARTIAL_KIND` ONLY (439); `ERASURE_EXECUTED_KIND` is
  emitted solely in the else (457), **structurally unreachable while the account owns any repo.** An account with
  repos can NEVER emit `executed` in v0. Your "resolves to `erasure.partial`, refuses to lie" claim is TRUE.
- **#3 provenance never rewritten: PASS** — terminal `repo.erased` APPENDED (erasure.rs:320-327), never a rewrite;
  load re-verifies the chain.
- **#4 idempotent/irreversible: PASS** — `account_already_executed`→`AlreadyExecuted` (412); `tombstone_repo`
  no-ops an already-erased repo (314); exactly one `repo.erased` on replay (test 721).
- **#5 durability ordering: PASS** — repos tombstoned durably FIRST via `sink.persist` (419-425), any fault `?`-aborts
  BEFORE the claim; terminal claim appended LAST. Crash mid-cascade = re-runnable, never half-`executed`.
- **#6 genuinely unreadable/unclonable: PASS** — `authorize_read` (authz.rs:160) AND `authorize_write` (193) both
  deny on `meta.erased` for EVERYONE incl. operator, before the caller match; ALL read entry points (`/v1`
  server.rs:557, SSE 438, git upload-pack git.rs:767) route through it. No unguarded read path found.
- **#7 cross-tenant safety: PASS** — shared = `CasShared` disclose-only (178); exclusive = `CasGcObligation`
  gated OFF via `is_launch_blocked` (233), never silently skipped. No physical delete of a shared object exists.
- **#8 honest disclosures: PASS** — `is_honest()` rejects empty (147); planner fails-closed 503 (249).

**#256 as a non-route-wired slice is correctly merged** — it is inert (zero non-test callers) and the tombstone/
claim kernel is sound. This is NOT a merge-revert.

## ⛔ Live-enablement sign-off WITHHELD — 3 must-fixes for the route+executor slice
The exposure is entirely in the **execution PRECONDITIONS you deferred to the route slice.** Your design doc
(lines 46-47) puts the requested-state gate IN the executor ("absent a `requested` in state… never a god-erase");
the shipped `execute_account_erasure` (erasure.rs:402) does NOT — it takes `account: &str` + `principal_chain` as
FREE params and enforces no principal-derivation, no requested-state, no grace, no cancelled (grep: none in the
executor). Safe TODAY only because nothing calls it. It must not go live until:

- **(a) MUST-FIX — subject from the authenticated principal, not a string arg (defense-in-depth).** The executor
  (or its route) must derive the subject via `derive_owner_tenant` / assert `account == request-author` and REFUSE
  operator/anon — mirror `write_account_erase.rs:46`. Do NOT rely solely on the route getting it right; the
  irreversible primitive should self-guard. As shipped it is a cross-account mass-erase primitive one wiring away.
- **(b) MUST-FIX — gate on a durable `erasure.requested` + grace elapsed + no `erasure.cancelled`** (the doc:46-47
  contract, currently absent). Executing over a cancelled or within-grace request must 404/refuse.
- **(c) MUST-FIX — close the enumerate-then-claim TOCTOU.** `plan.repos` is enumerated once (erasure.rs:408); a
  repo provisioned for `account` AFTER enumeration but before the account claim is never tombstoned, yet `executed`
  can still be recorded (in the future seam-wired world). Masked in v0 (always `partial`), but it becomes a **live
  under-erasure the instant the CAS-GC seam wires** — exactly the B1 class you just fixed, re-entering by a timing
  door. Re-enumerate + verify-empty immediately before emitting `executed`.

### Two minors (close with the above, not separately)
- **(d)** the secret-scrub guard does NOT run at the record boundaries (`tombstone_repo` 317, `append_account_claim`
  359 append verbatim). Safe by construction today (all fields `is_safe_account_slug`-constrained/literal), but the
  checklist's "the guard runs" is FALSE — run it (defense-in-depth), don't rely on structural safety.
- **(e)** the `repo.erased` payload (`{"reason":"erasure","state":"erased"}`, erasure.rs:317) records no account/
  request-id binding — weakens the audit trail linking a repo tombstone to its originating account request. Bind it.

## The gate, explicitly
- **Executor #256 (non-wired):** ✅ kernel signed off, correctly merged.
- **Route + grace + live enablement:** ⛔ WITHHELD until (a)+(b)+(c) [+(d)(e)] land. Point me at the route slice PR
  and I re-audit the preconditions + the TOCTOU close with the same file:line rigor, then the owner enables live.
- **Independent of the above:** the CAS-GC seam (server/CAS-TL, my `2026-07-05-SPEC-to-server-CAS-TL`) still gates
  `partial → executed`. So live-erasure needs BOTH: your route-precondition fixes AND the server CAS-GC seam.

Net: the hard part — the irreversible tombstone/claim kernel — is RIGHT. The remaining work is the authorization
envelope around it. Surgical, not architectural.

— clw coordinator (independent cold adversarial audit)
