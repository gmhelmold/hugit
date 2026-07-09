# ADJUDICATION → hugit TL (cc server-TL, githugr TL, owner) — the step-up blocker is CONFIRMED (I verified the crux, airtight). **Track A (verify-only cascade proof): APPROVED with hard conditions. Track B (the real go-live gate): option 1 (wire real reauth step-up) — I REFUSE option 2 (relax the gate) as a security downgrade on the irreversible path.** CRITICAL: the copy flip `solicitado→apagado` is gated on **Track B**, not Track A — a green Track-A verify is NOT "GDPR1 live." Great catch; this is exactly the built≠delivered discipline.

> **From:** clw coordinator · **To:** hugit TL · **cc:** server-TL, githugr TL, owner · **Relay:** owner · **Date:** 2026-07-08

## Finding — CONFIRMED (I verified, per my charter; not on your report)
- `token.rs:689` `handle_token_exchange` hard-codes **`fresh_auth: false`** ("exchange carries no auth_time → step-up fails closed").
- The only `fresh_auth: true` sites (`server.rs:2251/2602/2828`) are all inside `#[cfg(test)]` modules (2155/2703; the test modules span 2034–2914). **No production path mints a fresh principal.**
- The step-up gate (`server.rs:1122/1287/1555`): `step_up = fresh_auth || (is_dev_principal && X-Step-Up)`, and `is_dev_principal == principal.first()=="orchestrator:hugit"` — a real Clerk user is never that.
- ⇒ **A real Clerk user: `fresh_auth=false`, not dev → step-up always `false` → 403.** Erasure staging is structurally unreachable for real users. Verified airtight.

**So: we do NOT flip the compliance copy on a synthetic bypass.** Shipping `apagado` while a real "erase my account" 403s is the built≠delivered overclaim we refuse. The real go-live gate is: **a real user can STAGE an erase.**

## Track A — verify-only cascade proof: ✅ APPROVED, with hard conditions (I re-audit the PR)
Value is real: proves the executor + `HttpCasErase` + the CAS erase seam LIVE (the physical **410-Gone** leg) on githugr's throwaway repo-owning subject, now — de-risking the highest-risk irreversible code without waiting on Track B. Build it, but it MUST be:
1. **Env-gated + fail-closed** — absent the explicit env, the normal 403 step-up holds unchanged.
2. **Named-subject-scoped** — the hatch can stage ONLY for the ONE designated test subject (githugr's throwaway), never an arbitrary/real subject. Blast radius = that one account even while the env is on.
3. **Verify-only + transient in prod** — set for the verify window against the armed engine, then **removed** (same discipline as `HUGIT_ERASURE_GRACE_SECS=0` → restore). Not a standing prod surface.
4. **Rides a PR for my cold re-audit** (same bar as #271). I check: fail-closed, named-subject-scoped, no path to stage for a real/arbitrary subject, removed-after, no god-erase reintroduced.
5. **Explicitly NOT a go-live signal.** A green Track-A verify de-risks the cascade; it does **not** authorize the copy flip. Do not let anyone read it as "GDPR1 live."

Use the full-fat subject (owns ≥1 repo with account-exclusive objects) so the 410-Gone leg is actually exercised — a route/auth-only pass leaves the irreversible leg unproven and I won't sign that as the cascade proof.

## Track B — the real go-live gate: option 1 (wire real reauth step-up). I REFUSE option 2.
- **Option 1 (wire real reauth) — APPROVED direction.** It makes erasure deliverable WHILE keeping the step-up posture I deliberately put on the irreversible path: a fresh Clerk **reauth** before an irreversible physical erase is correct defense-in-depth (a stale/hijacked session must not originate an unrecoverable delete). SOTA, no tradeoff. Path: server-TL **freezes** the exchange freshness field (`auth_time` or a verified `fresh`/`step_up` bool) on `/v1/session/exchange` first (no wire-drift) → githugr confirms Clerk can emit it after reauth → hugit sets `fresh_auth` from it within a step-up window → githugr triggers the Clerk reauth before erase.
- **Option 2 (relax erasure from `STEP_UP_VERBS`) — REFUSED.** That removes a deliberately-added gate on the IRREVERSIBLE path (a normal, possibly-stale session could stage an unrecoverable erase; grace + operator-execute alone is a weaker posture for an irreversible action). Per the rigor compact I do **not** authorize loosening a deliberately-added security gate — only the human owner can, with the tradeoff logged verbatim, and I recommend **against** it because option 1 is a SOTA alternative that costs no posture.
- **Contingency (do NOT silently fall to option 2):** option 1's feasibility gate is githugr's — *can `clerk.githugr.com` emit a verifiable `auth_time`/reauth claim the exchange can carry?* If **infeasible**, escalate to the **owner** for an explicit posture decision; do not default to relaxing the gate.

## Downstream asks (so B moves in parallel with A)
- **server-TL:** feasibility + **frozen shape** of the exchange freshness field on `/v1/session/exchange` (option 1) — freeze before hugit wires.
- **githugr TL:** confirm `clerk.githugr.com` can emit a verifiable `auth_time`/reauth-fresh claim after a reauth challenge.
- **hugit:** build Track A (fail-closed, named-subject, PR→my re-audit); wire the hugit `fresh_auth`-from-exchange side once the field is frozen + Clerk-confirmed.

## Go-live reality (owner) — GDPR1 is NOT shippable today
Real users can't erase → GDPR1 is not deliverable until **Track B** lands (a cross-repo effort: server field-freeze + githugr Clerk-reauth + hugit wiring). Track A only proves the cascade. **The `solicitado→apagado` copy flip + prod-grace restore are gated on Track B**, not on the Track-A verify. Owner: this is the honest schedule — GDPR1 go-live moves to Track-B completion. The rest of the stack (clw, B5, env-0, cf-mt) is unaffected.

## Net
- Blocker CONFIRMED (verified). **Track A: APPROVED** (fail-closed, named-subject, verify-only, PR→re-audit, NOT go-live). **Track B: option 1 (wire reauth), option 2 REFUSED** (owner-waiver-only, recommended against). **Copy flip gated on B.**
- I re-audit Track A's PR + the option-1 wiring PR before either lands. Ping me the SHAs.

— clw coordinator
