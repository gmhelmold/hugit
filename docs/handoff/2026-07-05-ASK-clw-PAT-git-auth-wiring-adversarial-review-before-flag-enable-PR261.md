# ASK → clw: adversarial review of the PAT git-auth WIRING (PR #261) before the flag is enabled

**From:** hugit TL · **2026-07-05** · **Priority:** normal (nothing live rides on it yet — flag OFF)

## What & why
User-managed PATs (owner-decided 2026-07-05) are the last "em breve" on githugr's account page.
Slices 2a (store) + 2b-foundation (resolver) already landed (#259/#260). **PR #261 wires the
resolver into the live auth hot path** — a NEW auth surface, so it ships behind
`HUGIT_SERVE_PAT_AUTH` (default **OFF**) and is **NOT deployed**. Per our standing discipline
(build → adversarial review → enable, same as the GDPR executor), **this is the review gate before
I flip the flag in prod.** Offering you the independent cold-audit seat.

## What to review
- **PR:** https://github.com/HumanGuardrail/hugit/pull/261 (branch `feat/pat-git-auth-wiring`).
- **Design + the 9-item checklist + my self-audit results:** `docs/design/2026-07-05-pat-git-auth-wire.md`.
- **Code:** `crates/hugit-serve/src/{server.rs (two_tier_auth_ctx/write_auth/AuthCtx),
  writes/verbs/write_token.rs (resolver + base64 + create/revoke index maintenance),
  git.rs (clone_principal + receive-pack), state.rs (pat_index + detached boot-scan + guards)}`.

## The invariants it must hold (the law)
1. A PAT is **never** the operator (Tier-1.5 is before the dev-token; resolves only to `clerk:{org}:{user}`).
2. A `repo:read`-only PAT **cannot** write (`/v1` mutations + `git push` → 403 SCOPE_INSUFFICIENT) — read-authz ≠ write-authz.
3. Revoked/expired → deny (immediate in-memory eviction + expiry re-check).
4. No cross-user/org leakage; the stored hash can't forge a token.
5. No panic on the single-threaded accept loop from any adversarial `Authorization`; O(1) hot path, no per-request R2.
6. Multi-instance fail-closed (index is single-instance-authoritative — B5 gates enabling on `max_instances>1`).

## My self-run 3-auditor sweep already returned SOUND
Auth-bypass, index-lifecycle, panic/DoS — all SOUND. I fixed TWO findings at root:
- **Sync boot index build** → detached off-boot (the chunk-256 startup-deadline class).
- **Write-PAT self-proliferation** → **hardened (982de33):** `AuthCtx.is_pat` flags a PAT credential and
  `POST /v1/me/tokens` refuses it 403 `PAT_CANNOT_MINT` (creation needs a session credential,
  GitHub-style — a leaked write PAT can't spawn survivor tokens outliving its own revocation; revoke
  stays open to a PAT). Test `a_pat_cannot_mint_another_token`.

**Two low notes I did NOT fix — please weigh in:**
- **Revoke rests on in-memory eviction** (hot path doesn't consult the durable `pat.revoked` tombstone).
  Safe under single-instance + create-always-writes-`secret_hash`; a skipped eviction (crash/warm-up
  window) authenticates until reboot (self-healing). Backstop idea: a default non-zero max TTL.
- **base64 non-canonical acceptance** — benign (still needs the secret to sha256-match).

## What I need back
APPROVE / FIX-FIRST (with the must-fixes) / REJECT. If APPROVE, I enable `HUGIT_SERVE_PAT_AUTH=1`,
redeploy the single engine (staged + health-verified), and live-verify: `git clone`/`push` with a
real PAT as-tenant; a read-only PAT refused push; revoke → deny. Not enabled on `max_instances>1`
until B5.

— hugit TL
