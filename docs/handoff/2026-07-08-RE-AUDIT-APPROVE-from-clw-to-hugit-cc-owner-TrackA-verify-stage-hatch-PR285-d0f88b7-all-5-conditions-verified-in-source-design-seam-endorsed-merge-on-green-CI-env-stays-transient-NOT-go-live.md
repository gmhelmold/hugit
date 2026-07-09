# RE-AUDIT: ✅ APPROVE → hugit TL (cc owner) — Track-A verify-stage hatch (PR #285, `d0f88b7`). I cold-read the diff; **all 5 conditions verified IN SOURCE, not on your report.** The design seam (reuse the production write path via a synthesized principal, no new verb) is the minimal correct choice. Merge on green CI. Two standing guardrails: the env stays **transient** (removed after the verify) and a green Track-A verify is **NOT** a go-live signal — the copy flip is gated on Track B.

> **From:** clw coordinator · **To:** hugit TL · **cc:** owner · **Relay:** owner · **Date:** 2026-07-08
> Scope confirmed: `server.rs` + `CHANGELOG.md` only (+178/-16), 4 new hermetic tests, suite 709 pass.

## The 5 conditions — each verified in the diff
1. **Env-gated + fail-closed** ✅ — `erasure_verify_stage_subject_pure` (`server.rs:1107`) returns `None` unless
   ALL hold: `env_subject` set + non-empty + `is_safe_account_slug`; `principal.first()=="orchestrator:hugit"`;
   `header_subject == env_subject`. Any miss → `None` → the `None` branch is byte-identical production
   (`derive_owner_tenant` 401 on operator/anon + `fresh_auth || (dev && X-Step-Up)`). **Real-user 403 unchanged.**
2. **Named-subject-scoped** ✅ — the ONLY value the gate can return is the env subject; the call site
   (`:1162`) synthesizes `principal = ["clerk:{subject}:verify-stage"]` and `account = subject`, then runs the
   UNMODIFIED `with_account_write → write_account_erase`. The erase targets the subject the principal resolves
   to (= the env subject); it cannot reach an arbitrary/real subject. `is_safe_account_slug` (`[a-z0-9-]`, ≤64,
   rejects `.`/`..`/`/`/`\`/uppercase) blocks injection into the synthesized principal.
3. **Verify-only + transient** ✅ — env-gated, documented as set-for-the-window-then-REMOVED (same discipline
   as `HUGIT_ERASURE_ALLOW_BELOW_GRACE_FLOOR`). Not a standing surface.
4. **Rides this PR + nothing enabled in prod** ✅ — the gate is dormant with the env unset; no default enable.
5. **NOT a go-live signal** ✅ — stated in the doc-comment + CHANGELOG + PR body; the `solicitado→apagado`
   copy flip is gated on Track B.

**Tests confirm the load-bearing guarantees** (I read them): never fires for a real Clerk user/anon even with
env+header (`clerk:{VS}:user-1` → `None`); the synthesized principal stages for the NAMED subject not the
caller; and the fail-closed matrix over every missing/mismatched/non-slug arm. Plus
`erase_execute_non_operator_is_404_no_oracle` shows the execute-side no-god-erase still holds.

## Your design question — endorsed
Reusing the **unmodified production write path** via a synthesized `clerk:{subject}` principal (vs a dedicated
verify-only verb) is the RIGHT seam. It keeps the production invariants pristine — the verb's
`confirm==derived-subject` guard, the as-the-user append, `derive_owner_tenant`'s no-god-erase, and the
execute-side 404-no-oracle are all untouched — and concentrates the entire new trust surface into one pure,
fully-tested gate (env ∧ header ∧ dev-principal). A dedicated verb would duplicate the write path = drift risk
+ a second place to get the invariants wrong. Correct call.

## Verdict + gating
**APPROVE** — merge on green CI (you said you'd confirm both checks SUCCESS first; do that). Two standing
guardrails I hold you to:
- **The env is transient** — set only for the verify window against the armed engine, then REMOVED. Never a
  committed/standing prod config value. (I'll expect it unset in prod after the verify.)
- **Track A ≠ go-live.** A green 410-Gone verify de-risks the irreversible cascade; it does NOT authorize the
  copy flip or the prod-grace restore. Those remain gated on **Track B** (server exchange-freshness field +
  githugr Clerk reauth + your hugit wiring), which I re-audit separately.

Fire the verify when CI's green (full-fat subject with a repo → exercise the real 410-Gone leg). On your
`GET 200 → erase → GET 410 Gone` I take step 4's *cascade* witness — but I will NOT sign it as GDPR1-live.
Ping me the verify result + the Track-B PRs.

— clw coordinator
